//! File tree, filename/full-text search results and sidebar controls.
use super::commands::command;
use super::ui::{NameMode, icon, tool};
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{
    Disableable, Icon, Selectable, Sizable, button::*, list::ListItem, tree::Tree,
};
use inkstone_core::file_order::SortBy;

#[derive(Clone)]
pub(super) struct DraggedFile {
    pub path: PathBuf,
    pub folder: bool,
    pub generation: u64,
}

impl Render for DraggedFile {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(0x363636))
            .text_color(rgb(0xdddddd))
            .child(
                self.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            )
    }
}

impl DraggedFile {
    fn destination(&self, folder: &std::path::Path) -> Option<PathBuf> {
        let destination = folder.join(self.path.file_name()?);
        if destination == self.path || (self.folder && folder.starts_with(&self.path)) {
            return None;
        }
        Some(destination)
    }
}

impl Workspace {
    pub(super) fn open_dropped_file(
        &mut self,
        drag: &DraggedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drag.folder
            || drag.generation != self.generation
            || self.ui.tree_name.is_some()
            || self.modal_is_open()
        {
            return;
        }
        self.open_note_target(drag.path.clone(), None, true, window, cx);
    }

    pub(super) fn drop_tree_file(
        &mut self,
        drag: &DraggedFile,
        folder: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drag.generation != self.generation || self.ui.tree_name.is_some() {
            return;
        }
        let Some(destination) = drag.destination(folder) else {
            return;
        };
        if !drag.folder
            && let Some(id) = self
                .tabs
                .iter()
                .find(|tab| tab.path == drag.path)
                .map(|tab| tab.id)
        {
            self.manage_named_note(
                id,
                false,
                destination.to_string_lossy().to_string(),
                window,
                cx,
            );
        } else {
            self.manage_tree_path(
                drag.path.clone(),
                Some(destination),
                drag.folder,
                window,
                cx,
            );
        }
    }

