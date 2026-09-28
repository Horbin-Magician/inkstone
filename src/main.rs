use gpui::{prelude::*, *};
use gpui_component::{
    Root,
    button::*,
    input::{Editor, EditorState, TextDecoration, TextDecorationCollection},
};
use inkstone::markdown::{self, Kind};

const SAMPLE: &str = "# 砚台 · 编辑器验证\n\n在这里测试微软拼音：中文 English 混排，😀 👩‍💻。\n\n**粗体文本** 和 `inline code`，以及 [[欢迎]]。\n\n光标进入语法范围会显示标记，离开时隐藏标记并保留占位。\n\n- Ctrl+A / C / X / V：选择与剪贴板\n- Ctrl+Z / Ctrl+Y：撤销与重做\n- Ctrl+F：文档查找\n\n请测试跨行选择、软换行、中文组词提交及取消。\n";

struct Prototype {
    editor: Entity<EditorState>,
    decorations: TextDecorationCollection,
    live: bool,
    parse_source: SharedString,
    parse_revision: u64,
    spans: Vec<markdown::Span>,
    parse_task: Option<Task<()>>,
    last_presentation: Option<(SharedString, std::ops::Range<usize>, bool)>,
    _subscription: Subscription,
}

impl Prototype {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(SAMPLE)
                .line_number(true)
        });
        let decorations = editor.update(cx, |state, cx| {
            state.create_decorations_collection(vec![], cx)
        });
        let subscription = cx.observe(&editor, |_, _, cx| cx.notify());
        editor.update(cx, |state, cx| state.focus(window, cx));
        Self {
            editor,
            decorations,
            live: true,
            parse_source: "".into(),
            parse_revision: 0,
            spans: vec![],
            parse_task: None,
            last_presentation: None,
            _subscription: subscription,
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
            let source = text.clone();
            let task = cx
                .background_executor()
                .spawn(async move { markdown::spans(&source) });
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let spans = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.parse_revision != revision {
                        return;
                    }
                    this.spans = spans;
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

impl Render for Prototype {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_presentation(cx);
        let state = self.editor.read(cx);
        let range = state.selected_range();
        let status = format!(
            "UTF-8 选区 {}..{} · {} 字节 · 原型阶段，尚未接入笔记库",
            range.start,
            range.end,
            state.value().len()
        );
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
                    .gap_4()
                    .child("砚台 / Inkstone")
                    .child(
                        Button::new("toggle-live")
                            .label(if self.live {
                                "切换源码"
                            } else {
                                "限定实时样式"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.live = !this.live;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9babc0))
                    .child("编辑器原型 · 标记隐藏保留占位 · 表格、图片与复杂嵌套保留源码"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(&self.editor).h_full().text_size(px(18.))),
            )
            .child(div().text_sm().child(status))
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_component::set_locale("zh-CN");
            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(80.), px(60.)),
                            size(px(1100.), px(760.)),
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("砚台 · 编辑器原型".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| Prototype::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("无法创建 GPUI 窗口");
            })
            .detach();
        });
}
