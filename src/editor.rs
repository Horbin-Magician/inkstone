use crate::editor_links::{self, ParsedCache, PathCache, WikiCompletions, WikiDefinitions};
use gpui::{prelude::*, *};
use gpui_base::text::{TextView, TextViewState};
use gpui_component::{
    button::*,
    input::{Editor, EditorState, TextDecoration, TextDecorationCollection},
};
use inkstone::index::{self, ParsedNote};
use inkstone::markdown::{self, Kind};
use std::path::PathBuf;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub enum EditorEvent {
    FollowLink(String),
    FollowMarkdownLink(String),
}
impl EventEmitter<EditorEvent> for EditorPane {}

pub struct EditorPane {
    pub editor: Entity<EditorState>,
    decorations: TextDecorationCollection,
    live: bool,
    parse_source: SharedString,
    parse_revision: u64,
    spans: Vec<markdown::Span>,
    parse_task: Option<Task<()>>,
    parsed: ParsedNote,
    reading: bool,
    preview: Entity<TextViewState>,
    pub image_dir: PathBuf,
    link_cache: ParsedCache,
    path_cache: PathCache,
    last_presentation: Option<(SharedString, std::ops::Range<usize>, bool)>,
    _subscription: Subscription,
    last_geometry: Option<(Size<Pixels>, bool)>,
}

