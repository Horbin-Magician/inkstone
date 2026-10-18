use super::*;
use gpui_component::{
    Disableable,
    button::Button,
    menu::{DropdownMenu, PopupMenuItem},
};
use inkstone_core::preferences::{AnchorBookmark, BookmarkAnchor};

/// Keep only the last displayed document. Identity, revision and path prevent
/// stale candidates after switching views, editing text or renaming a note.
#[derive(Default)]
pub(super) struct CandidateCache(std::cell::RefCell<Option<CachedCandidates>>);

struct CachedCandidates {
    editor: gpui::EntityId,
    revision: u64,
    path: PathBuf,
    entries: Arc<Vec<AnchorBookmark>>,
}

impl CandidateCache {
    fn get(&self, tab: &Tab, cx: &App) -> Arc<Vec<AnchorBookmark>> {
        let editor = tab.save.editor.entity_id();
        let revision = tab.save.editor.read(cx).text_revision();
        let mut cache = self.0.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.editor == editor
            && cached.revision == revision
            && cached.path == tab.path
        {
            return cached.entries.clone();
        }
        let parsed = inkstone_core::index::parse(&tab.save.text_snapshot(cx));
        let mut occurrences = std::collections::HashMap::new();
        let mut entries = Vec::with_capacity(parsed.headings.len() + parsed.blocks.len());
        for heading in &parsed.headings {
            let occurrence = occurrences.entry(&heading.title).or_insert(0);
            entries.push(AnchorBookmark {
                path: tab.path.clone(),
                anchor: BookmarkAnchor::Heading {
                    title: heading.title.clone(),
                    occurrence: *occurrence,
                },
            });
            *occurrence += 1;
        }
        entries.extend(parsed.blocks.iter().map(|block| AnchorBookmark {
            path: tab.path.clone(),
            anchor: BookmarkAnchor::Block {
                id: block.id.clone(),
            },
        }));
        let entries = Arc::new(entries);
        *cache = Some(CachedCandidates {
            editor,
            revision,
            path: tab.path.clone(),
            entries: entries.clone(),
        });
        entries
    }
}

impl Workspace {
    fn toggle_anchor_bookmark(&mut self, entry: AnchorBookmark, cx: &mut Context<Self>) {
        if let Some(i) = self
            .ui
            .prefs
            .anchor_bookmarks
            .iter()
            .position(|b| b == &entry)
        {
            self.ui.prefs.anchor_bookmarks.remove(i);
        } else {
            self.ui.prefs.anchor_bookmarks.push(entry);
        }
        self.persist_workspace(cx);
        cx.notify();
    }

    fn open_anchor_bookmark(
        &mut self,
        entry: &AnchorBookmark,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = if let Some(tab) = self.tabs.iter().find(|t| t.path == entry.path) {
            entry.offset(&inkstone_core::index::parse(
                &tab.save.editor.read(cx).value(),
            ))
        } else {
            self.index
                .notes
                .get(&entry.path)
                .and_then(|note| entry.offset(&note.parsed))
        };
        if let Some(offset) = offset {
            self.pending_jump = Some((entry.path.clone(), offset));
            self.open_note(entry.path.clone(), window, cx);
            self.apply_jump(window, cx);
        } else {
            self.notifications
                .publish(format!("书签目标不存在：{}", entry.label()));
        }
        cx.notify();
    }

