use super::*;

fn assert_lookup_matches_rebuild(index: &Index) {
    let mut rebuilt = index.clone();
    rebuilt.rebuild_lookup();
    assert_eq!(index.by_path, rebuilt.by_path);
    assert_eq!(index.by_alias, rebuilt.by_alias);
    assert_eq!(index.backlinks, rebuilt.backlinks);
    let sorted = |map: &HashMap<String, Vec<PathBuf>>| {
        let mut map = map.clone();
        for paths in map.values_mut() {
            paths.sort();
        }
        map
    };
    assert_eq!(sorted(&index.by_stem), sorted(&rebuilt.by_stem));
}

#[test]
fn limited_search_is_a_prefix_for_every_sort_and_query_shape() {
    use crate::file_order::SortBy;
    let mut index = Index::default();
    for i in 0..75 {
        let path = PathBuf::from(format!("资料/Note{i}.md"));
        index.update(
            path.clone(),
            format!("---\nstatus: done\n---\n# 标题\n目标 {i}\n目标 again\n"),
        );
        let note = Arc::make_mut(index.notes.get_mut(&path).unwrap());
        note.times.modified =
            (i % 4 != 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(i % 9));
        note.times.created =
            (i % 3 != 0).then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs(i % 7));
    }
    for by in [SortBy::Name, SortBy::Modified, SortBy::Created] {
        for descending in [false, true] {
            for query in [
                "目标",
                "file:Note",
                "-[missing]",
                "[status:done]",
                "line:(目标 again)",
                "目标 -content:74",
                "不存在",
            ] {
                let all = index
                    .search_limited(query, false, by, descending, usize::MAX)
                    .unwrap();
                for limit in [0, 1, 17, 74, 100, 200] {
                    let limited = index
                        .search_limited(query, false, by, descending, limit)
                        .unwrap();
                    let signature = |hit: &SearchHit| {
                        (
                            hit.path.clone(),
                            hit.offset,
                            hit.line,
                            hit.excerpt.clone(),
                            hit.highlights.clone(),
                        )
                    };
                    assert_eq!(
                        limited.iter().map(signature).collect::<Vec<_>>(),
                        all.iter().take(limit).map(signature).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn incremental_links_match_rebuild_and_body_edits_share_lookup_maps() {
    let mut index = Index::default();
    for (path, text) in [
        ("a.md", "---\naliases: [别名]\n---\n# A"),
        ("folder/b.md", "# B"),
        (
            "entry.md",
            "[[a]] [[a#heading]] [B](folder/b.md) [[missing]]",
        ),
        ("other.md", "[[别名]] [[b]]"),
    ] {
        index.update(path.into(), text.into());
    }
    let original = index.clone();
    index.update(
        "entry.md".into(),
        format!("正文\n{}", original.notes[Path::new("entry.md")].text),
    );
    assert!(Arc::ptr_eq(&original.by_path, &index.by_path));
    assert!(Arc::ptr_eq(&original.by_stem, &index.by_stem));
    assert!(Arc::ptr_eq(&original.by_alias, &index.by_alias));
    assert!(Arc::ptr_eq(&original.backlinks, &index.backlinks));
    for (path, text) in [
        ("entry.md", "[[b]] [[b]] [self](#heading) [[entry]]"),
        ("entry.md", "[A][ref]\n\n[ref]: a.md\n"),
        ("a.md", "---\naliases: [新别名]\n---\n# A"),
        ("other/b.md", "# ambiguous"),
        ("missing.md", "# newly resolved"),
        ("entry.md", "no links"),
    ] {
        index.update(path.into(), text.into());
        assert_lookup_matches_rebuild(&index);
    }
    assert_eq!(
        original.backlinks(Path::new("a.md")),
        vec![PathBuf::from("entry.md"), PathBuf::from("other.md")]
    );
    assert_lookup_matches_rebuild(&original);
    let unchanged = index.clone();
    index.update("entry.md".into(), "no links".into());
    assert!(Arc::ptr_eq(
        &unchanged.notes[Path::new("entry.md")],
        &index.notes[Path::new("entry.md")]
    ));
}

#[test]
fn batched_name_link_and_error_changes_match_fresh_index() {
    let root = std::env::temp_dir().join(format!("inkstone-index-batch-{}", std::process::id()));
    std::fs::create_dir_all(root.join("vault")).unwrap();
    let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
    for (path, text) in [("a.md", ""), ("b.md", "[[a]]"), ("c.md", "[[别名]]")] {
        std::fs::write(vault.root.join(path), text).unwrap();
    }
    let mut index = Index::build(&vault).unwrap();
    let updates = [
        (
            PathBuf::from("a.md"),
            "---\naliases: [别名]\n---\n".to_string(),
        ),
        ("b.md".into(), "[[c]]".into()),
    ];
    for (path, text) in &updates {
        std::fs::write(vault.root.join(path), text).unwrap();
    }
    assert!(index.apply_known(&vault, updates));
    assert_lookup_matches_rebuild(&index);
    std::fs::remove_file(vault.root.join("a.md")).unwrap();
    std::fs::write(vault.root.join("b.md"), "[[别名]]").unwrap();
    assert!(
        index
            .refresh_paths(&vault, ["a.md".into(), "b.md".into()])
            .unwrap()
    );
    assert_lookup_matches_rebuild(&index);
    assert_eq!(index.backlinks, Index::build(&vault).unwrap().backlinks);
    std::fs::write(vault.root.join("invalid.md"), [0xff]).unwrap();
    assert!(index.refresh_paths(&vault, ["invalid.md".into()]).unwrap());
    assert!(!index.refresh_paths(&vault, ["invalid.md".into()]).unwrap());
    std::fs::remove_file(vault.root.join("invalid.md")).unwrap();
    assert!(index.refresh_paths(&vault, ["invalid.md".into()]).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn relocating_a_folder_reuses_note_arcs_and_drops_trashed_paths() {
    let mut index = Index::default();
    index.update("keep.md".into(), "keep".into());
    index.update("old.md".into(), "sibling".into());
    index.update("old/note.md".into(), "moved".into());
    index.files = vec![
        "keep.md".into(),
        "old/image.png".into(),
        "old/note.md".into(),
    ];
    index
        .errors
        .insert("old/bad.md".into(), "unreadable".into());
    let moved = index.relocate(Path::new("old"), Some(Path::new("archive/new")), true);
    assert!(std::sync::Arc::ptr_eq(
        &index.notes[Path::new("keep.md")],
        &moved.notes[Path::new("keep.md")]
    ));
    assert!(std::sync::Arc::ptr_eq(
        &index.notes[Path::new("old.md")],
        &moved.notes[Path::new("old.md")]
    ));
    assert!(std::sync::Arc::ptr_eq(
        &index.notes[Path::new("old/note.md")],
        &moved.notes[Path::new("archive/new/note.md")]
    ));
    assert_eq!(
        moved.files,
        vec![
            PathBuf::from("archive/new/image.png"),
            PathBuf::from("archive/new/note.md"),
            PathBuf::from("keep.md"),
        ]
    );
    assert_eq!(moved.errors[Path::new("archive/new/bad.md")], "unreadable");
    let trashed = index.relocate(Path::new("old"), None, true);
    assert!(std::sync::Arc::ptr_eq(
        &index.notes[Path::new("keep.md")],
        &trashed.notes[Path::new("keep.md")]
    ));
    assert!(!trashed.notes.contains_key(Path::new("old/note.md")));
    assert_eq!(trashed.files, vec![PathBuf::from("keep.md")]);
    assert!(trashed.notes.contains_key(Path::new("old.md")));
    assert!(trashed.errors.is_empty());
}

#[test]
fn index_snapshots_share_unchanged_notes_and_isolate_updates() {
    let mut old = Index::default();
    old.update("first.md".into(), "before".into());
    old.update("second.md".into(), "unchanged".into());
    let mut next = old.clone();
    assert!(std::sync::Arc::ptr_eq(
        &old.notes[Path::new("first.md")],
        &next.notes[Path::new("first.md")]
    ));
    next.update("first.md".into(), "after".into());
    assert_eq!(old.notes[Path::new("first.md")].text, "before");
    assert_eq!(next.notes[Path::new("first.md")].text, "after");
    assert!(std::sync::Arc::ptr_eq(
        &old.notes[Path::new("second.md")],
        &next.notes[Path::new("second.md")]
    ));
}

#[test]
fn unreadable_notes_are_isolated_and_rejoin_after_repair() {
    let root = std::env::temp_dir().join(format!(
        "inkstone-index-errors-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("vault")).unwrap();
    let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
    std::fs::write(vault.root.join("good.md"), "good").unwrap();
    std::fs::write(vault.root.join("bad.md"), [0xff]).unwrap();
    let mut index = Index::build(&vault).unwrap();
    assert_eq!(index.notes.len(), 1);
    assert!(index.errors.contains_key(Path::new("bad.md")));
    assert_eq!(index.note_paths().len(), 2);
    std::fs::write(vault.root.join("good.md"), "updated").unwrap();
    std::fs::write(vault.root.join("bad.md"), "repaired").unwrap();
    index
        .refresh_paths(&vault, ["good.md".into(), "bad.md".into()])
        .unwrap();
    assert!(index.errors.is_empty());
    assert_eq!(index.notes[Path::new("good.md")].text, "updated");
    assert_eq!(index.notes[Path::new("bad.md")].text, "repaired");
    std::fs::write(vault.root.join("bad.md"), [0xff]).unwrap();
    index.refresh_paths(&vault, ["bad.md".into()]).unwrap();
    assert!(!index.notes.contains_key(Path::new("bad.md")));
    std::fs::remove_file(vault.root.join("bad.md")).unwrap();
    index.refresh_paths(&vault, ["bad.md".into()]).unwrap();
    assert!(index.errors.is_empty());
    std::fs::write(vault.root.join("good.md"), "on disk").unwrap();
    assert!(index.apply_known(&vault, [(PathBuf::from("good.md"), "already known".into())]));
    assert_eq!(index.notes[Path::new("good.md")].text, "already known");
    assert!(!index.apply_known(&vault, [(PathBuf::from("good.md"), "already known".into())]));
    std::fs::remove_file(vault.root.join("good.md")).unwrap();
    assert!(index.apply_known(&vault, [(PathBuf::from("good.md"), "already known".into())]));
    assert!(!index.notes.contains_key(Path::new("good.md")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_task_markers_share_index_and_search_semantics() {
    let source = "- [-] cancelled\n> - [!] important\n1. [✓] complete\n- [ ] open\n\n```\n- [!] code\n```\n\n- [link](url)\n";
    let parsed = parse(source);
    assert_eq!(parsed.tasks.len(), 4);
    assert_eq!(
        parsed
            .tasks
            .iter()
            .map(|t| &source[t.marker.clone()])
            .collect::<Vec<_>>(),
        vec!["-", "!", "✓", " "]
    );
    assert!(parsed.tasks[..3].iter().all(|t| t.checked));
    let changed = set_task(source, parsed.tasks[2].marker.clone(), false).unwrap();
    assert!(changed.contains("1. [ ] complete"));
    let mut index = Index::default();
    index.update("a.md".into(), source.into());
    assert_eq!(index.search("task-done:important").len(), 1);
    assert!(index.search("task-todo:important").is_empty());
    assert!(index.search("task:code").is_empty());
}
#[test]
fn property_search_locates_quoted_keys_and_multiline_values() {
    let mut index = Index::default();
    let source = "---\nother: keep\n\"名称: 中文\": done\naliases:\n  - one\n  - two\n---\nbody";
    index.update("note.md".into(), source.into());
    let hits = index.search("[\"名称: 中文\":done]");
    assert_eq!(hits[0].line, 3);
    assert_eq!(hits[0].offset, source.find("\"名称").unwrap());
    assert!(hits[0].excerpt.contains("done"));
    assert!(!hits[0].highlights.is_empty());
    let list = index.search("[aliases:two]");
    assert_eq!(
        list.iter().map(|hit| hit.line).collect::<Vec<_>>(),
        [4, 5, 6]
    );
    assert!(index.search("-[missing]")[0].highlights.is_empty());
}
#[test]
fn property_conditions_use_original_metadata_inside_line_and_section_scopes() {
    let mut index = Index::default();
    let source = "---\nstatus: done\n---\n# Title\nalpha\nbeta\n";
    index.update("note.md".into(), source.into());
    assert_eq!(index.search("[status:done]").len(), 1);
    let line = index.search("line:([status:done] alpha)");
    assert_eq!(line.len(), 1);
    assert_eq!(line[0].line, 5);
    assert_eq!(&line[0].excerpt[line[0].highlights[0].clone()], "alpha");
    assert_eq!(index.search("section:([status:done] Title)")[0].line, 4);
    index.update("note.md".into(), source.replace("done", "draft"));
    assert!(index.search("[status:done]").is_empty());
    assert_eq!(index.search("[status:draft]").len(), 1);
}
#[test]
fn section_search_respects_heading_boundaries_and_nested_descendants() {
    let mut index = Index::default();
    let source =
        "intro\n# Parent\nalpha\n\nbeta\n## Child\nchildword\n### Deep\ndeepword\n# Other\ngamma\n";
    index.update("note.md".into(), source.into());
    assert_eq!(
        index
            .search("section:(alpha beta)")
            .iter()
            .map(|hit| hit.line)
            .collect::<Vec<_>>(),
        [3, 5]
    );
    assert!(index.search("section:(alpha childword)").is_empty());
    let nested = index.search("section:(Parent section:(match-case:Child section:deepword))");
    assert_eq!(
        nested.iter().map(|hit| hit.line).collect::<Vec<_>>(),
        [2, 6, 9]
    );
    assert_eq!(nested[2].offset, source.find("deepword").unwrap());
    assert_eq!(
        &nested[2].excerpt[nested[2].highlights[0].clone()],
        "deepword"
    );
    assert!(index.search("section:(Other section:childword)").is_empty());
    assert!(index.search("section:(intro section:childword)").is_empty());
}
#[test]
fn block_search_keeps_paragraphs_and_list_items_separate() {
    let mut index = Index::default();
    let source = "alpha\nbeta\n\nalpha\n\nbeta\n\n- item alpha\n  continuation beta\n- item only alpha\n- beta alone\n\n```\nalpha beta\n```\n";
    index.update("note.md".into(), source.into());
    let hits = index.search("block:(alpha beta)");
    assert_eq!(
        hits.iter().map(|hit| hit.line).collect::<Vec<_>>(),
        [1, 2, 8, 9, 14]
    );
    assert_eq!(&hits[3].excerpt[hits[3].highlights[0].clone()], "beta");
    index.update("note.md".into(), "- alpha\n- beta\n".into());
    assert!(index.search("block:(alpha beta)").is_empty());
    index.update("note.md".into(), "> alpha\n>\n> beta\n".into());
    assert_eq!(index.search("block:(alpha beta)").len(), 2);
}
#[test]
fn task_search_filters_state_and_includes_continuations_without_code() {
    let mut index = Index::default();
    let source = "---\ntext: |\n  - [ ] YAMLfake\n---\n- [ ] alpha\n  continuation beta\n\n- [x] alpha beta\n- [?] custom done\n- plain [x] notTask\n\n```md\n- [ ] codefake\n```\n";
    index.update("FilenameOnly.md".into(), source.into());
    let todo = index.search("task-todo:(alpha beta)");
    assert_eq!(todo.iter().map(|hit| hit.line).collect::<Vec<_>>(), [5, 6]);
    assert_eq!(&todo[1].excerpt[todo[1].highlights[0].clone()], "beta");
    assert_eq!(index.search("task-done:alpha").len(), 1);
    assert_eq!(index.search("task-done:custom").len(), 1);
    assert!(index.search("task:codefake").is_empty());
    assert!(index.search("task:YAMLfake").is_empty());
    assert!(index.search("task:notTask").is_empty());
    assert!(index.search("task:FilenameOnly").is_empty());
    assert_eq!(index.search("task:\"\"").len(), 3);
    assert_eq!(index.search("task-todo: \"\"").len(), 1);
    assert_eq!(index.search("task-done:\"\"").len(), 2);
}
#[test]
fn line_scope_requires_same_line_and_preserves_global_offsets() {
    let mut index = Index::default();
    let source = "alpha\r\nbeta\r\n😀alpha beta\r\nalpha skip\r\n";
    index.update("FilenameOnly.md".into(), source.into());
    let hits = index.search("line:(alpha beta)");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, 3);
    assert_eq!(hits[0].offset, source.find("😀alpha").unwrap() + "😀".len());
    assert_eq!(
        hits[0]
            .highlights
            .iter()
            .map(|range| &hits[0].excerpt[range.clone()])
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(
        index
            .search("line:(alpha -skip)")
            .iter()
            .map(|hit| hit.line)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        index
            .search("line:-skip")
            .iter()
            .map(|hit| hit.line)
            .collect::<Vec<_>>(),
        [1, 2, 3, 5]
    );
    assert!(index.search("line:FilenameOnly").is_empty());
    assert!(index.search("-line:(alpha beta)").is_empty());
}
#[test]
fn filename_and_path_highlights_follow_successful_query_scopes() {
    let mut index = Index::default();
    let path = PathBuf::from("资料/İdea.md");
    index.update(path.clone(), "body 资料".into());
    let title = path.to_string_lossy().replace('\\', "/");
    let basic = index.search("i");
    assert_eq!(&title[basic[0].title_highlights[0].clone()], "İ");
    let scoped = index.search("(file:İdea OR path:资料) content:body");
    assert_eq!(
        scoped[0]
            .title_highlights
            .iter()
            .map(|range| &title[range.clone()])
            .collect::<Vec<_>>(),
        ["资料", "İdea"]
    );
    assert!(index.search("content:资料")[0].title_highlights.is_empty());
    let excluded = index.search("(file:missing OR path:资料) -content:blocked");
    assert_eq!(excluded[0].title_highlights.len(), 1);
    assert_eq!(&title[excluded[0].title_highlights[0].clone()], "资料");
}
#[test]
fn default_search_includes_filename_while_content_scope_excludes_it() {
    let mut index = Index::default();
    index.update("Meeting.md".into(), "正文\nproject target\n尾行".into());
    assert_eq!(index.search("meeting").len(), 1);
    assert_eq!(index.search("meeting")[0].offset, 0);
    assert!(index.search("content:meeting").is_empty());
    let combined = index.search("meeting project");
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].line, 2);
    assert_eq!(
        &combined[0].excerpt[combined[0].highlights[0].clone()],
        "project"
    );
    assert!(index.search("project -meeting").is_empty());
    assert!(index.search_with_case("meeting", true).unwrap().is_empty());
    assert_eq!(index.search("/Meet.*\\.md/").len(), 1);
}
#[test]
fn grouped_queries_highlight_only_successful_positive_branches() {
    let mut index = Index::default();
    index.update("note.md".into(), "alpha\nblocked\ngamma\ntarget".into());
    let hits = index.search("(alpha -blocked OR gamma) target");
    assert_eq!(hits.iter().map(|hit| hit.line).collect::<Vec<_>>(), [3, 4]);
    assert_eq!(&hits[0].excerpt[hits[0].highlights[0].clone()], "gamma");
    assert_eq!(index.search("-(missing OR absent)").len(), 1);
    assert!(index.search("-(alpha OR gamma)").is_empty());
}
#[test]
fn search_highlights_match_excerpt_bytes_and_merge_overlapping_terms() {
    let mut index = Index::default();
    index.update("note.md".into(), "前😀 Alpha alpha 汉字\nblocked".into());
    let hits = index.search("alpha");
    assert_eq!(
        hits[0]
            .highlights
            .iter()
            .map(|range| &hits[0].excerpt[range.clone()])
            .collect::<Vec<_>>(),
        ["Alpha", "alpha"]
    );
    let overlaps = index.search("alpha OR pha");
    assert_eq!(overlaps[0].highlights.len(), 2);
    let excluded = index.search("alpha -blocked OR 汉字");
    assert_eq!(
        &excluded[0].excerpt[excluded[0].highlights[0].clone()],
        "汉字"
    );
    index.update(
        "note.md".into(),
        format!("{}İx 目标 目标", "前😀".repeat(180)),
    );
    let cropped = index.search("目标");
    assert_eq!(cropped[0].highlights.len(), 2);
    for range in &cropped[0].highlights {
        assert_eq!(&cropped[0].excerpt[range.clone()], "目标");
    }
    let expanded_case = index.search("i");
    assert_eq!(
        &expanded_case[0].excerpt[expanded_case[0].highlights[0].clone()],
        "İ"
    );
}
#[test]
fn search_sorting_happens_before_the_result_limit() {
    use crate::file_order::SortBy;
    let mut index = Index::default();
    for i in 0..210 {
        index.update(format!("note{i}.md").into(), "match".into());
    }
    std::sync::Arc::make_mut(index.notes.get_mut(Path::new("note209.md")).unwrap())
        .times
        .modified = Some(std::time::SystemTime::UNIX_EPOCH);
    let newest = index
        .search_ordered("match", false, SortBy::Modified, true)
        .unwrap();
    assert_eq!(newest.len(), 200);
    assert_eq!(newest[0].path, Path::new("note209.md"));
    let natural = index
        .search_ordered("match", false, SortBy::Name, false)
        .unwrap();
    assert_eq!(natural[2].path, Path::new("note2.md"));
    let reverse = index
        .search_ordered("match", false, SortBy::Name, true)
        .unwrap();
    assert_eq!(reverse[0].path, Path::new("note209.md"));
}
#[test]
fn regex_results_use_multiline_anchors_and_ignore_empty_matches() {
    let mut index = Index::default();
    index.update("note.md".into(), "alpha\nbeta\nalpha\nbeta\n".into());
    assert_eq!(index.search("/^alpha/").len(), 2);
    assert_eq!(
        index
            .search("/(?m)^alpha/")
            .iter()
            .map(|h| h.line)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        index
            .search("/alpha\\nbeta/")
            .iter()
            .map(|h| h.line)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert!(index.search("/$/").is_empty());
    assert!(index.search("//").is_empty());
    index.update("note.md".into(), "ab a\ncb".into());
    assert_eq!(index.search("/(?s)a.*?b|c/").len(), 1);
}
#[test]
fn cross_line_queries_show_all_terms_and_skip_false_or_branches() {
    let mut index = Index::default();
    let source = "前言\n😀alpha\nblocked\n中文beta\ngamma\n";
    index.update("note.md".into(), source.into());
    let both = index.search("alpha beta");
    assert_eq!(both.iter().map(|hit| hit.line).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(both[0].offset, source.find("alpha").unwrap());
    assert_eq!(both[1].offset, source.find("beta").unwrap());
    let alternatives = index.search("alpha -blocked OR gamma");
    assert_eq!(alternatives.len(), 1);
    assert_eq!(alternatives[0].line, 5);
    assert_eq!(index.search("file:note -absent").len(), 1);
    assert!(index.search("alpha -blocked").is_empty());
    index.update("many.md".into(), "alpha\n".repeat(300));
    assert_eq!(index.search("alpha").len(), 200);
}
#[test]
fn search_excerpts_include_late_matches_and_metadata_hits_are_not_repeated() {
    let mut index = Index::default();
    let text = format!("title\n{}目标\n其他条件\n", "前😀".repeat(160));
    index.update("note.md".into(), text.clone());
    let hits = index.search("目标");
    assert_eq!(hits[0].offset, text.find("目标").unwrap());
    assert!(hits[0].excerpt.contains("目标"));
    let spanning = index.search("目标 其他条件");
    assert_eq!(spanning[0].offset, text.find("目标").unwrap());
    assert_eq!(spanning[0].line, 2);
    assert!(spanning[0].excerpt.contains("目标"));
    assert_eq!(index.search("file:note").len(), 1);
}
#[test]
fn tag_index_keeps_occurrences_separate_from_search_tags() {
    let parsed =
        parse("---\ntags: [work, work]\nAliases: ['Case Alias']\n---\n#work #work `#work` \\#work");
    assert_eq!(parsed.tags, ["work"]);
    assert_eq!(parsed.tag_counts["work"], 4);
    assert_eq!(parsed.aliases, ["Case Alias"]);
}
#[test]
fn yaml_aliases_with_punctuation_participate_in_search_and_backlinks() {
    let mut index = Index::default();
    index.update(
        "target.md".into(),
        "---\naliases: [\"Smith, John\", 'ÉTUDE']\ntags: ['工作/项目']\n---\n正文".into(),
    );
    index.update("entry.md".into(), "[[Smith, John]] [[étude]]".into());
    assert_eq!(
        index.filenames("Smith, John")[0].path,
        Path::new("target.md")
    );
    assert_eq!(
        index.resolve(Path::new("entry.md"), "étude"),
        Resolution::Found("target.md".into())
    );
    assert_eq!(
        index.backlinks(Path::new("target.md")),
        vec![PathBuf::from("entry.md")]
    );
    assert_eq!(
        index.search("tag:工作/项目")[0].path,
        Path::new("target.md")
    );
}
#[test]
fn filenames_rank_names_and_aliases_and_keep_real_paths() {
    let mut index = Index::default();
    for (path, aliases) in [
        ("a-project-notes.md", "[]"),
        ("b.md", "[project, project plan]"),
        ("c.md", "[project planning]"),
        ("project draft.md", "[]"),
        ("z/project.md", "[]"),
    ] {
        index.update(path.into(), format!("---\naliases: {aliases}\n---\n"));
    }
    let hits = index.filenames("  PROJECT  ");
    assert_eq!(
        hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(),
        [
            "z/project.md",
            "b.md",
            "project draft.md",
            "c.md",
            "a-project-notes.md"
        ]
        .map(PathBuf::from)
    );
    assert_eq!(hits[1].display_name.as_deref(), Some("project"));
    assert_eq!(hits[3].display_name.as_deref(), Some("project planning"));
    assert!(hits[0].display_name.is_none());
    assert!(
        index
            .search("project")
            .iter()
            .all(|h| h.display_name.is_none())
    );
}
#[test]
fn filenames_limit_after_ranking_and_normalize_queries() {
    let mut index = Index::default();
    for i in 0..250 {
        index.update(format!("a-{i}-目标.md").into(), String::new());
    }
    index.update("资料/目标.md".into(), "---\naliases: [ÉTUDE]\n---\n".into());
    let hits = index.filenames("目标");
    assert_eq!(hits.len(), 200);
    assert_eq!(hits[0].path, Path::new("资料/目标.md"));
    assert_eq!(index.filenames("资料\\目")[0].path, hits[0].path);
    let aliases = index.filenames("étude");
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].display_name.as_deref(), Some("ÉTUDE"));
    assert!(index.filenames("不存在").is_empty());
}
#[test]
fn refresh_updates_file_dates_even_when_text_is_unchanged() {
    let root = std::env::temp_dir().join(format!("inkstone-index-dates-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("note.md"), "same text").unwrap();
    let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
    let mut index = Index::build(&vault).unwrap();
    let newer = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(root.join("note.md"))
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(newer))
        .unwrap();
    index
        .refresh_paths(&vault, [PathBuf::from("note.md")])
        .unwrap();
    assert_eq!(index.notes[Path::new("note.md")].text, "same text");
    assert_eq!(
        index.notes[Path::new("note.md")].times.modified,
        Some(newer)
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn fold_ranges_follow_heading_levels_and_nested_blocks() {
    let source = "# A\nbody\n## B\n- parent\n  - child\n# C\n```md\n# literal\n```";
    let parsed = parse(source);
    assert_eq!(parsed.folds, vec![0..5, 2..5, 3..5, 5..9, 6..9]);
    assert_eq!(
        parsed.fold_ranges(true, false),
        vec![0..5, 2..5, 5..9, 6..9]
    );
    assert_eq!(parsed.fold_ranges(false, true), vec![3..5, 6..9]);
    assert!(parsed.fold_ranges(false, false).is_empty());
    assert!(!parsed.folds.iter().any(|r| r.start == 7));
    assert!(parse("# single").folds.is_empty());
    assert_eq!(parse("Title\n=====\nbody").folds, vec![0..3]);
}
#[test]
fn folder_move_updates_incoming_outgoing_and_embedded_assets_once() {
    let root =
        std::env::temp_dir().join(format!("inkstone-folder-references-{}", std::process::id()));
    std::fs::create_dir_all(root.join("原目录/子目录")).unwrap();
    std::fs::write(root.join("原目录/子目录/图 示.svg"), "<svg/>").unwrap();
    let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
    let mut index = Index::default();
    let incoming = "[[原目录/子目录/笔记#标题|显示]]\n![[原目录/子目录/图 示.svg|160]]\n[笔记](原目录/子目录/笔记.md#%E6%A0%87)\n![图](原目录/子目录/图%20示.svg)\n`[[原目录/子目录/笔记]]`\n[外部](https://example.com/a)";
    let outgoing = "[[../同级]] [[外部]] [[#局部]]\n[根](../../外部.md) ![图](图%20示.svg)\n[局部](#标题)\n[引用][r]\n\n[r]: ../../外部.md \"标题\"";
    index.update("入口.md".into(), incoming.into());
    index.update("原目录/子目录/笔记.md".into(), outgoing.into());
    index.update("原目录/同级.md".into(), "# 同级".into());
    index.update("外部.md".into(), "# 外部".into());
    let edits = index.relocation_edits(
        Path::new("原目录"),
        Path::new("归档/新目录"),
        true,
        Some(&vault.root),
    );
    assert_eq!(edits.len(), 2);
    let entry = &edits
        .iter()
        .find(|e| e.0 == Path::new("入口.md"))
        .unwrap()
        .2;
    let entry = percent_encoding::percent_decode_str(entry)
        .decode_utf8()
        .unwrap();
    assert!(entry.contains("[[/归档/新目录/子目录/笔记#标题|显示]]"));
    assert!(entry.contains("![[/归档/新目录/子目录/图 示.svg|160]]"));
    assert!(entry.contains("![图](归档/新目录/子目录/图 示.svg)"));
    assert!(entry.contains("`[[原目录/子目录/笔记]]`"));
    assert!(entry.contains("https://example.com/a"));
    let moved = &edits
        .iter()
        .find(|e| e.0 == Path::new("归档/新目录/子目录/笔记.md"))
        .unwrap()
        .2;
    let moved = percent_encoding::percent_decode_str(moved)
        .decode_utf8()
        .unwrap();
    assert!(moved.contains("[[/归档/新目录/同级]] [[/外部]] [[#局部]]"));
    assert!(moved.contains("[根](../../../外部.md) ![图](图 示.svg)"));
    assert!(moved.contains("[局部](#标题)"));
    assert!(moved.contains("[r]: ../../../外部.md \"标题\""));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn tasks_and_block_anchors_are_source_based_and_exclude_code() {
    let source = "# 章节 A\r\n\r\n段落😀 ^para-1\r\n\r\n- [ ] 任务一 ^todo-1\r\n- [X] 任务二\r\n\r\n```md\n- [ ] 假任务 ^fake\n```\n\n^code-1\n\n# 下一节\n";
    let parsed = parse(source);
    assert_eq!(parsed.tasks.len(), 2);
    assert_eq!(&source[parsed.tasks[0].marker.clone()], " ");
    assert!(parsed.tasks[1].checked);
    assert_eq!(
        &source[anchor_range(source, &parsed, "^para-1").unwrap()],
        "段落😀 ^para-1"
    );
    assert!(source[anchor_range(source, &parsed, "^todo-1").unwrap()].starts_with("- [ ]"));
    assert!(source[anchor_range(source, &parsed, "^code-1").unwrap()].starts_with("```"));
    assert!(anchor_range(source, &parsed, "^fake").is_none());
    let section = anchor_range(source, &parsed, "章节-A").unwrap();
    assert!(!source[section].contains("下一节"));
    let changed = set_task(source, parsed.tasks[0].marker.clone(), true).unwrap();
    assert!(changed.contains("- [x] 任务一"));
    assert!(changed.contains("\r\n"));
    assert!(set_task(source, 0..1, true).is_none());
}
#[test]
fn reference_style_links_participate_in_backlinks() {
    let mut index = Index::default();
    index.update("来源.md".into(), "[目标][ref]\n\n[ref]: 目标.md\n".into());
    index.update("目标.md".into(), "".into());
    assert_eq!(
        index.backlinks(Path::new("目标.md")),
        vec![PathBuf::from("来源.md")]
    );
}
#[test]
fn tags_aliases_and_standard_backlinks_ignore_code() {
    let source = "---\naliases: [别名, 'Second']\ntags:\n  - 工作/项目\n---\n# 标题\n\n#中文 #123 #work/sub `#code` \\#escaped\n\n[目标](目标.md#章节)\n\n```md\n#not-a-tag [[目标]]\n```\n";
    let note = parse(source);
    assert_eq!(note.aliases, vec!["别名", "Second"]);
    assert!(note.tags.contains(&"中文".into()));
    assert!(note.tags.contains(&"工作/项目".into()));
    assert!(
        !note
            .tags
            .iter()
            .any(|t| matches!(t.as_str(), "123" | "code" | "escaped" | "not-a-tag"))
    );
    let mut index = Index::default();
    index.update("来源.md".into(), source.into());
    index.update("目标.md".into(), "".into());
    assert_eq!(
        index.backlinks(Path::new("目标.md")),
        vec![PathBuf::from("来源.md")]
    );
    assert_eq!(
        index.resolve(Path::new("目标.md"), "别名"),
        Resolution::Found("来源.md".into())
    );
}
#[test]
fn rename_preserves_aliases_fragments_code_and_relative_destinations() {
    let mut index = Index::default();
    index.update(
        "原目录/笔记.md".into(),
        "[[../目标]] [目标](../目标.md)\n".into(),
    );
    index.update("目标.md".into(), "# 目标".into());
    index.update(
        "来源.md".into(),
        "[[原目录/笔记#章节|显示]]\n[标题](原目录/笔记.md#章节)\n`[[原目录/笔记]]`\n".into(),
    );
    let edits = index.relocation_edits(
        Path::new("原目录/笔记.md"),
        Path::new("新 目录/改名.md"),
        false,
        None,
    );
    let source = &edits
        .iter()
        .find(|(p, _, _)| p == Path::new("来源.md"))
        .unwrap()
        .2;
    assert!(source.contains("[[/新 目录/改名#章节|显示]]"));
    assert!(source.contains("[标题](%E6%96%B0%20%E7%9B%AE%E5%BD%95/%E6%94%B9%E5%90%8D.md#章节)"));
    assert!(source.contains("`[[原目录/笔记]]`"));
    let moved = &edits
        .iter()
        .find(|(p, _, _)| p == Path::new("新 目录/改名.md"))
        .unwrap()
        .2;
    assert!(moved.contains("[[/目标]]"));
}
#[test]
fn dotted_wiki_names_keep_their_entire_stem() {
    let mut index = Index::default();
    index.update("会议.2026.md".into(), String::new());
    assert_eq!(
        index.resolve(Path::new("来源.md"), "会议.2026"),
        Resolution::Found("会议.2026.md".into())
    );
    assert_eq!(
        index.resolve(Path::new("来源.md"), "项目/会议.2027"),
        Resolution::Missing("项目/会议.2027.md".into())
    );
}
#[test]
fn markdown_paths_are_relative_and_decode_path_separately_from_anchor() {
    let mut index = Index::default();
    index.update("目录/子目录/中文 空格#笔记.md".into(), "# 标题".into());
    index.update("子目录/中文 空格#笔记.md".into(), String::new());
    assert_eq!(index.resolve_markdown(Path::new("目录/来源.md"),"子目录/%E4%B8%AD%E6%96%87%20%E7%A9%BA%E6%A0%BC%23%E7%AC%94%E8%AE%B0.md#%E6%A0%87%E9%A2%98"),
        (Resolution::Found("目录/子目录/中文 空格#笔记.md".into()),Some("标题".into())));
    assert_eq!(
        index
            .resolve_markdown(Path::new("目录/来源.md"), "missing.md")
            .0,
        Resolution::Missing("目录/missing.md".into())
    );
    assert_eq!(
        index.resolve_markdown(Path::new("来源.md"), "image.png").0,
        Resolution::Invalid
    );
    assert_eq!(
        index
            .resolve_markdown(Path::new("来源.md"), "../outside.md")
            .0,
        Resolution::Invalid
    );
}
#[test]
fn ast_links_headings_exclude_code_and_preserve_unicode_ranges() {
    let source = "# 中文😀\n**[[笔记|别名]]** `[[不是]]`\n\n```\n[[也不是]]\n```\n";
    let parsed = parse(source);
    assert_eq!(parsed.headings[0].title, "中文😀");
    assert_eq!(parsed.links.len(), 1);
    assert_eq!(&source[parsed.links[0].range.clone()], "[[笔记|别名]]");
    assert!(
        crate::rendering::reading_document(&Index::default(), Path::new("a.md"), source)
            .markdown
            .contains("[别名](inkstone-reference:0)")
    );
}
#[test]
fn resolution_ambiguity_relative_paths_and_backlink_rebuild() {
    let mut index = Index::default();
    index.update("a/同名.md".into(), "A".into());
    index.update("b/同名.md".into(), "B".into());
    index.update("a/来源.md".into(), "[[同名]] [[b/同名#标题]]".into());
    assert_eq!(
        index.resolve(Path::new("a/来源.md"), "同名"),
        Resolution::Found("a/同名.md".into())
    );
    assert!(matches!(
        index.resolve(Path::new("来源.md"), "同名"),
        Resolution::Ambiguous(_)
    ));
    assert_eq!(
        index.backlinks(Path::new("b/同名.md")),
        vec![PathBuf::from("a/来源.md")]
    );
    index.update("a/来源.md".into(), "无链接".into());
    assert!(index.backlinks(Path::new("b/同名.md")).is_empty());
    assert_eq!(
        index.resolve(Path::new("a/来源.md"), "../新文档"),
        Resolution::Missing("新文档.md".into())
    );
    assert_eq!(
        index.resolve(Path::new("来源.md"), "../越界"),
        Resolution::Invalid
    );
}
#[test]
fn chinese_search_returns_byte_position() {
    let mut index = Index::default();
    index.update("中文.md".into(), "第一行😀\n第二行目标\n".into());
    let hits = index.search("目标");
    assert_eq!(hits[0].offset, "第一行😀\n第二行".len());
    assert_eq!(hits[0].line, 2);
}