impl EditorPane {
    pub fn focus_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            let handle = self.preview.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }
    pub fn set_paths(&mut self, paths: Arc<Vec<String>>) {
        *self.path_cache.borrow_mut() = paths;
    }
    pub fn jump(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.reading = false;
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(text)
                .line_number(true)
        });
        let decorations = editor.update(cx, |state, cx| {
            state.create_decorations_collection(vec![], cx)
        });
        let subscription = cx.observe_in(&editor, window, |this, editor, window, cx| {
            let state = editor.read(cx);
            let visible = state.cursor_layout().is_some_and(|(mut caret, _)| {
                caret.origin += state.scroll_offset();
                state.input_bounds().intersects(&caret)
            });
            let size = state.input_bounds().size;
            let previous = this.last_geometry.replace((size, visible));
            if !this.reading
                && state.focus_handle(cx).is_focused(window)
                && previous.is_some_and(|(old, was_visible)| old != size && was_visible)
            {
                // Geometry notifications arrive after paint, so the new map is ready.
                cx.on_next_frame(window, |this, window, cx| {
                    if !this.reading && this.editor.read(cx).focus_handle(cx).is_focused(window) {
                        this.editor.update(cx, |state, cx| state.reveal_cursor(cx));
                    }
                });
            }
            cx.notify();
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));
        let link_cache = Rc::new(RefCell::new((
            SharedString::default(),
            ParsedNote::default(),
        )));
        let path_cache = Rc::new(RefCell::new(Arc::new(vec![])));
        let weak = cx.entity().downgrade();
        editor.update(cx, |state, _| {
            state.lsp_mut().definition_provider =
                Some(Rc::new(WikiDefinitions(link_cache.clone())));
            state.lsp_mut().completion_provider = Some(Rc::new(WikiCompletions(
                path_cache.clone(),
                Some(editor.downgrade()),
            )));
            state.lsp_mut().show_document = Some(Rc::new(move |params, _, cx| {
                let Some(target) = editor_links::decode_uri(&params.uri.to_string()) else {
                    return false;
                };
                let _ = weak.update(cx, |_, cx| cx.emit(EditorEvent::FollowLink(target)));
                true
            }));
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        Self {
            editor,
            decorations,
            live: true,
            parse_source: "".into(),
            parse_revision: 0,
            spans: vec![],
            parse_task: None,
            parsed: ParsedNote::default(),
            reading: false,
            preview,
            image_dir: PathBuf::new(),
            link_cache,
            path_cache,
            last_presentation: None,
            _subscription: subscription,
            last_geometry: None,
        }
    }

    fn update_presentation(&mut self, cx: &mut Context<Self>) {
        let state = self.editor.read(cx);
        let text = state.value();
        let selection = state.selected_range();
        if self.parse_source != text {
            self.parse_source = text.clone();
            self.parse_revision += 1;
            let revision = self.parse_revision;
            self.spans.clear();
            self.parsed = ParsedNote::default();
            *self.link_cache.borrow_mut() = (SharedString::default(), ParsedNote::default());
            let source = text.clone();
            let task = cx.background_executor().spawn(async move {
                let parsed = index::parse(&source);
                let reading = index::reading_source(&source, &parsed);
                (markdown::spans(&source), parsed, reading)
            });
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let (spans, parsed, reading) = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.parse_revision != revision
                        || this.editor.read(cx).value() != this.parse_source
                    {
                        return;
                    }
                    this.spans = spans;
                    this.parsed = parsed;
                    *this.link_cache.borrow_mut() =
                        (this.parse_source.clone(), this.parsed.clone());
                    this.preview
                        .update(cx, |state, cx| state.set_text(&reading, cx));
                    this.last_presentation = None;
                    cx.notify();
                });
            }));
        }
        let key = (text.clone(), selection.clone(), self.live);
        if self.last_presentation.as_ref() == Some(&key) {
            return;
        }
        self.last_presentation = Some(key);
        let mut decorations = Vec::new();
        if self.live {
            for span in &self.spans {
                let style = match span.kind {
                    Kind::Heading => HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        color: Some(rgb(0x77bdfb).into()),
                        ..Default::default()
                    },
                    Kind::Strong => HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    },
                    Kind::Code => HighlightStyle {
                        color: Some(rgb(0xf5c77e).into()),
                        background_color: Some(rgb(0x283446).into()),
                        ..Default::default()
                    },
                    Kind::WikiLink => HighlightStyle {
                        color: Some(rgb(0x80d8bd).into()),
                        ..Default::default()
                    },
                };
                decorations.push(TextDecoration::new(span.content.clone(), style));
                if !span.active(&selection) {
                    for marker in &span.markers {
                        decorations.push(TextDecoration::new(
                            marker.clone(),
                            HighlightStyle {
                                fade_out: Some(1.0),
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
        }
        self.decorations.set(decorations, cx);
    }
}

impl Render for EditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "metrics")]
        if crate::metrics::needs_ready() {
            cx.on_next_frame(_window, |_, _, _| crate::metrics::record_ready());
        }
        self.update_presentation(cx);
        let state = self.editor.read(cx);
        let range = state.selected_range();
        let status = format!(
            "UTF-8 选区 {}..{} · {} 字节",
            range.start,
            range.end,
            state.value().len()
        );
        let image_dir = self.image_dir.clone();
        let weak = cx.entity().downgrade();
        let preview = TextView::new(&self.preview)
            .style(
                gpui_base::text::TextViewStyle::from_theme(&gpui_base::Theme::global(cx))
                    .with_table_head(
                        StyleRefinement::default()
                            .bg(rgb(0x283446))
                            .text_color(rgb(0xe6edf6)),
                    ),
            )
            .scrollable(true)
            .selectable(true)
            .image_source(move |uri| {
                let raw = uri.to_string();
                if raw.starts_with("http://")
                    || raw.starts_with("https://")
                    || raw.starts_with("data:")
                {
                    uri.clone().into()
                } else {
                    image_dir
                        .join(
                            percent_encoding::percent_decode_str(raw.trim_start_matches("file://"))
                                .decode_utf8_lossy()
                                .as_ref(),
                        )
                        .into()
                }
            })
            .on_link_click(move |url, _, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if let Some(index) = url
                        .strip_prefix("inkstone-link:")
                        .and_then(|i| i.parse::<usize>().ok())
                    {
                        if let Some(link) = this.parsed.links.get(index) {
                            cx.emit(EditorEvent::FollowLink(link.target.clone()));
                        }
                    } else if url.starts_with("https://") || url.starts_with("http://") {
                        cx.open_url(url);
                    } else {
                        cx.emit(EditorEvent::FollowMarkdownLink(url.to_string()));
                    }
                });
            });
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x151c28))
            .text_color(rgb(0xe6edf6))
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child("Markdown")
                    .child(
                        Button::new("reading")
                            .label(if self.reading {
                                "返回编辑"
                            } else {
                                "阅读预览"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reading = !this.reading;
                                this.focus_view(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("toggle-live")
                            .label(if self.live {
                                "切换源码"
                            } else {
                                "限定实时样式"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.live = !this.live;
                                this.reading = false;
                                this.editor.update(cx, |state, cx| state.focus(window, cx));
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9babc0))
                    .child("标记隐藏保留占位 · 复杂语法可切源码 · 阅读视图支持完整块结构"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(self.reading, |s| s.child(preview))
                            .when(!self.reading, |s| {
                                s.child(Editor::new(&self.editor).h_full().text_size(px(18.)))
                            }),
                    )
                    .child(
                        div()
                            .id("outline")
                            .w(px(170.))
                            .overflow_y_scroll()
                            .p_2()
                            .child(div().text_sm().child("大纲"))
                            .children(self.parsed.headings.iter().enumerate().map(|(i, h)| {
                                let offset = h.offset;
                                div()
                                    .id(("heading", i))
                                    .p_1()
                                    .cursor_pointer()
                                    .child(format!(
                                        "{}{}",
                                        "  ".repeat((h.level - 1) as usize),
                                        h.title
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.reading = false;
                                        this.editor.update(cx, |state, cx| {
                                            state.set_selected_range(offset..offset, cx);
                                            state.focus(window, cx);
                                        });
                                        cx.notify();
                                    }))
                            }))
                            .child(div().mt_4().text_sm().child("出站双链"))
                            .children(self.parsed.links.iter().enumerate().map(|(i, link)| {
                                let target = link.target.clone();
                                div()
                                    .id(("outlink", i))
                                    .p_1()
                                    .cursor_pointer()
                                    .text_color(rgb(0x80d8bd))
                                    .child(link.label.clone())
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(EditorEvent::FollowLink(target.clone()))
                                    }))
                            })),
                    ),
            )
            .child(div().text_sm().child(status))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn ctrl_hover_click_emits_wiki_target_without_changing_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "中文😀 [[目录/笔记|标签]]";
        let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
        let targets = Rc::new(RefCell::new(Vec::new()));
        let captured = targets.clone();
        let _subscription = handle
            .update(cx, |pane, _, cx| {
                pane.editor.update(cx, |state, cx| {
                    let offset = "中文😀 [[目".len();
                    state.set_selected_range(offset..offset, cx);
                });
                cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                    if let EditorEvent::FollowLink(target) = event {
                        captured.borrow_mut().push(target.clone());
                    }
                })
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let position = handle
            .update(&mut visual, |pane, _, cx| {
                let state = pane.editor.read(cx);
                let (caret, _) = state.cursor_layout().unwrap();
                caret.origin + state.scroll_offset() + point(px(1.), px(5.))
            })
            .unwrap();
        let modifiers = Modifiers {
            control: true,
            ..Default::default()
        };
        visual.simulate_mouse_move(position, None, modifiers);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_click(position, modifiers);
        assert_eq!(targets.borrow().as_slice(), &["目录/笔记"]);
        handle
            .update(&mut visual, |pane, _, cx| {
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn resize_keeps_visible_caret_and_selection_direction(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "中文 English 👩‍💻 e\u{301}".repeat(500);
        let handle = cx.add_window(|window, cx| EditorPane::new(&source, window, cx));
        let editor = handle.update(cx, |pane, _, _| pane.editor.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-end shift-left");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let before = editor.read_with(&visual, |state, _| (state.selected_range(), state.cursor()));
        for width in [1900., 900.] {
            visual.simulate_resize(size(px(width), px(800.)));
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            editor.read_with(&visual, |state, _| {
                assert_eq!((state.selected_range(), state.cursor()), before);
                let (mut caret, _) = state.cursor_layout().expect("caret should be laid out");
                caret.origin += state.scroll_offset();
                assert!(state.input_bounds().intersects(&caret),
                    "caret should remain visible at width {width}: caret={caret:?}, input={:?}, scroll={:?}",state.input_bounds(),state.scroll_offset());
                assert_eq!(state.value().as_ref(), source);
            });
        }
    }

    #[gpui::test]
    fn keyboard_selection_and_delete_preserve_whole_graphemes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "A👩‍💻e\u{301}";
        let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
        let editor = handle
            .update(cx, |pane, _, cx| {
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(source.len()..source.len(), cx)
                });
                pane.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_keystrokes("shift-left");
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.selected_range(), 12..15)
        });
        visual.simulate_keystrokes("shift-left");
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.selected_range(), 1..15)
        });
        visual.simulate_keystrokes("backspace");
        editor.read_with(&visual, |state, _| assert_eq!(state.value().as_ref(), "A"));
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.value().as_ref(), source)
        });
    }

    #[gpui::test]
    fn newer_parse_wins_and_reading_keeps_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle =
            cx.add_window(|window, cx| EditorPane::new("# 旧文档\n[[旧链接]]", window, cx));
        handle
            .update(cx, |pane, window, cx| {
                pane.update_presentation(cx);
                pane.editor.update(cx, |state, cx| {
                    state.set_value(
                        "# 新文档😀\n\n|列|值|\n|-|-|\n|中文|数据|\n\n[[新链接]]",
                        window,
                        cx,
                    )
                });
                pane.update_presentation(cx);
                pane.reading = true;
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |pane, _, cx| {
                assert_eq!(pane.parsed.headings[0].title, "新文档😀");
                assert_eq!(pane.parsed.links[0].target, "新链接");
                assert!(pane.editor.read(cx).value().contains("|中文|数据|"));
                assert!(pane.editor.read(cx).value().contains("[[新链接]]"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn component_ime_utf16_commit_cancel_and_source_preservation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| EditorPane::new("**中😀**\n", window, cx));
        handle
            .update(cx, |pane, window, cx| {
                let editor = pane.editor.clone();
                editor.update(cx, |state, cx| {
                    state.set_selected_range(12..12, cx);
                    state.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\nni");
                    assert_eq!(state.marked_text_range(window, cx), Some(8..10));
                    state.replace_text_in_range(None, "你", window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\n你");
                    assert_eq!(state.marked_text_range(window, cx), None);
                    state.replace_and_mark_text_in_range(None, "hao", Some(3..3), window, cx);
                    state.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
                    state.unmark_text(window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\n你");
                    let mut actual = None;
                    assert_eq!(
                        state
                            .text_for_range(3..5, &mut actual, window, cx)
                            .as_deref(),
                        Some("😀")
                    );
                    assert_eq!(actual, Some(3..5));
                });
                pane.update_presentation(cx);
                assert_eq!(pane.editor.read(cx).value().as_ref(), "**中😀**\n你");
            })
            .unwrap();
    }
}
