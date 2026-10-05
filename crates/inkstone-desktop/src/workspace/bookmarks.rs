use super::*;
use gpui_component::{
    Disableable,
    button::Button,
    menu::{DropdownMenu, PopupMenuItem},
};
use inkstone_core::preferences::{AnchorBookmark, BookmarkAnchor};

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
            self.status = format!("书签目标不存在：{}", entry.label());
        }
        cx.notify();
    }

    pub(super) fn bookmark_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut candidates = Vec::new();
        if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
            let parsed = inkstone_core::index::parse(&tab.save.editor.read(cx).value());
            for (i, heading) in parsed.headings.iter().enumerate() {
                candidates.push(AnchorBookmark {
                    path: tab.path.clone(),
                    anchor: BookmarkAnchor::Heading {
                        title: heading.title.clone(),
                        occurrence: parsed.headings[..i]
                            .iter()
                            .filter(|h| h.title == heading.title)
                            .count(),
                    },
                });
            }
            candidates.extend(parsed.blocks.iter().map(|block| AnchorBookmark {
                path: tab.path.clone(),
                anchor: BookmarkAnchor::Block {
                    id: block.id.clone(),
                },
            }));
        }
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
                        for entry in &candidates {
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
                assert!(w.status.contains("书签目标不存在"));
                w.apply_relocated_index(
                    std::path::Path::new("a.md"),
                    Some(std::path::Path::new("folder/a.md")),
                    false,
                    cx,
                );
                assert!(
                    w.ui.prefs
                        .anchor_bookmarks
                        .iter()
                        .all(|b| b.path == std::path::Path::new("folder/a.md"))
                );
                w.apply_relocated_index(
                    std::path::Path::new("folder"),
                    Some(std::path::Path::new("moved")),
                    true,
                    cx,
                );
                assert!(
                    w.ui.prefs
                        .anchor_bookmarks
                        .iter()
                        .all(|b| b.path == std::path::Path::new("moved/a.md"))
                );
                w.apply_relocated_index(std::path::Path::new("moved"), None, true, cx);
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
