//! Right sidebar: outline, links, tags and properties.
use super::ui::{icon, tool};
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Disableable, Selectable, button::*, list::ListItem};

#[derive(Default)]
pub(super) struct OutlineState {
    key: Option<(PathBuf, Vec<inkstone_core::index::Heading>)>,
    collapsed: std::collections::BTreeSet<usize>,
}

impl Workspace {
    fn right_header(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .h(px(40.))
            .px_1()
            .gap_0()
            .border_b_1()
            .border_color(self.border())
            .children(
                [
                    (0, "list", "大纲"),
                    (1, "links", "反向链接"),
                    (2, "link", "出链"),
                    (3, "tags", "标签"),
                    (4, "inbox", "属性"),
                ]
                .iter()
                .map(|&(i, ico, label)| {
                    Button::new(("right-mode", i))
                        .accessibility_id(format!("right-mode-{i}"))
                        .accessibility_label(label)
                        .selected(self.ui.right_mode == i)
                        .toggled(self.ui.right_mode == i)
                        .ghost()
                        .compact()
                        .icon(icon(ico))
                        .tooltip(label)
                        .w(px(32.))
                        .h(px(28.))
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.ui.right_mode = i;
                            if i == 3 {
                                w.focus(&this.ui.tags_focus, cx);
                            }
                            cx.notify();
                        }))
                }),
            )
            .into_any_element()
    }
    pub(super) fn right_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pane = self.current_pane();
        let headings = pane
            .as_ref()
            .map(|p| p.read(cx).parsed.headings.clone())
            .unwrap_or_default();
        let outline_query = if self.ui.outline_filter_open {
            self.ui
                .outline_filter
                .read(cx)
                .value()
                .trim()
                .to_lowercase()
        } else {
            String::new()
        };
        let fold_key = (
            pane.as_ref()
                .map(|p| p.read(cx).current_path.clone())
                .unwrap_or_default(),
            headings.clone(),
        );
        let empty_folds = Default::default();
        let collapsed = if self.ui.outline.key.as_ref() == Some(&fold_key) {
            &self.ui.outline.collapsed
        } else {
            &empty_folds
        };
        let headings = outline_rows(&headings, collapsed, &outline_query);
        let links = pane
            .as_ref()
            .map(|p| p.read(cx).parsed.links.clone())
            .unwrap_or_default();
        let properties = pane
            .as_ref()
            .map(|p| p.update(cx, |pane, cx| pane.properties(cx).to_vec()))
            .unwrap_or_default();
        let tab_path = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        div()
            .id("workspace-right-panel")
            .debug_selector(|| "workspace-right-panel".into())
            .flex()
            .flex_col()
            .child(self.right_header(cx))
            .size_full()
            .bg(self.bg())
            .child(
                div()
                    .id("right-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .when(self.ui.right_mode == 0, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(28.))
                                .mb_2()
                                .child(
                                    tool("outline-filter", "search", "筛选大纲")
                                        .selected(self.ui.outline_filter_open)
                                        .toggled(self.ui.outline_filter_open)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.ui.outline_filter_open =
                                                !this.ui.outline_filter_open;
                                            if this.ui.outline_filter_open {
                                                this.ui.outline_filter.update(cx, |input, cx| {
                                                    input.focus(window, cx)
                                                });
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    tool("outline-collapse", "fold-vertical", "全部展开或折叠标题")
                                        .on_click(cx.listener({
                                            let key = fold_key.clone();
                                            move |this, _, _, cx| {
                                                this.toggle_outline_fold(key.clone(), None, cx)
                                            }
                                        })),
                                ),
                        )
                        .when(self.ui.outline_filter_open, |s| {
                            s.child(
                                div()
                                    .mb_2()
                                    .child(Input::new(&self.ui.outline_filter).cleanable(true)),
                            )
                        })
                        .when(headings.is_empty(), |s| {
                            s.child(
                                div()
                                    .w_full()
                                    .pt(px(28.))
                                    .text_center()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                    .child(if outline_query.is_empty() {
                                        "笔记中的标题会显示在这里"
                                    } else {
                                        "没有匹配的标题"
                                    }),
                            )
                        })
                        .children(headings.into_iter().enumerate().map(
                            |(i, (h, has_children, folded))| {
                                let pane = pane.clone();
                                let key = fold_key.clone();
                                let offset = h.offset;
                                div()
                                    .id(("outline", i))
                                    .h(px(27.))
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .rounded(px(4.))
                                    .hover(|s| s.bg(rgba(0x88888818)))
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .pl(px((h.level - 1) as f32 * 17.))
                                    .cursor_pointer()
                                    .child(div().w(px(18.)).flex_shrink_0().when(
                                        has_children,
                                        |s| {
                                            s.child(
                                                Button::new(("outline-fold", offset))
                                                    .ghost()
                                                    .compact()
                                                    .w(px(18.))
                                                    .h(px(24.))
                                                    .icon(
                                                        icon(if folded {
                                                            "chevron-right"
                                                        } else {
                                                            "chevron-down"
                                                        })
                                                        .size(px(14.)),
                                                    )
                                                    .accessibility_label(format!(
                                                        "展开或折叠 {}",
                                                        h.title
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.toggle_outline_fold(
                                                                key.clone(),
                                                                Some(offset),
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                            )
                                        },
                                    ))
                                    .child(div().flex_1().min_w_0().truncate().child(h.title))
                                    .on_click(cx.listener(move |_, _, w, cx| {
                                        if let Some(pane) = &pane {
                                            pane.update(cx, |p, cx| p.jump(h.offset, w, cx));
                                        }
                                    }))
                            },
                        ))
                    })
                    .when(self.ui.right_mode == 1, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(32.))
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                .child("链接当前文件")
                                .child(self.backlinks.len().to_string()),
                        )
                        .when(self.backlinks.is_empty(), |s| {
                            s.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(rgb(0x777777))
                                    .child("没有笔记链接当前文件"),
                            )
                        })
                        .child(
                            uniform_list(
                                "sidebar-backlinks",
                                self.backlinks.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .filter_map(|i| {
                                            this.backlinks.get(i).cloned().map(|path| {
                                                ListItem::new(("backlink", i))
                                                    .h(px(27.))
                                                    .px_2()
                                                    .text_size(px(MIN_UI_FONT_SIZE))
                                                    .rounded(px(4.))
                                                    .child(
                                                        div().truncate().child(
                                                            path.to_string_lossy().to_string(),
                                                        ),
                                                    )
                                                    .on_click(cx.listener(move |this, _, w, cx| {
                                                        this.open_note(path.clone(), w, cx)
                                                    }))
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.backlink_scroll)
                            .h(px((self.backlinks.len() as f32 * 27.).min(400.)))
                            .w_full(),
                        )
                    })
                    .when(self.ui.right_mode == 2, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(32.))
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                .child("当前笔记中的链接")
                                .child(links.len().to_string()),
                        )
                        .children(links.into_iter().enumerate().map(|(i, link)| {
                            let from = tab_path.clone();
                            div()
                                .id(("outlink", i))
                                .h(px(27.))
                                .px_2()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .rounded(px(4.))
                                .hover(|s| s.bg(rgba(0x88888818)))
                                .py_1()
                                .text_color(crate::theme::palette(self.ui.prefs.light).accent)
                                .cursor_pointer()
                                .child(div().truncate().child(link.label))
                                .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                                    if let Some(from) = &from {
                                        this.follow_link(
                                            from.clone(),
                                            link.target.clone(),
                                            event.modifiers().secondary(),
                                            w,
                                            cx,
                                        );
                                    }
                                }))
                        }))
                    })
                    .when(self.ui.right_mode == 4, |s| {
                        s.children(properties.iter().enumerate().map(|(i, p)| {
                            let name = p.name.clone();
                            let value = p.value.clone();
                            let kind = self
                                .ui
                                .prefs
                                .property_types
                                .get(&name.to_lowercase())
                                .copied()
                                .unwrap_or_else(|| inkstone_core::properties::Kind::infer(&value));
                            let symbol = if name == "tags" {
                                "tags"
                            } else {
                                match kind {
                                    inkstone_core::properties::Kind::Number => "hash",
                                    inkstone_core::properties::Kind::Checkbox => "check-square",
                                    inkstone_core::properties::Kind::Date
                                    | inkstone_core::properties::Kind::DateTime => "calendar",
                                    inkstone_core::properties::Kind::List => "list",
                                    _ => "text",
                                }
                            };
                            let display = match serde_json::from_str::<serde_json::Value>(&value) {
                                Ok(serde_json::Value::String(text)) => text,
                                Ok(serde_json::Value::Null) => String::new(),
                                Ok(serde_json::Value::Array(items)) => items
                                    .iter()
                                    .map(|item| {
                                        item.as_str()
                                            .map(str::to_owned)
                                            .unwrap_or_else(|| item.to_string())
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                _ => value.clone(),
                            };
                            div()
                                .id(("property", i))
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_h(px(34.))
                                .px_2()
                                .py_1()
                                .rounded(px(4.))
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(0x88888818)))
                                .child(
                                    icon(symbol).size(px(16.)).text_color(
                                        crate::theme::palette(self.ui.prefs.light).muted,
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(100.))
                                        .flex_shrink_0()
                                        .truncate()
                                        .text_color(
                                            crate::theme::palette(self.ui.prefs.light).muted,
                                        )
                                        .child(name.clone()),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(display))
                                .on_click(cx.listener(move |this, _, w, cx| {
                                    this.edit_property(&name, &value, w, cx)
                                }))
                        }))
                        .child(
                            Button::new("new-property")
                                .ghost()
                                .w_full()
                                .justify_start()
                                .h(px(34.))
                                .accessibility_label("添加笔记属性")
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_size(px(MIN_UI_FONT_SIZE))
                                        .text_color(
                                            crate::theme::palette(self.ui.prefs.light).muted,
                                        )
                                        .child(icon("plus").size(px(14.)))
                                        .child("添加笔记属性"),
                                )
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.edit_property("", "", w, cx)),
                                ),
                        )
                    })
                    .when(self.ui.right_mode == 3, |s| {
                        s.child(self.tags_panel(window, cx))
                    }),
            )
            .into_any_element()
    }
}