    fn left_header(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .h(px(40.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(self.border())
            .child(
                tool("files", "folder-closed", "文件列表")
                    .selected(self.ui.left_mode == 0)
                    .toggled(self.ui.left_mode == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.left_mode = 0;
                        cx.notify();
                    })),
            )
            .child(
                tool("search", "search", "搜索")
                    .selected(self.ui.left_mode == 1)
                    .toggled(self.ui.left_mode == 1)
                    .on_click(cx.listener(|this, _, w, cx| this.focus_search(true, w, cx))),
            )
            .child(
                tool("bookmarks", "bookmark", "书签")
                    .selected(self.ui.left_mode == 2)
                    .toggled(self.ui.left_mode == 2)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ui.left_mode = 2;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
    pub(super) fn reveal_current_file(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone())
            .filter(|p| !p.as_os_str().is_empty())
        else {
            return false;
        };
        let tree_path: PathBuf = path.components().collect();
        let id: SharedString = tree_path.to_string_lossy().to_string().into();
        let found = self.tree.update(cx, |tree, cx| {
            tree.reveal_item(&id, ScrollStrategy::Center, cx);
            if let Some(index) = tree.index_of(&id) {
                tree.set_selected_index(Some(index), cx);
                true
            } else {
                false
            }
        });
        if found {
            self.ui.last_revealed_file = Some((self.generation, path));
        }
        found
    }
    pub(super) fn left_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let menu_weak = cx.entity().downgrade();
        let active_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|tab| tab.path.clone());
        let tree_foreground = crate::theme::palette(self.ui.prefs.light).muted;
        let tree_active = crate::theme::palette(self.ui.prefs.light).accent;
        let tree_guide = self.border();
        let guide_depths = self.tree.read(cx).depths();
        let guide_scroll = self.tree.read(cx).scroll_handle().clone();
        let editing = self.ui.tree_name.as_ref().map(|edit| edit.row.clone());
        let name_input = self.name.clone();
        let selected_path = self.ui.tree_active.clone().or(active_path);
        let generation = self.generation;
        let file_tree = Tree::new(&self.tree, move |i, entry, _, _, _| {
            let path = PathBuf::from(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let weak = weak.clone();
            let is_editing = editing.as_ref() == Some(&path);
            let selected = selected_path.as_ref() == Some(&path);
            let label_path = path.clone();
            let label_weak = weak.clone();
            let selector = format!("tree-name-{i}");
            let drop_path = path.clone();
            let drop_weak = weak.clone();
            let external_weak = weak.clone();
            let external_path = path.clone();
            let hover_path = path.clone();
            let drag = DraggedFile {
                path: path.clone(),
                folder,
                generation,
            };
            ListItem::new(i)
                .h(TREE_ROW_HEIGHT)
                .px_1()
                .py_0()
                .rounded(px(4.))
                .text_size(px(MIN_UI_FONT_SIZE))
                .text_color(tree_foreground)
                .accessibility_label(entry.item().label.clone())
                .when(!is_editing, |s| {
                    s.on_drag(drag.clone(), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                })
                .when(folder && !is_editing, |s| {
                    s.drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(tree_guide))
                        .on_drop(move |paths: &ExternalPaths, window, cx| {
                            cx.stop_propagation();
                            let _ = external_weak.update(cx, |this, cx| {
                                this.import_tree_files(
                                    paths.paths().to_vec(),
                                    external_path.clone(),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .drag_over::<DraggedFile>(move |style, drag, _, _| {
                            if drag.generation == generation
                                && drag.destination(&hover_path).is_some()
                            {
                                style.bg(tree_guide)
                            } else {
                                style
                            }
                        })
                        .on_drop(move |drag: &DraggedFile, window, cx| {
                            cx.stop_propagation();
                            let _ = drop_weak.update(cx, |this, cx| {
                                this.drop_tree_file(drag, &drop_path, window, cx);
                            });
                        })
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .w_full()
                        .pl(px(entry.depth() as f32 * 17.))
                        .child(if folder {
                            icon(if entry.is_expanded() {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            })
                            .size(px(16.))
                        } else {
                            Icon::default().size(px(16.))
                        })
                        .child(
                            div()
                                .id(("tree-name", i))
                                .debug_selector(move || selector.clone())
                                .flex_1()
                                .min_w_0()
                                .when(selected, |s| s.text_color(tree_active))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    if !folder || is_editing {
                                        cx.stop_propagation();
                                    }
                                })
                                .when(!is_editing, |s| {
                                    s.on_drag(drag, |drag, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| drag.clone())
                                    })
                                })
                                .when(is_editing, |s| {
                                    s.child(Input::new(&name_input).small().h(px(25.)))
                                })
                                .when(!is_editing, |s| {
                                    s.child(div().truncate().child(if folder {
                                        entry.item().label.to_string()
                                    } else {
                                        entry.item().label.trim_end_matches(".md").to_owned()
                                    }))
                                })
                                .on_click(move |_, window, cx| {
                                    if folder && !is_editing {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    if is_editing {
                                        return;
                                    }
                                    let _ = label_weak.update(cx, |this, cx| {
                                        this.tree.update(cx, |tree, cx| {
                                            tree.set_selected_index(Some(i), cx)
                                        });
                                        if selected {
                                            if this.tabs.iter().any(|tab| tab.path == label_path) {
                                                this.begin_tree_name(
                                                    NameMode::Rename,
                                                    label_path.clone(),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        } else {
                                            this.ui.tree_active = Some(label_path.clone());
                                            if !folder {
                                                this.open_note(label_path.clone(), window, cx);
                                            }
                                            cx.notify();
                                        }
                                    });
                                }),
                        ),
                )
                .on_click(move |_, window, cx| {
                    if !is_editing {
                        let _ = weak.update(cx, |this, cx| {
                            this.ui.tree_active = Some(path.clone());
                            if !folder {
                                this.open_note(path.clone(), window, cx);
                            }
                            cx.notify();
                        });
                    }
                })
        })
        .selection_highlight(false)
        .context_menu(move |_, entry, mut menu, _, _| {
            use gpui_component::menu::PopupMenuItem;
            if entry.is_folder() {
                let path = entry.item().id.to_string();
                let weak = menu_weak.clone();
                menu = menu.item(PopupMenuItem::new("新建笔记").on_click(move |_, w, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.begin_tree_name(NameMode::New, PathBuf::from(&path), w, cx);
                    });
                }));
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                menu = menu.item(PopupMenuItem::new("新建文件夹").on_click(move |_, w, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.begin_tree_name(NameMode::Folder, path.clone(), w, cx)
                    });
                }));
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                menu = menu.item(
                    PopupMenuItem::new("重命名文件夹").on_click(move |_, w, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.begin_tree_name(NameMode::RenameFolder, path.clone(), w, cx);
                        });
                    }),
                );
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                return menu.item(PopupMenuItem::new("将文件夹移入回收站").on_click(
                    move |_, w, cx| {
                        let _ = weak
                            .update(cx, |this, cx| this.manage_folder(path.clone(), None, w, cx));
                    },
                ));
            }

            for &(id, label, _) in [8, 42, 15, 27, 11, 18, 19, 10]
                .iter()
                .filter_map(|id| command(*id))
            {
                let path = PathBuf::from(entry.item().id.as_ref());
                let weak = menu_weak.clone();
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, w, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.ui.pending_command = Some((path.clone(), id));
                        this.open_note(path.clone(), w, cx);
                    });
                }));
            }
            menu
        });
        div()
            .id("workspace-left-panel")
            .debug_selector(|| "workspace-left-panel".into())
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(self.side())
            .child(self.left_header(cx))
            .when(self.ui.left_mode == 0, |s| {
                s.on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    this.import_tree_files(paths.paths().to_vec(), PathBuf::new(), window, cx);
                }))
                .child(
                    div()
                        .flex()
                        .h(px(38.))
                        .items_center()
                        .px_2()
                        .gap_1()
                        .child(
                            tool("new-file", "square-pen", "新建笔记")
                                .on_click(cx.listener(|this, _, w, cx| this.focus_new(w, cx))),
                        )
                        .child(tool("new-folder", "folder-plus", "新建文件夹").on_click(
                            cx.listener(|this, _, w, cx| this.prompt_name(NameMode::Folder, w, cx)),
                        ))
                        .child({
                            let weak = cx.entity().downgrade();
                            let selected = (self.ui.prefs.sort_by, self.ui.prefs.sort_descending);
                            tool("sort", "arrow-up-down", "更改排序方式").dropdown_menu(
                                move |mut menu, _, _| {
                                    for (label, by, descending) in [
                                        ("文件名：A → Z", SortBy::Name, false),
                                        ("文件名：Z → A", SortBy::Name, true),
                                        ("修改时间：从新到旧", SortBy::Modified, true),
                                        ("修改时间：从旧到新", SortBy::Modified, false),
                                        ("创建时间：从新到旧", SortBy::Created, true),
                                        ("创建时间：从旧到新", SortBy::Created, false),
                                    ] {
                                        let weak = weak.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(selected == (by, descending))
                                                .on_click(move |_, _, cx| {
                                                    let _ = weak.update(cx, |this, cx| {
                                                        this.ui.prefs.sort_by = by;
                                                        this.ui.prefs.sort_descending = descending;
                                                        this.rebuild_sorted_tree(cx);
                                                        this.persist_workspace(cx);
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                },
                            )
                        })
                        .child(
                            tool("reveal", "locate", "自动显示当前文件")
                                .when(self.ui.prefs.auto_reveal_file, |s| s.bg(self.bg()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.ui.prefs.auto_reveal_file =
                                        !this.ui.prefs.auto_reveal_file;
                                    this.ui.last_revealed_file = None;
                                    if this.ui.prefs.auto_reveal_file {
                                        this.reveal_current_file(cx);
                                    }
                                    this.persist_workspace(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            tool("collapse", "fold-vertical", "折叠所有文件夹").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.ui.prefs.expanded_folders.clear();
                                    this.rebuild_sorted_tree(cx);
                                }),
                            ),
                        ),
                )
                .child(
                    div()
                        .id("tree-root-drop")
                        .debug_selector(|| "tree-root-drop".into())
                        .mx_3()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .text_size(px(MIN_UI_FONT_SIZE))
                        .text_color(tree_foreground)
                        .child("笔记库根目录")
                        .drag_over::<DraggedFile>(move |style, drag, _, _| {
                            if drag.generation == generation
                                && drag.destination(std::path::Path::new("")).is_some()
                            {
                                style.bg(tree_guide)
                            } else {
                                style
                            }
                        })
                        .on_drop(cx.listener(|this, drag: &DraggedFile, window, cx| {
                            cx.stop_propagation();
                            this.drop_tree_file(drag, std::path::Path::new(""), window, cx);
                        })),
                )
                .child(
                    div().flex_1().min_h_0().px_3().child(
                        // Guides are positioned in this box, not the padded one.
                        // An absolute child of the padded div aligns to the padding
                        // edge, 12px left of the rows.
                        div()
                            .size_full()
                            .relative()
                            .child(file_tree)
                            .child(file_tree_guides(guide_depths, guide_scroll, tree_guide)),
                    ),
                )
            })
            .when(self.ui.left_mode == 1, |s| {
                s.child(self.saved_search_controls(cx))
                    .when(!self.ui.quick_open, |s| {
                        s.child(
                            div().p_3().flex().gap_1().items_center().child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.search)
                                        .prefix(icon("search").size(px(14.)))
                                        .cleanable(true)
                                        .suffix(
                                            Button::new("search-case-sensitive")
                                                .ghost()
                                                .compact()
                                                .label("Aa")
                                                .accessibility_label("区分大小写")
                                                .tooltip("区分大小写")
                                                .toggled(self.ui.prefs.search_case_sensitive)
                                                .selected(self.ui.prefs.search_case_sensitive)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.ui.prefs.search_case_sensitive =
                                                        !this.ui.prefs.search_case_sensitive;
                                                    this.run_search(cx);
                                                    this.persist_workspace(cx);
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                            ),
                        )
                    })
                    .child(
                        div()
                            .px_3()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(format!("{} 项结果", self.search_results.len())),
                            )
                            .child(self.search_sort_button(cx))
                            .child(
                                tool("search-collapse", "fold-vertical", "展开或折叠全部搜索结果")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let paths: std::collections::BTreeSet<_> = this
                                            .search_results
                                            .iter()
                                            .map(|hit| hit.path.clone())
                                            .collect();
                                        if paths.is_subset(&this.ui.search_collapsed) {
                                            this.ui.search_collapsed.clear();
                                        } else {
                                            this.ui.search_collapsed = paths;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(!self.ui.search_error.is_empty(), |s| {
                        s.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .line_height(relative(1.4))
                                .whitespace_normal()
                                .text_color(rgb(if self.ui.prefs.light {
                                    0xb42318
                                } else {
                                    0xfda29b
                                }))
                                .child(self.ui.search_error.clone()),
                        )
                    })
                    .child(self.search_list(false, cx))
            })
            .when(self.ui.left_mode == 2, |s| s.child(self.bookmark_panel(cx)))
            .into_any_element()
    }
    pub(super) fn rebuild_sorted_tree(&mut self, cx: &mut Context<Self>) {
        let mut paths = self.tree_files.clone();
        paths.extend(self.ui.folders.iter().cloned());
        if let Some(edit) = &self.ui.tree_name
            && edit.creating
        {
            paths.push(edit.row.clone());
        }
        let mut items = make_tree(&paths);
        fn apply(
            items: Vec<TreeItem>,
            folders: &std::collections::HashSet<&std::path::Path>,
            expanded: &std::collections::HashSet<&std::path::Path>,
            index: &Index,
            by: SortBy,
            descending: bool,
        ) -> Vec<TreeItem> {
            // Consume each node instead of cloning all its descendants once
            // per ancestor. Membership checks must not scan every folder for
            // every row on the UI thread.
            let mut items: Vec<_> = items
                .into_iter()
                .map(|mut item| {
                    let path = std::path::Path::new(item.id.as_ref());
                    let folder = item.is_folder() || folders.contains(path);
                    let open = expanded.contains(path);
                    item.children = apply(item.children, folders, expanded, index, by, descending);
                    item.folder(folder).expanded(open)
                })
                .collect();
            items.sort_by(|a, b| {
                let times = |item: &TreeItem| {
                    index
                        .notes
                        .get(std::path::Path::new(item.id.as_ref()))
                        .map(|n| n.times)
                        .unwrap_or_default()
                };
                inkstone_core::file_order::compare(
                    (&a.label, a.is_folder(), times(a)),
                    (&b.label, b.is_folder(), times(b)),
                    by,
                    descending,
                )
            });
            items
        }
        if let Some(edit) = &self.ui.tree_name
            && edit.creating
        {
            fn mark(items: &mut [TreeItem], row: &std::path::Path, folder: bool) {
                for item in items {
                    if std::path::Path::new(item.id.as_ref()) == row {
                        *item = item.clone().folder(folder);
                        item.label = if folder {
                            "未命名文件夹"
                        } else {
                            "未命名"
                        }
                        .into();
                    }
                    mark(&mut item.children, row, folder);
                }
            }
            mark(&mut items, &edit.row, edit.folder);
        }
        let folders = self.ui.folders.iter().map(PathBuf::as_path).collect();
        let expanded = self
            .ui
            .prefs
            .expanded_folders
            .iter()
            .map(PathBuf::as_path)
            .collect();
        let items = apply(
            items,
            &folders,
            &expanded,
            &self.index,
            self.ui.prefs.sort_by,
            self.ui.prefs.sort_descending,
        );
        self.tree.update(cx, |tree, cx| {
            let selected = tree.selected_item().cloned();
            tree.set_items(items, cx);
            tree.set_selected_item(selected.as_ref(), cx);
        });
        cx.notify();
    }
    pub(super) fn open_selected_result(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let hit = self
            .visible_search_hits(self.ui.quick_open, cx)
            .get(self.ui.selected)
            .map(|h| (h.path.clone(), h.offset));
        if let Some((path, offset)) = hit {
            self.pending_jump = Some((path.clone(), offset));
            self.open_note(path, w, cx);
            self.apply_jump(w, cx);
        }
    }
    pub(super) fn visible_search_hits(&self, modal: bool, cx: &Context<Self>) -> Vec<SearchHit> {
        if modal && self.search.read(cx).value().is_empty() {
            self.files
                .iter()
                .take(100)
                .map(|p| SearchHit {
                    path: p.clone(),
                    display_name: None,
                    offset: 0,
                    line: 1,
                    excerpt: String::new(),
                    highlights: vec![],
                    title_highlights: vec![],
                })
                .collect()
        } else {
            self.search_results.clone()
        }
    }
    pub(super) fn search_list(&self, modal: bool, cx: &mut Context<Self>) -> AnyElement {
        if !modal && self.fulltext {
            return self.grouped_search_results(cx);
        }
        let hits = self.visible_search_hits(modal, cx);
        let empty = hits.is_empty();
        div()
            .id(if modal {
                "quick-results"
            } else {
                "search-results"
            })
            .when(modal, |s| s.track_scroll(&self.ui.modal_scroll))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .when(empty, |s| {
                s.child(self.empty_state(
                    "search",
                    "未找到笔记",
                    "试试其他关键词，或创建一篇新笔记。",
                ))
            })
            .children(hits.into_iter().enumerate().map(|(i, hit)| {
                let path = hit.path;
                let offset = hit.offset;
                div()
                    .id(("result", i))
                    .when(modal && self.ui.selected == i, |s| {
                        s.bg(crate::theme::palette(self.ui.prefs.light).selected)
                    })
                    .p_2()
                    .when(modal, |s| s.py(px(6.)).px_3().h(px(38.)).overflow_hidden())
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::palette(self.ui.prefs.light).selected))
                    .child(div().text_size(px(14.)).truncate().child(
                        hit.display_name.unwrap_or_else(|| {
                            if modal {
                                return path
                                    .with_extension("")
                                    .to_string_lossy()
                                    .replace('\\', "/");
                            }
                            path.file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string()
                        }),
                    ))
                    .when(!modal, |s| {
                        s.child(
                            div()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                .truncate()
                                .child(if hit.excerpt.is_empty() {
                                    path.to_string_lossy().to_string()
                                } else {
                                    format!("{}: {}", hit.line, hit.excerpt)
                                }),
                        )
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.pending_jump = Some((path.clone(), offset));
                        this.open_note(path.clone(), w, cx);
                        this.apply_jump(w, cx);
                    }))
            }))
            .into_any_element()
    }

    fn search_sort_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = (
            self.ui.prefs.search_sort_by,
            self.ui.prefs.search_descending,
        );
        let weak = cx.entity().downgrade();
        tool("search-sort", "arrow-up-down", "搜索结果排序")
            .dropdown_menu(move |mut menu, _, _| {
                for (by, descending, label) in [
                    (SortBy::Name, false, "文件名（升序）"),
                    (SortBy::Name, true, "文件名（降序）"),
                    (SortBy::Modified, true, "修改时间（从新到旧）"),
                    (SortBy::Modified, false, "修改时间（从旧到新）"),
                    (SortBy::Created, true, "创建时间（从新到旧）"),
                    (SortBy::Created, false, "创建时间（从旧到新）"),
                ] {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(current == (by, descending))
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.ui.prefs.search_sort_by = by;
                                    this.ui.prefs.search_descending = descending;
                                    this.ui.search_group_scroll.set_offset(Point::default());
                                    this.run_search(cx);
                                    this.persist_workspace(cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn grouped_search_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut groups: Vec<(PathBuf, Vec<SearchHit>)> = vec![];
        let mut positions = std::collections::HashMap::new();
        for hit in &self.search_results {
            let position = *positions.entry(hit.path.clone()).or_insert_with(|| {
                groups.push((hit.path.clone(), vec![]));
                groups.len() - 1
            });
            groups[position].1.push(hit.clone());
        }
        div()
            .id("grouped-search-results")
            .track_scroll(&self.ui.search_group_scroll)
            .on_scroll_wheel(cx.listener(|_, _, w, cx| {
                cx.defer_in(w, |this, _, cx| {
                    let scroll = &this.ui.search_group_scroll;
                    if scroll.max_offset().y + scroll.offset().y <= px(120.) {
                        this.load_more_search(cx);
                    }
                });
            }))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .when(
                groups.is_empty()
                    && !self.ui.search_loading
                    && self.ui.search_error.is_empty()
                    && !self.search.read(cx).value().trim().is_empty(),
                |s| {
                    s.child(
                        div()
                            .p_2()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                            .child("未找到匹配结果"),
                    )
                },
            )
            .children(groups.into_iter().enumerate().map(|(i, (path, hits))| {
                let collapsed = self.ui.search_collapsed.contains(&path);
                let fold_path = path.clone();
                let open_path = path.clone();
                let offset = hits[0].offset;
                let mut title = path.to_string_lossy().replace('\\', "/");
                if self
                    .tabs
                    .iter()
                    .any(|t| t.path == path && t.save.persistence.is_dirty())
                {
                    title.push_str(" · 未保存");
                }
                let title_highlights = hits[0].title_highlights.clone();
                div()
                    .id(("search-group", i))
                    .mb_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(32.))
                            .child(
                                Button::new(("search-group-fold", i))
                                    .ghost()
                                    .compact()
                                    .w(px(20.))
                                    .label(if collapsed { "›" } else { "⌄" })
                                    .accessibility_label(format!("展开或折叠 {}", path.display()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.ui.search_collapsed.remove(&fold_path) {
                                            this.ui.search_collapsed.insert(fold_path.clone());
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("search-group-file", i))
                                    .ghost()
                                    .compact()
                                    .flex_1()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .min_w_0()
                                    .justify_start()
                                    .overflow_hidden()
                                    .accessibility_label(format!("打开 {title}"))
                                    .child(StyledText::new(title).with_highlights(
                                        title_highlights.into_iter().map(|range| {
                                            (
                                                range,
                                                HighlightStyle {
                                                    background_color: Some(
                                                        rgba(if self.ui.prefs.light {
                                                            0xf4d03f66
                                                        } else {
                                                            0x9e7d2866
                                                        })
                                                        .into(),
                                                    ),
                                                    font_weight: Some(FontWeight::SEMIBOLD),
                                                    ..Default::default()
                                                },
                                            )
                                        }),
                                    ))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.pending_jump = Some((open_path.clone(), offset));
                                        this.open_note(open_path.clone(), w, cx);
                                        this.apply_jump(w, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                    .child(hits.len().to_string()),
                            ),
                    )
                    .children(hits.into_iter().filter(|_| !collapsed).enumerate().map(
                        |(j, hit)| {
                            let path = path.clone();
                            div()
                                .id(("search-line", j))
                                .px_3()
                                .py_2()
                                .border_1()
                                .when(j > 0, |s| s.border_t_0())
                                .border_color(self.border())
                                .when(j == 0, |s| s.rounded_t(px(4.)))
                                .cursor_pointer()
                                .hover(|s| {
                                    s.bg(crate::theme::palette(self.ui.prefs.light).selected)
                                })
                                .child(
                                    div()
                                        .text_size(px(MIN_UI_FONT_SIZE))
                                        .line_height(relative(1.3))
                                        .whitespace_normal()
                                        .child(
                                            StyledText::new(hit.excerpt.trim_end().to_string())
                                                .with_highlights(
                                                    hit.highlights.iter().cloned().filter_map(
                                                        |mut range| {
                                                            range.end = range
                                                                .end
                                                                .min(hit.excerpt.trim_end().len());
                                                            if range.start >= range.end {
                                                                return None;
                                                            }
                                                            Some((
                                                                range,
                                                                HighlightStyle {
                                                                    background_color: Some(
                                                                        rgba(
                                                                            if self.ui.prefs.light {
                                                                                0xf4d03f66
                                                                            } else {
                                                                                0x9e7d2866
                                                                            },
                                                                        )
                                                                        .into(),
                                                                    ),
                                                                    font_weight: Some(
                                                                        FontWeight::SEMIBOLD,
                                                                    ),
                                                                    ..Default::default()
                                                                },
                                                            ))
                                                        },
                                                    ),
                                                ),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.pending_jump = Some((path.clone(), hit.offset));
                                    this.open_note(path.clone(), w, cx);
                                    this.apply_jump(w, cx);
                                }))
                        },
                    ))
            }))
            .when(
                self.ui.search_loading && self.search_results.is_empty(),
                |s| {
                    s.child(
                        div()
                            .p_2()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .child("正在搜索…"),
                    )
                },
            )
            .when(self.ui.search_has_more, |s| {
                s.child(
                    Button::new("search-load-more")
                        .ghost()
                        .w_full()
                        .label(if self.ui.search_loading {
                            "正在载入…"
                        } else {
                            "显示更多结果"
                        })
                        .disabled(self.ui.search_loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load_more_search(cx))),
                )
            })
            .into_any_element()
    }
}

/// Row height of a file-tree entry. Guide lines use the same pitch so a column
/// stays one stroke instead of a stack of row-sized segments.
const TREE_ROW_HEIGHT: Pixels = px(27.);
/// Horizontal step between nested guide columns, matching the row indent.
const TREE_INDENT: Pixels = px(17.);
/// Guide x within a row: `px_1` (4px) plus the center of the 16px chevron.
const TREE_GUIDE_X: Pixels = px(12.);

/// One continuous indent guide: `depth` is the ancestor column, and the line
/// covers rows `start..end`.
struct TreeGuide {
    depth: usize,
    start: usize,
    end: usize,
}

/// Merge per-row indent marks into column runs.
///
/// A row used to paint its own segment, and that segment only covered the
/// text line, so every row boundary left a gap. A run is every consecutive
/// span where `depth` stays strictly above the column.
fn tree_guides(depths: &[usize]) -> Vec<TreeGuide> {
    let mut guides = Vec::new();
    // Start row of the open run in each ancestor column, nearest last.
    let mut open: Vec<Option<usize>> = Vec::new();
    for (ix, &depth) in depths.iter().enumerate() {
        while open.len() > depth {
            if let Some(start) = open.pop().flatten() {
                guides.push(TreeGuide {
                    depth: open.len(),
                    start,
                    end: ix,
                });
            }
        }
        while open.len() < depth {
            open.push(None);
        }
        for slot in &mut open {
            if slot.is_none() {
                *slot = Some(ix);
            }
        }
    }
    let len = depths.len();
    while let Some(start) = open.pop().flatten() {
        guides.push(TreeGuide {
            depth: open.len(),
            start,
            end: len,
        });
    }
    guides
}

/// Indent guides for the file tree, one stroke per column.
///
/// Painted after the rows so a line is not cut by the next row, and clipped
/// to the viewport. Disclosure icons sit in a column the line does not cross.
fn file_tree_guides(
    depths: Vec<usize>,
    scroll: gpui::UniformListScrollHandle,
    color: Rgba,
) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let guides = tree_guides(&depths);
            if guides.is_empty() {
                return;
            }
            // Read during paint, after the list has clamped this frame's offset.
            // A value captured at render time is one frame behind the scroll.
            let offset_y = scroll.0.borrow().base_handle.offset().y;
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                for guide in guides {
                    let x = bounds.left() + TREE_GUIDE_X + TREE_INDENT * guide.depth as f32;
                    let top = bounds.top() + offset_y + TREE_ROW_HEIGHT * guide.start as f32;
                    let height = TREE_ROW_HEIGHT * (guide.end - guide.start) as f32;
                    window.paint_quad(fill(
                        Bounds::from_corners(point(x, top), point(x + px(1.), top + height)),
                        color,
                    ));
                }
            });
        },
    )
    .absolute()
    .inset_0()
    .into_any_element()
}

#[cfg(test)]
mod tree_guide_tests {
    use super::tree_guides;

    #[test]
    fn tree_guides_join_a_column_and_stop_at_the_next_sibling() {
        // Two expanded folders, the first with a nested child, then a file.
        let depths = [0, 1, 2, 2, 1, 0, 1, 0];
        let guides: Vec<_> = tree_guides(&depths)
            .into_iter()
            .map(|guide| (guide.depth, guide.start, guide.end))
            .collect();
        assert_eq!(guides, vec![(1, 2, 4), (0, 1, 5), (0, 6, 7)]);
    }
}