    pub(super) fn bookmark_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let candidates = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| self.ui.bookmark_candidates.get(tab, cx))
            .unwrap_or_default();
        let weak = cx.entity().downgrade();
        let saved = self.ui.prefs.anchor_bookmarks.clone();
        div()
            .id("bookmark-list")
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .child(div().p_2().child("书签"))
            .child(
                Button::new("bookmark-anchor")
                    .compact()
                    .label("添加标题 / 块书签")
                    .disabled(candidates.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        for entry in candidates.iter() {
                            let weak = weak.clone();
                            let entry = entry.clone();
                            menu = menu.item(
                                PopupMenuItem::new(entry.label())
                                    .checked(saved.contains(&entry))
                                    .on_click(move |_, _, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.toggle_anchor_bookmark(entry.clone(), cx)
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .when(
                self.ui.prefs.bookmarks.is_empty()
                    && self.ui.prefs.anchor_bookmarks.is_empty()
                    && self.ui.prefs.saved_searches.is_empty(),
                |s| {
                    s.child(div().p_2().child(
                        "可从文件菜单收藏文件，在此添加标题或带 ^块ID 的块，或在搜索面板保存搜索。",
                    ))
                },
            )
            .children(self.ui.prefs.bookmarks.iter().enumerate().map(|(i, path)| {
                let open = path.clone();
                let remove = path.clone();
                div()
                    .id(("bookmark-file", i))
                    .flex()
                    .items_center()
                    .p_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .cursor_pointer()
                            .id(("open-file-bookmark", i))
                            .child(path.display().to_string())
                            .on_click(cx.listener(move |this, _, w, cx| {
                                this.open_note(open.clone(), w, cx)
                            })),
                    )
                    .child(
                        Button::new(("remove-file-bookmark", i))
                            .compact()
                            .label("移除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ui.prefs.bookmarks.retain(|p| p != &remove);
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    )
            }))
            .children(
                self.ui
                    .prefs
                    .anchor_bookmarks
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let open = entry.clone();
                        let remove = entry.clone();
                        div()
                            .id(("bookmark-anchor-row", i))
                            .flex()
                            .items_center()
                            .p_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .cursor_pointer()
                                    .id(("open-anchor-bookmark", i))
                                    .child(entry.label())
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.open_anchor_bookmark(&open, w, cx)
                                    })),
                            )
                            .child(
                                Button::new(("remove-anchor-bookmark", i))
                                    .compact()
                                    .label("移除")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_anchor_bookmark(remove.clone(), cx)
                                    })),
                            )
                    }),
            )
            .children(
                self.ui
                    .prefs
                    .saved_searches
                    .iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let open = entry.clone();
                        let remove = entry.clone();
                        div()
                            .id(("bookmark-search", i))
                            .flex()
                            .items_center()
                            .p_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .cursor_pointer()
                                    .id(("open-search-bookmark", i))
                                    .child(format!("搜索：{}", entry.query))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.apply_saved_search(open.clone(), w, cx)
                                    })),
                            )
                            .child(
                                Button::new(("remove-search-bookmark", i))
                                    .compact()
                                    .label("移除")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.ui.prefs.saved_searches.retain(|s| s != &remove);
                                        this.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            )
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn bookmark_candidates_track_document_revision_identity_and_path(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let text = "# Same\n\n## Other\n\n# Same\n\nText ^block\n";
                w.add_tab("a.md".into(), Some(text.into()), false, window, cx);
                let initial = w.ui.bookmark_candidates.get(&w.tabs[0], cx);
                assert_eq!(initial.len(), 4);
                assert_eq!(
                    initial[2].anchor,
                    BookmarkAnchor::Heading {
                        title: "Same".into(),
                        occurrence: 1
                    }
                );
                assert_eq!(
                    initial[3].anchor,
                    BookmarkAnchor::Block { id: "block".into() }
                );
                // Repaints and another view of this document reuse the parsed candidates.
                assert!(Arc::ptr_eq(
                    &initial,
                    &w.ui.bookmark_candidates.get(&w.tabs[0], cx)
                ));
                w.add_tab("a.md".into(), None, false, window, cx);
                assert!(Arc::ptr_eq(
                    &initial,
                    &w.ui.bookmark_candidates.get(&w.tabs[1], cx)
                ));
                // Same-length text changes must not reuse the old title or block ID.
                w.tabs[0].save.editor.update(cx, |editor, cx| {
                    editor.replace_all(
                        text.replace("Same", "Else").replace("block", "other"),
                        window,
                        cx,
                    );
                });
                let changed = w.ui.bookmark_candidates.get(&w.tabs[1], cx);
                assert_eq!(
                    changed[2].anchor,
                    BookmarkAnchor::Heading {
                        title: "Else".into(),
                        occurrence: 1
                    }
                );
                assert_eq!(
                    changed[3].anchor,
                    BookmarkAnchor::Block { id: "other".into() }
                );
                let document = w.tabs[0].save.clone();
                w.relocate_document(&document, "renamed.md".into());
                let renamed = w.ui.bookmark_candidates.get(&w.tabs[0], cx);
                assert!(
                    renamed
                        .iter()
                        .all(|entry| entry.path == std::path::Path::new("renamed.md"))
                );
                // A different editor may have the same revision; it has its own candidates.
                w.add_tab(
                    "b.md".into(),
                    Some("# Different\n".into()),
                    false,
                    window,
                    cx,
                );
                let other = w.ui.bookmark_candidates.get(&w.tabs[2], cx);
                assert_eq!(other.len(), 1);
                assert_eq!(
                    other[0].anchor,
                    BookmarkAnchor::Heading {
                        title: "Different".into(),
                        occurrence: 0
                    }
                );
                assert_eq!(w.ui.bookmark_candidates.get(&w.tabs[0], cx), renamed);
                w.tabs[0].save.editor.update(cx, |editor, cx| {
                    editor.replace_all("plain text", window, cx)
                });
                assert!(w.ui.bookmark_candidates.get(&w.tabs[0], cx).is_empty());
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn anchor_bookmarks_follow_drafts_relocations_and_missing_targets(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-bookmarks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "# Same\n\n# Same\n\nText ^target\n").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note("a.md".into(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                let heading = AnchorBookmark {
                    path: "a.md".into(),
                    anchor: BookmarkAnchor::Heading {
                        title: "Same".into(),
                        occurrence: 1,
                    },
                };
                let block = AnchorBookmark {
                    path: "a.md".into(),
                    anchor: BookmarkAnchor::Block {
                        id: "target".into(),
                    },
                };
                w.toggle_anchor_bookmark(heading.clone(), cx);
                w.toggle_anchor_bookmark(block.clone(), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let heading = w.ui.prefs.anchor_bookmarks[0].clone();
                let block = w.ui.prefs.anchor_bookmarks[1].clone();
                let prefs = inkstone_core::preferences::Preferences::load(
                    &root.join(".inkstone-workspace.json"),
                );
                assert_eq!(prefs.anchor_bookmarks, vec![heading.clone(), block.clone()]);
                let draft = "Inserted\n\n# Same\n\n# Same\n\nText ^target\n";
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all(draft, window, cx));
                w.open_anchor_bookmark(&heading, window, cx);
                assert_eq!(
                    w.current_pane()
                        .unwrap()
                        .read(cx)
                        .editor
                        .read(cx)
                        .selected_range()
                        .start,
                    draft.rfind("# Same").unwrap()
                );
                w.open_anchor_bookmark(&block, window, cx);
                assert_eq!(
                    w.current_pane()
                        .unwrap()
                        .read(cx)
                        .editor
                        .read(cx)
                        .selected_range()
                        .start,
                    draft.find("Text").unwrap()
                );
                let missing = AnchorBookmark {
                    path: "a.md".into(),
                    anchor: BookmarkAnchor::Block {
                        id: "missing".into(),
                    },
                };
                w.open_anchor_bookmark(&missing, window, cx);
                assert!(w.notifications.text().contains("书签目标不存在"));
                let prepared = super::super::file_sync::PreparedRelocation::new(
                    w.index.clone(),
                    std::path::Path::new("a.md"),
                    Some(std::path::Path::new("folder/a.md")),
                    false,
                );
                w.apply_relocated_index(
                    std::path::Path::new("a.md"),
                    Some(std::path::Path::new("folder/a.md")),
                    false,
                    prepared,
                    cx,
                );
                assert!(
                    w.ui.prefs
                        .anchor_bookmarks
                        .iter()
                        .all(|b| b.path == std::path::Path::new("folder/a.md"))
                );
                let prepared = super::super::file_sync::PreparedRelocation::new(
                    w.index.clone(),
                    std::path::Path::new("folder"),
                    Some(std::path::Path::new("moved")),
                    true,
                );
                w.apply_relocated_index(
                    std::path::Path::new("folder"),
                    Some(std::path::Path::new("moved")),
                    true,
                    prepared,
                    cx,
                );
                assert!(
                    w.ui.prefs
                        .anchor_bookmarks
                        .iter()
                        .all(|b| b.path == std::path::Path::new("moved/a.md"))
                );
                let prepared = super::super::file_sync::PreparedRelocation::new(
                    w.index.clone(),
                    std::path::Path::new("moved"),
                    None,
                    true,
                );
                w.apply_relocated_index(std::path::Path::new("moved"), None, true, prepared, cx);
                assert!(w.ui.prefs.anchor_bookmarks.is_empty());
                w.toggle_anchor_bookmark(heading.clone(), cx);
                w.toggle_anchor_bookmark(heading, cx);
                assert!(w.ui.prefs.anchor_bookmarks.is_empty());
                w.watcher = None;
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }
}
