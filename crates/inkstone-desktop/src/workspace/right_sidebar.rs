//! Right sidebar: outline, links and properties.
use super::ui::{icon, tool};
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{Selectable, button::*, list::ListItem};
use std::rc::Rc;

type OutlineKey = Rc<(PathBuf, Vec<inkstone_core::index::Heading>)>;
type OutlineRows = Rc<Vec<(inkstone_core::index::Heading, bool, bool)>>;

struct OutlineProjection {
    key: OutlineKey,
    numbered: bool,
    query: String,
    collapsed: std::collections::BTreeSet<usize>,
    rows: OutlineRows,
}

#[derive(Default)]
pub(super) struct OutlineState {
    key: Option<OutlineKey>,
    projection: Option<OutlineProjection>,
    scroll: UniformListScrollHandle,
    collapsed: std::collections::BTreeSet<usize>,
}

impl OutlineState {
    fn project(
        &mut self,
        path: &std::path::Path,
        headings: &[inkstone_core::index::Heading],
        query: &str,
        numbered: bool,
    ) -> (OutlineKey, OutlineRows) {
        let empty = Default::default();
        let collapsed = if self
            .key
            .as_ref()
            .is_some_and(|key| key.0 == path && key.1 == headings)
        {
            &self.collapsed
        } else {
            &empty
        };
        if let Some(cached) = &self.projection
            && cached.key.0 == path
            && cached.key.1 == headings
            && cached.numbered == numbered
            && cached.query == query
            && cached.collapsed == *collapsed
        {
            return (cached.key.clone(), cached.rows.clone());
        }
        // Keep only the current projection. Compare content rather than length:
        // edits may rename headings or move source offsets without changing count.
        let key = Rc::new((path.to_path_buf(), headings.to_vec()));
        let mut display = std::borrow::Cow::Borrowed(headings);
        if numbered {
            for (heading, prefix) in display
                .to_mut()
                .iter_mut()
                .zip(inkstone_core::heading_numbers::prefixes(headings))
            {
                heading.title = format!("{prefix} {}", heading.title);
            }
        }
        let rows = Rc::new(outline_rows(&display, collapsed, query));
        self.projection = Some(OutlineProjection {
            key: key.clone(),
            numbered,
            query: query.to_owned(),
            collapsed: collapsed.clone(),
            rows: rows.clone(),
        });
        (key, rows)
    }
}