impl Workspace {
    pub(super) fn navigate_tags(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.navigate_tags_modified(key, false, window, cx)
    }
    fn navigate_tags_modified(
        &mut self,
        key: &str,
        combine: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use inkstone_core::tags::{Navigation, rows};
        let action = inkstone_core::tags::navigate(
            &rows(&self.index, &self.ui.prefs.tags),
            self.ui.tags_selected.as_deref(),
            key,
        );
        match action {
            Navigation::None => return false,
            Navigation::Select(tag) => self.ui.tags_selected = Some(tag.to_lowercase()),
            Navigation::Fold(tag, collapsed) => {
                let key = tag.to_lowercase();
                self.ui.tags_selected = Some(key.clone());
                if collapsed {
                    self.ui.prefs.tags.collapsed.insert(key);
                } else {
                    self.ui.prefs.tags.collapsed.remove(&key);
                }
                self.persist_workspace(cx);
            }
            Navigation::Open(tag) => {
                self.search_tag(&tag, combine, window, cx);
            }
        }
        if let Some(i) = rows(&self.index, &self.ui.prefs.tags)
            .iter()
            .position(|row| Some(row.tag.to_lowercase()) == self.ui.tags_selected)
        {
            self.ui.tags_scroll.scroll_to_item(i);
        }
        cx.notify();
        true
    }
    pub(super) fn search_tag(
        &mut self,
        tag: &str,
        combine: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = if self.fulltext {
            self.search.read(cx).value().to_string()
        } else {
            self.ui.prefs.search_query.clone()
        };
        let query = inkstone_core::tags::search_query(&current, tag, combine);
        self.ui.prefs.search_query = query.clone();
        self.search
            .update(cx, |s, cx| s.set_value(query, window, cx));
        self.focus_search(true, window, cx);
    }
    fn tags_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        use inkstone_core::tags::{Sort, rows};
        let options = &self.ui.prefs.tags;
        let items = rows(&self.index, options);
        let sort = options.sort;
        let weak = cx.entity().downgrade();
        let sorting =
            tool("tags-sort", "arrow-up-down", "标签排序").dropdown_menu(move |mut menu, _, _| {
                for (value, label) in [
                    (Sort::Name, "标签名（升序）"),
                    (Sort::NameDescending, "标签名（降序）"),
                    (Sort::Frequency, "使用次数（从多到少）"),
                    (Sort::FrequencyAscending, "使用次数（从少到多）"),
                ] {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(sort == value).on_click(
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.ui.prefs.tags.sort = value;
                                this.persist_workspace(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
        div()
            .track_focus(&self.ui.tags_focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                if this.ui.tags_focus.is_focused(w)
                    && this.navigate_tags_modified(
                        &event.keystroke.key,
                        event.keystroke.modifiers.control,
                        w,
                        cx,
                    )
                {
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pb_2()
                    .child(sorting)
                    .child(
                        tool("tags-hierarchy", "network", "显示嵌套标签")
                            .toggled(options.hierarchy)
                            .selected(options.hierarchy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ui.prefs.tags.hierarchy = !this.ui.prefs.tags.hierarchy;
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        tool("tags-collapse", "fold-vertical", "全部展开或折叠")
                            .disabled(!options.hierarchy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut expanded = this.ui.prefs.tags.clone();
                                expanded.collapsed.clear();
                                expanded.show_filter = false;
                                let parents: std::collections::BTreeSet<_> =
                                    rows(&this.index, &expanded)
                                        .into_iter()
                                        .filter(|row| row.children)
                                        .map(|row| row.tag.to_lowercase())
                                        .collect();
                                if parents.is_subset(&this.ui.prefs.tags.collapsed) {
                                    this.ui.prefs.tags.collapsed.clear();
                                } else {
                                    this.ui.prefs.tags.collapsed = parents;
                                }
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        tool("tags-filter", "search", "筛选标签")
                            .toggled(options.show_filter)
                            .selected(options.show_filter)
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.ui.prefs.tags.show_filter = !this.ui.prefs.tags.show_filter;
                                if this.ui.prefs.tags.show_filter {
                                    this.ui.tags_filter.update(cx, |s, cx| s.focus(w, cx));
                                } else {
                                    this.ui.prefs.tags.query.clear();
                                    this.ui
                                        .tags_filter
                                        .update(cx, |s, cx| s.set_value("", w, cx));
                                }
                                this.persist_workspace(cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(options.show_filter, |s| {
                s.child(
                    div()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, w, cx| {
                            if event.keystroke.key == "down" {
                                w.focus(&this.ui.tags_focus, cx);
                                this.navigate_tags("down", w, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(Input::new(&self.ui.tags_filter)),
                )
            })
            .when_some(
                if options.show_filter {
                    inkstone_core::search::Query::parse(&options.query).err()
                } else {
                    None
                },
                |s, error| {
                    s.child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(rgb(0xe87979))
                            .child(error),
                    )
                },
            )
            .when(
                items.is_empty() && options.show_filter && !options.query.is_empty(),
                |s| {
                    s.child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                            .child(if options.show_filter && !options.query.is_empty() {
                                "未找到匹配标签"
                            } else {
                                "没有标签"
                            }),
                    )
                },
            )
            .child(
                div()
                    .id("tag-rows")
                    .max_h((window.viewport_size().height - px(210.)).max(px(100.)))
                    .overflow_y_scroll()
                    .track_scroll(&self.ui.tags_scroll)
                    .children(items.into_iter().enumerate().map(|(i, row)| {
                        let tag = row.tag;
                        let fold_key = tag.to_lowercase();
                        let label = if options.hierarchy {
                            tag.rsplit('/').next().unwrap_or(&tag).to_string()
                        } else {
                            tag.clone()
                        };
                        div()
                            .id(("tag", i))
                            .when(
                                self.ui.tags_selected.as_deref() == Some(fold_key.as_str()),
                                |s| s.bg(rgba(0x88888833)),
                            )
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(27.))
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .rounded(px(4.))
                            .py_1()
                            .pl(px(row.depth as f32 * 17.))
                            .cursor_pointer()
                            .hover(|s| s.bg(crate::theme::palette(self.ui.prefs.light).selected))
                            .child(div().w(px(20.)).flex_shrink_0().when(row.children, |s| {
                                s.child(
                                    Button::new(("tag-fold", i))
                                        .ghost()
                                        .compact()
                                        .w(px(20.))
                                        .h(px(24.))
                                        .label(if row.collapsed { "›" } else { "⌄" })
                                        .accessibility_label(format!("展开或折叠标签 {tag}"))
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            cx.stop_propagation();
                                            this.ui.tags_selected = Some(fold_key.clone());
                                            w.focus(&this.ui.tags_focus, cx);
                                            if !this.ui.prefs.tags.collapsed.remove(&fold_key) {
                                                this.ui
                                                    .prefs
                                                    .tags
                                                    .collapsed
                                                    .insert(fold_key.clone());
                                            }
                                            this.persist_workspace(cx);
                                            cx.notify();
                                        })),
                                )
                            }))
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            .child(
                                div()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(crate::theme::palette(self.ui.prefs.light).muted)
                                    .child(row.count.to_string()),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, w, cx| {
                                this.search_tag(&tag, event.modifiers().control, w, cx);
                            }))
                    })),
            )
            .into_any_element()
    }
}

fn outline_rows(
    headings: &[inkstone_core::index::Heading],
    collapsed: &std::collections::BTreeSet<usize>,
    query: &str,
) -> Vec<(inkstone_core::index::Heading, bool, bool)> {
    let mut hidden_below = None;
    headings
        .iter()
        .enumerate()
        .filter_map(|(i, heading)| {
            let has_children = headings
                .get(i + 1)
                .is_some_and(|next| next.level > heading.level);
            let folded = query.is_empty() && collapsed.contains(&heading.offset);
            if !query.is_empty() {
                return heading
                    .title
                    .to_lowercase()
                    .contains(query)
                    .then(|| (heading.clone(), has_children, false));
            }
            if hidden_below.is_some_and(|level| heading.level > level) {
                return None;
            }
            hidden_below = if folded { Some(heading.level) } else { None };
            Some((heading.clone(), has_children, folded))
        })
        .collect()
}

impl Workspace {
    fn toggle_outline_fold(
        &mut self,
        key: (PathBuf, Vec<inkstone_core::index::Heading>),
        offset: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.ui.outline.key.as_ref() != Some(&key) {
            self.ui.outline.collapsed.clear();
            self.ui.outline.key = Some(key.clone());
        }
        if let Some(offset) = offset {
            if !self.ui.outline.collapsed.remove(&offset) {
                self.ui.outline.collapsed.insert(offset);
            }
        } else {
            let parents: std::collections::BTreeSet<_> = key
                .1
                .windows(2)
                .filter(|pair| pair[1].level > pair[0].level)
                .map(|pair| pair[0].offset)
                .collect();
            if parents.is_subset(&self.ui.outline.collapsed) {
                self.ui.outline.collapsed.clear();
            } else {
                self.ui.outline.collapsed = parents;
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod outline_tests {
    use super::outline_rows;

    #[test]
    fn outline_folds_respect_hierarchy_and_filter_reveals_hidden_matches() {
        let headings: Vec<_> = [1, 3, 4, 2, 1]
            .into_iter()
            .enumerate()
            .map(|(offset, level)| inkstone_core::index::Heading {
                level,
                offset,
                title: format!("Section {offset}"),
            })
            .collect();
        let offsets = |rows: Vec<(inkstone_core::index::Heading, bool, bool)>| {
            rows.into_iter().map(|row| row.0.offset).collect::<Vec<_>>()
        };
        assert_eq!(
            offsets(outline_rows(&headings, &[0].into(), "")),
            vec![0, 4]
        );
        assert_eq!(
            offsets(outline_rows(&headings, &[1].into(), "")),
            vec![0, 1, 3, 4]
        );
        assert_eq!(
            offsets(outline_rows(&headings, &[0, 1].into(), "section 2")),
            vec![2]
        );
        assert!(!outline_rows(&headings, &Default::default(), "")[4].1);
    }
}