impl Workspace {
    fn right_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        // Use the actual remaining title-bar width, including during a resize.
        // Window controls and the drag spacer have already claimed their space.
        let header = gpui::canvas(
            move |bounds, window, cx| {
                let mut header = weak
                    .update(cx, |this, cx| {
                        this.right_header_tools(bounds.size.width, !cfg!(target_os = "windows"), cx)
                    })
                    .unwrap_or_else(|_| div().into_any_element());
                header.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                header
            },
            |_, mut header, window, cx| header.paint(window, cx),
        )
        .w_full()
        .h(px(40.));
        if cfg!(target_os = "windows") {
            self.windows_workspace_header(
                div()
                    .flex()
                    .w_full()
                    // Keep the space around the responsive toolbar draggable.
                    // Tool buttons occlude this area to keep their normal click behavior.
                    .window_control_area(gpui::WindowControlArea::Drag)
                    .child(header),
                8.,
            )
        } else {
            self.workspace_header(div().flex().child(header), false, false)
        }
    }
    fn right_header_tools(
        &self,
        width: Pixels,
        show_toggle: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const MODES: [(usize, &str, &str); 4] = [
            (0, "list", "大纲"),
            (1, "links", "链接"),
            (4, "inbox", "属性"),
            (5, "sparkles", "Agent"),
        ];
        const BUTTON_WIDTH: f32 = 32.;
        const PADDING: f32 = 6.;
        const GAP: f32 = 2.;
        // The toggle adds a spacer as well as a button, so reserve both gaps.
        let toggle_width = if show_toggle {
            BUTTON_WIDTH + 2. * GAP
        } else {
            0.
        };
        let slots = ((f32::from(width) - 2. * PADDING - toggle_width + GAP).max(0.)
            / (BUTTON_WIDTH + GAP)) as usize;
        let collapsed = slots < MODES.len();
        let visible_count = if collapsed {
            slots.saturating_sub(1)
        } else {
            MODES.len()
        };
        // Retain the active view first when space is limited.
        let visible: Vec<_> = if collapsed {
            MODES
                .iter()
                .map(|&(i, _, _)| i)
                .filter(|&i| i == self.ui.right_mode)
                .chain(
                    MODES
                        .iter()
                        .map(|&(i, _, _)| i)
                        .filter(|&i| i != self.ui.right_mode),
                )
                .take(visible_count)
                .collect()
        } else {
            MODES.iter().map(|&(i, _, _)| i).collect()
        };
        let weak = cx.entity().downgrade();
        let selected = self.ui.right_mode;
        let colors = crate::theme::palette(self.ui.prefs.light);
        div()
            .id("right-header-tools")
            .debug_selector(|| "right-header-tools".into())
            .w(width)
            .flex()
            .items_center()
            .h(px(40.))
            .when(cfg!(target_os = "windows"), |bar| {
                bar.window_control_area(gpui::WindowControlArea::Drag)
            })
            .px(px(PADDING))
            .gap(px(GAP))
            .children(visible.iter().map(|&mode| {
                let (i, ico, label) = MODES.iter().copied().find(|&(i, _, _)| i == mode).unwrap();
                let is_active = self.ui.right_mode == i;
                div()
                    .debug_selector(move || format!("right-mode-{i}"))
                    .flex_shrink_0()
                    .child(
                        Button::new(("right-mode", i))
                            .occlude()
                            .accessibility_id(format!("right-mode-{i}"))
                            .accessibility_label(label)
                            .selected(is_active)
                            .toggled(is_active)
                            .ghost()
                            .compact()
                            .rounded(px(7.))
                            .when(is_active, |b| {
                                b.bg(colors.selected).text_color(colors.accent)
                            })
                            .icon(icon(ico))
                            .tooltip(label)
                            .w(px(BUTTON_WIDTH))
                            .h(px(28.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ui.right_mode = i;
                                cx.notify();
                            })),
                    )
            }))
            .when(collapsed, |bar| {
                bar.child(
                    div()
                        .debug_selector(|| "right-mode-more".into())
                        // Shield the title-bar drag area without blocking the popover trigger.
                        .occlude()
                        .flex_shrink_0()
                        .child(
                            tool("right-mode-more", "ellipsis-vertical", "更多侧栏功能")
                                .flex_shrink_0()
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (i, ico, label) in MODES {
                                        if visible.contains(&i) {
                                            continue;
                                        }
                                        let weak = weak.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .icon(icon(ico))
                                                .checked(selected == i)
                                                .on_click(move |_, _, cx| {
                                                    let _ = weak.update(cx, |this, cx| {
                                                        this.ui.right_mode = i;
                                                        cx.notify();
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }),
                        ),
                )
            })
            .when(show_toggle, |bar| {
                bar.child(div().flex_1().min_w_0())
                    .child(self.sidebar_toggle(true, true, cx))
            })
            .into_any_element()
    }
    pub(super) fn right_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if matches!(self.ui.right_mode, 5 | 6) {
            return div()
                .id("workspace-right-panel")
                .flex()
                .flex_col()
                .size_full()
                .bg(self.bg())
                .child(self.right_header(cx))
                .child(if self.ui.right_mode == 6 {
                    self.background_task_panel(cx)
                } else {
                    self.agent_panel(cx)
                })
                .into_any_element();
        }
        let pane = self.current_pane();
        let outline_query = if self.ui.right_mode == 0 && self.ui.outline_filter_open {
            self.ui
                .outline_filter
                .read(cx)
                .value()
                .trim()
                .to_lowercase()
        } else {
            String::new()
        };
        let (fold_key, headings) = if self.ui.right_mode == 0 {
            if let Some(pane) = &pane {
                let pane = pane.read(cx);
                self.ui.outline.project(
                    &pane.current_path,
                    &pane.parsed.headings,
                    &outline_query,
                    pane.heading_numbering,
                )
            } else {
                self.ui
                    .outline
                    .project(std::path::Path::new(""), &[], &outline_query, false)
            }
        } else {
            (Rc::new((PathBuf::new(), Vec::new())), Rc::new(Vec::new()))
        };
        let links = pane
            .as_ref()
            .filter(|_| self.ui.right_mode == 1)
            .map(|p| p.read(cx).parsed.links.clone())
            .unwrap_or_default();
        let attachment = self
            .active
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.attachment_properties.as_ref());
        let property_error = attachment.and_then(|a| a.source.as_ref().err()).cloned();
        let can_edit_properties =
            pane.is_some() || attachment.is_some() && property_error.is_none();
        let properties = pane
            .as_ref()
            .filter(|_| self.ui.right_mode == 4)
            .map(|p| p.update(cx, |pane, cx| pane.properties(cx).to_vec()))
            .or_else(|| {
                attachment
                    .filter(|_| self.ui.right_mode == 4)?
                    .text()
                    .map(inkstone_core::properties::parse)
            })
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
                    .when(self.ui.right_mode == 0, |s| {
                        s.flex().flex_col().overflow_hidden()
                    })
                    .p_3()
                    // Center the 28px outline tools on the adjacent 40px view header.
                    .when(self.ui.right_mode == 0, |s| s.pt(px(6.)))
                    .when(self.ui.right_mode == 0, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(28.))
                                .flex_shrink_0()
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
                                    .flex_shrink_0()
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
                        .child(
                            uniform_list(
                                "outline-rows",
                                headings.len(),
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    let colors = crate::theme::palette(this.ui.prefs.light);
                                    range
                                        .map(|i| {
                                            let (h, has_children, folded) = headings[i].clone();
                                            let pane = pane.clone();
                                            let key = fold_key.clone();
                                            let offset = h.offset;
                                            div()
                                                .id(("outline", i))
                                                .debug_selector(move || format!("outline-row-{i}"))
                                                .h(px(28.))
                                                .text_size(px(MIN_UI_FONT_SIZE))
                                                .rounded(px(6.))
                                                .hover(|s| s.bg(colors.hover))
                                                .flex()
                                                .items_center()
                                                .gap_1()
                                                .pl(px((h.level - 1) as f32 * 13.))
                                                .cursor_pointer()
                                                .child(div().w(px(28.)).flex_shrink_0().when(
                                                    has_children,
                                                    |s| {
                                                        s.child(
                                                            Button::new(("outline-fold", offset))
                                                                .ghost()
                                                                .compact()
                                                                .w(px(28.))
                                                                .h(px(24.))
                                                                .icon(icon(if folded {
                                                                    "chevron-right"
                                                                } else {
                                                                    "chevron-down"
                                                                }))
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
                                                .child(
                                                    Button::new(("outline-jump", offset))
                                                        .ghost()
                                                        .compact()
                                                        .h(px(28.))
                                                        .flex_1()
                                                        .min_w_0()
                                                        .justify_start()
                                                        .accessibility_label(format!(
                                                            "跳转到 {} 级标题 {}",
                                                            h.level, h.title
                                                        ))
                                                        .child(
                                                            div()
                                                                .truncate()
                                                                .when(h.level <= 2, |row| {
                                                                    row.font_weight(
                                                                        FontWeight::MEDIUM,
                                                                    )
                                                                    .text_color(colors.foreground)
                                                                })
                                                                .when(h.level > 2, |row| {
                                                                    row.text_color(colors.muted)
                                                                })
                                                                .child(h.title),
                                                        )
                                                        .on_click(cx.listener(
                                                            move |_, _, w, cx| {
                                                                if let Some(pane) = &pane {
                                                                    pane.update(cx, |p, cx| {
                                                                        p.jump(offset, w, cx)
                                                                    });
                                                                }
                                                            },
                                                        )),
                                                )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.ui.outline.scroll)
                            .flex_1()
                            .min_h_0()
                            .w_full(),
                        )
                    })
                    .when(self.ui.right_mode == 1, |s| {
                        let colors = crate::theme::palette(self.ui.prefs.light);
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(34.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child("链接当前文件")
                                .child(
                                    div()
                                        .px_2()
                                        .py(px(1.))
                                        .rounded_full()
                                        .bg(colors.surface)
                                        .border_1()
                                        .border_color(colors.border)
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(self.backlinks.len().to_string()),
                                ),
                        )
                        .when(self.backlinks.is_empty(), |s| {
                            s.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .text_color(colors.muted)
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
                                                    .h(px(28.))
                                                    .px_2()
                                                    .text_size(px(MIN_UI_FONT_SIZE))
                                                    .rounded(px(6.))
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
                            .h(px((self.backlinks.len() as f32 * 28.).min(400.)))
                            .w_full(),
                        )
                    })
                    .when(self.ui.right_mode == 1, |s| {
                        let colors = crate::theme::palette(self.ui.prefs.light);
                        s.child(
                            div()
                                .mt_3()
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .h(px(34.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child("当前笔记中的链接")
                                .child(
                                    div()
                                        .px_2()
                                        .py(px(1.))
                                        .rounded_full()
                                        .bg(colors.surface)
                                        .border_1()
                                        .border_color(colors.border)
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(links.len().to_string()),
                                ),
                        )
                        .children(links.into_iter().enumerate().map(|(i, link)| {
                            let from = tab_path.clone();
                            div()
                                .id(("outlink", i))
                                .h(px(28.))
                                .px_2()
                                .text_size(px(MIN_UI_FONT_SIZE))
                                .rounded(px(6.))
                                .hover(move |s| s.bg(colors.hover))
                                .py_1()
                                .text_color(colors.accent)
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
                        s.children(property_error)
                            .children(properties.iter().enumerate().map(|(i, p)| {
                                let name = p.name.clone();
                                let value = p.value.clone();
                                let kind = self
                                    .ui
                                    .prefs
                                    .property_types
                                    .get(&name.to_lowercase())
                                    .copied()
                                    .unwrap_or_else(|| {
                                        inkstone_core::properties::Kind::infer(&value)
                                    });
                                let symbol = match kind {
                                    inkstone_core::properties::Kind::Number => "hash",
                                    inkstone_core::properties::Kind::Checkbox => "check-square",
                                    inkstone_core::properties::Kind::Date
                                    | inkstone_core::properties::Kind::DateTime => "calendar",
                                    inkstone_core::properties::Kind::List => "list",
                                    _ => "text",
                                };
                                let display =
                                    match serde_json::from_str::<serde_json::Value>(&value) {
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
                                let colors = crate::theme::palette(self.ui.prefs.light);
                                div()
                                    .id(("property", i))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_h(px(34.))
                                    .px_2p5()
                                    .py_1()
                                    .rounded(px(6.))
                                    .text_size(px(MIN_UI_FONT_SIZE))
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(colors.hover))
                                    .child(icon(symbol).size(px(15.)).text_color(colors.muted))
                                    .child(
                                        div()
                                            .w(px(100.))
                                            .flex_shrink_0()
                                            .truncate()
                                            .text_color(colors.muted)
                                            .child(name.clone()),
                                    )
                                    .child(div().flex_1().min_w_0().truncate().child(display))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.edit_property(&name, &value, w, cx)
                                    }))
                            }))
                            .when(can_edit_properties, |s| {
                                s.child(
                                    Button::new("new-property")
                                        .ghost()
                                        .w_full()
                                        .justify_start()
                                        .h(px(34.))
                                        .accessibility_label("添加文件属性")
                                        .child(
                                            div()
                                                .w_full()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .text_size(px(MIN_UI_FONT_SIZE))
                                                .text_color(
                                                    crate::theme::palette(self.ui.prefs.light)
                                                        .muted,
                                                )
                                                .child(icon("plus"))
                                                .child("添加文件属性"),
                                        )
                                        .on_click(cx.listener(|this, _, w, cx| {
                                            this.edit_property("", "", w, cx)
                                        })),
                                )
                            })
                    }),
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
        key: OutlineKey,
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
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn right_header_collapses_without_overlap_and_restores_on_resize(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.ui.prefs.right_open = true;
                w.ui.right_mode = 4;
                w.add_tab(
                    "header.md".into(),
                    Some("# Header".into()),
                    false,
                    window,
                    cx,
                );
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        for width in [208., 215., 216., 250., 300., 345., 500., 208.] {
            handle
                .update(&mut visual, |w, _, cx| {
                    w.ui.prefs.right_width = width;
                    cx.notify();
                })
                .unwrap();
            for _ in 0..2 {
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            let toolbar = visual.debug_bounds("right-header-tools").unwrap();
            assert!(visual.debug_bounds("right-mode-2").is_none());
            assert!(visual.debug_bounds("right-mode-3").is_none());
            assert!(visual.debug_bounds("right-mode-6").is_none());
            let mut buttons: Vec<_> = [
                "right-mode-0",
                "right-mode-1",
                "right-mode-4",
                "right-mode-5",
            ]
            .into_iter()
            .filter_map(|selector| visual.debug_bounds(selector))
            .collect();
            let more = visual.debug_bounds("right-mode-more");
            assert_eq!(more.is_some(), buttons.len() < 4);
            if !buttons.is_empty() {
                assert!(visual.debug_bounds("right-mode-4").is_some());
            }
            buttons.extend(more);
            buttons.sort_by(|a, b| a.left().partial_cmp(&b.left()).unwrap());
            for button in &buttons {
                assert!(button.left() >= toolbar.left());
                assert!(
                    button.right() <= toolbar.right(),
                    "button {button:?} overflows toolbar {toolbar:?} at sidebar width {width}"
                );
                assert_eq!(button.size.width, px(32.));
            }
            for pair in buttons.windows(2) {
                assert!(pair[0].right() <= pair[1].left());
            }
            if width == 500. {
                assert_eq!(buttons.len(), 4);
                assert!(more.is_none());
            }
        }
    }

    #[gpui::test]
    fn right_header_controls_switch_view_with_or_without_overflow(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.ui.prefs.right_open = true;
                w.ui.prefs.right_width = 208.;
                w.ui.right_mode = 4;
                w.add_tab(
                    "header.md".into(),
                    Some("# Header".into()),
                    false,
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        for _ in 0..2 {
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        let more = visual.debug_bounds("right-mode-more");
        let Some(more) = more else {
            // If every mode fits, verify direct activation instead.
            for selector in [
                "right-mode-0",
                "right-mode-1",
                "right-mode-4",
                "right-mode-5",
            ] {
                assert!(visual.debug_bounds(selector).is_some());
            }
            let outline = visual.debug_bounds("right-mode-0").unwrap();
            visual.simulate_click(outline.center(), Modifiers::default());
            visual.run_until_parked();
            handle
                .update(&mut visual, |w, _, _| assert_eq!(w.ui.right_mode, 0))
                .unwrap();
            return;
        };
        let first_hidden = [
            (0, "right-mode-0"),
            (1, "right-mode-1"),
            (4, "right-mode-4"),
            (5, "right-mode-5"),
        ]
        .into_iter()
        .find(|(_, selector)| visual.debug_bounds(selector).is_none())
        .unwrap()
        .0;
        visual.simulate_click(more.center(), Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                gpui_base::GlobalState::is_in_deferred_context(cx),
                "more menu should open on click"
            );
        });
        for key in ["down", "enter"] {
            let keystroke = Keystroke::parse(key).unwrap();
            visual.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(KeyUpEvent { keystroke });
        }
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.ui.right_mode, first_hidden)
            })
            .unwrap();
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.prefs.right_width = 500.;
                cx.notify();
            })
            .unwrap();
        for _ in 0..2 {
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert!(visual.debug_bounds("right-mode-more").is_none());
        let outline = visual.debug_bounds("right-mode-0").unwrap();
        visual.simulate_click(outline.center(), Modifiers::default());
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.ui.right_mode, 0))
            .unwrap();
    }

    #[gpui::test]
    fn large_outline_virtualizes_rows_and_reconciles_scroll_after_filter_and_fold(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = format!(
            "# 总览\n\n{}",
            (0..500)
                .map(|i| format!("## 标题{i:03}\n\n正文\n\n"))
                .collect::<String>()
        );
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.ui.prefs.right_open = true;
                w.ui.right_mode = 0;
                w.add_tab("outline.md".into(), Some(source.clone()), false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        fn draw(v: &mut VisualTestContext) {
            for _ in 0..2 {
                v.update(|window, cx| window.draw(cx).clear(cx));
            }
        }
        draw(&mut visual);
        assert!(visual.debug_bounds("outline-row-0").is_some());
        assert!(visual.debug_bounds("outline-row-100").is_none());
        assert!(visual.debug_bounds("outline-row-500").is_none());
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.outline
                    .scroll
                    .scroll_to_item(500, ScrollStrategy::Bottom);
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        assert!(visual.debug_bounds("outline-row-500").is_some());
        assert!(visual.debug_bounds("outline-row-1").is_none());
        let last = visual.debug_bounds("outline-row-500").unwrap();
        visual.simulate_click(last.center(), Modifiers::default());
        draw(&mut visual);
        let offset = source.find("## 标题499").unwrap();
        handle
            .update(&mut visual, |w, _, cx| {
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).selected_range(),
                    offset..offset
                );
            })
            .unwrap();
        handle
            .update(&mut visual, |w, window, cx| {
                w.ui.outline_filter_open = true;
                w.ui.outline_filter
                    .update(cx, |s, cx| s.set_value("标题499", window, cx));
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        assert!(visual.debug_bounds("outline-row-0").is_some());
        assert!(visual.debug_bounds("outline-row-1").is_none());
        for activation in ["enter", "space"] {
            handle
                .update(&mut visual, |w, window, cx| {
                    let editor = w.tabs[0].pane.read(cx).editor.clone();
                    editor.update(cx, |s, cx| s.set_selected_range(0..0, cx));
                    w.ui.outline_filter.update(cx, |s, cx| s.focus(window, cx));
                })
                .unwrap();
            draw(&mut visual);
            for key in ["tab", activation] {
                let keystroke = Keystroke::parse(key).unwrap();
                visual.simulate_event(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                });
                visual.simulate_event(KeyUpEvent { keystroke });
                draw(&mut visual);
            }
            handle
                .update(&mut visual, |w, window, cx| {
                    let editor = w.tabs[0].pane.read(cx).editor.read(cx);
                    assert_eq!(editor.selected_range(), offset..offset);
                    assert!(editor.focus_handle(cx).is_focused(window));
                    assert_eq!(editor.value().as_ref(), source);
                    assert_eq!(w.ui.outline_filter.read(cx).value().as_ref(), "标题499");
                })
                .unwrap();
        }
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.outline_filter_open = false;
                let pane = w.tabs[0].pane.read(cx);
                let key = Rc::new((pane.current_path.clone(), pane.parsed.headings.clone()));
                w.toggle_outline_fold(key, Some(0), cx);
            })
            .unwrap();
        draw(&mut visual);
        assert!(visual.debug_bounds("outline-row-0").is_some());
        assert!(visual.debug_bounds("outline-row-1").is_none());
        handle
            .update(&mut visual, |w, _, cx| {
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                    source
                );
            })
            .unwrap();
    }

    #[test]
    fn outline_projection_reuses_rows_and_invalidates_all_visible_inputs() {
        let mut state = OutlineState::default();
        let path = std::path::Path::new("one.md");
        let mut headings = vec![
            inkstone_core::index::Heading {
                level: 1,
                offset: 0,
                title: "Parent".into(),
            },
            inkstone_core::index::Heading {
                level: 2,
                offset: 12,
                title: "Child".into(),
            },
        ];
        let (key, rows) = state.project(path, &headings, "", false);
        for _ in 0..10 {
            let (same_key, same_rows) = state.project(path, &headings, "", false);
            assert!(Rc::ptr_eq(&key, &same_key));
            assert!(Rc::ptr_eq(&rows, &same_rows));
        }
        state.key = Some(key);
        state.collapsed.insert(0);
        let (_, folded) = state.project(path, &headings, "", false);
        assert_eq!(folded.len(), 1);
        assert!(folded[0].2);
        let (_, filtered) = state.project(path, &headings, "child", false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0.offset, 12);
        let (_, numbered) = state.project(path, &headings, "1.1", true);
        assert_eq!(numbered.len(), 1);
        assert_eq!(numbered[0].0.title, "1.1 Child");
        let (_, cached) = state.project(path, &headings, "1.1", true);
        assert!(Rc::ptr_eq(&cached, &numbered));
        let (_, unnumbered) = state.project(path, &headings, "1.1", false);
        assert!(unnumbered.is_empty());
        let (_, restored) = state.project(path, &headings, "", false);
        assert_eq!(*restored, *folded);
        // A different note must not inherit the old note's folded parent.
        let (_, other) = state.project(std::path::Path::new("two.md"), &headings, "", false);
        assert_eq!(other.len(), 2);
        // Same-count and same-length title edits must invalidate cached rows.
        headings[1].title = "Other".into();
        headings[1].offset = 14;
        let (_, edited) = state.project(path, &headings, "", false);
        assert_eq!(edited.len(), 2);
        assert_eq!(edited[1].0.title, "Other");
        assert_eq!(edited[1].0.offset, 14);
        headings[1].level = 1;
        let (_, siblings) = state.project(path, &headings, "", false);
        assert!(!siblings[0].1);
        let (_, empty) = state.project(path, &[], "", false);
        assert!(empty.is_empty());
    }

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
