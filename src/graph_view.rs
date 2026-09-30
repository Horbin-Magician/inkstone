use gpui::{prelude::*, *};
use gpui_component::{
    button::*,
    input::{Input, InputEvent, InputState},
    slider::{Slider, SliderEvent, SliderState},
};
use inkstone::{
    graph::{ColorGroup, Graph, Options},
    index::Index,
};
use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

pub enum GraphEvent {
    Open(PathBuf, bool),
    Close,
    Settings(Options),
}
#[derive(Clone, Copy)]
struct GraphDrag {
    start: Point<Pixels>,
    last: Point<Pixels>,
    node: Option<usize>,
    moved: bool,
}
struct GroupInputs {
    query: Entity<InputState>,
    color: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
pub struct GraphView {
    index: Arc<Index>,
    options: Options,
    graph: Arc<Graph>,
    query: Entity<InputState>,
    _query: Subscription,
    task: Option<Task<()>>,
    revision: u64,
    loading: bool,
    light: bool,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    scale: f32,
    pan: Point<Pixels>,
    drag: Option<GraphDrag>,
    hover: Option<usize>,
    focus: FocusHandle,
    sliders: Vec<Entity<SliderState>>,
    _sliders: Vec<Subscription>,
    group_inputs: Vec<GroupInputs>,
    group_error: String,
    layout_epoch: u64,
    applied_layout_epoch: u64,
    force_task: Option<Task<()>>,
}
impl EventEmitter<GraphEvent> for GraphView {}
impl GraphView {
    fn recalculate(&mut self, cx: &mut Context<Self>) {
        self.force_task = None;
        self.layout_epoch += 1;
        self.rebuild(cx);
    }
    fn queue_layout(&mut self, cx: &mut Context<Self>) {
        self.force_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            let _ = this.update(cx, |s, cx| s.recalculate(cx));
        }));
    }
    fn reset_group_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.group_error.clear();
        self.group_inputs = self
            .options
            .groups
            .iter()
            .enumerate()
            .map(|(i, group)| {
                let query = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("path:项目 OR tag:工作")
                        .default_value(group.query.clone())
                });
                let color = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("#RRGGBB")
                        .default_value(format!("#{:06X}", group.color))
                });
                let q = cx.subscribe(&query, move |s, input, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change)
                        && let Some(g) = s.options.groups.get_mut(i)
                    {
                        g.query = input.read(cx).value().to_string();
                        s.rebuild(cx);
                    }
                });
                let c = cx.subscribe(&color, move |s, input, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change) {
                        let text = input.read(cx).value();
                        let hex = text.trim().trim_start_matches('#');
                        if hex.len() == 6
                            && let Ok(value) = u32::from_str_radix(hex, 16)
                            && let Some(g) = s.options.groups.get_mut(i)
                        {
                            g.color = value;
                            s.group_error.clear();
                            s.rebuild(cx);
                        } else {
                            s.group_error = "颜色需要六位十六进制值，如 #9775FA".into();
                            cx.notify();
                        }
                    }
                });
                GroupInputs {
                    query,
                    color,
                    _subscriptions: vec![q, c],
                }
            })
            .collect();
    }
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("graph-controls")
            .absolute()
            .top(px(12.))
            .right(px(12.))
            .w(px(if self.options.controls_open {
                240.
            } else {
                76.
            }))
            .max_h(if self.bounds.get().size.height > px(0.) {
                (self.bounds.get().size.height - px(24.)).max(px(80.))
            } else {
                px(500.)
            })
            .overflow_y_scroll()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(crate::theme::palette(self.light).border)
            .bg(crate::theme::palette(self.light).surface)
            .shadow_sm()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                Button::new("graph-controls-toggle")
                    .ghost()
                    .compact()
                    .label(if self.options.controls_open {
                        "收起设置"
                    } else {
                        "设置"
                    })
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.options.controls_open = !s.options.controls_open;
                        cx.emit(GraphEvent::Settings(s.options.clone()));
                        cx.notify();
                    })),
            )
            .when(self.options.controls_open, |s| {
                s.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .pt_2()
                        .child(div().font_weight(FontWeight::BOLD).child("筛选"))
                        .child(Input::new(&self.query))
                        .child(
                            Button::new("graph-orphans")
                                .ghost()
                                .compact()
                                .label(if self.options.orphans {
                                    "孤立笔记 ✓"
                                } else {
                                    "孤立笔记"
                                })
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.options.orphans = !s.options.orphans;
                                    s.rebuild(cx);
                                })),
                        )
                        .child(
                            Button::new("graph-missing")
                                .ghost()
                                .compact()
                                .label(if self.options.missing {
                                    "未创建笔记 ✓"
                                } else {
                                    "未创建笔记"
                                })
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.options.missing = !s.options.missing;
                                    s.rebuild(cx);
                                })),
                        )
                        .when(self.options.root.is_some(), |s| {
                            s.child(
                                Button::new("graph-depth")
                                    .ghost()
                                    .compact()
                                    .label(format!("深度 {}", self.options.depth))
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.options.depth = s.options.depth % 5 + 1;
                                        s.rebuild(cx);
                                    })),
                            )
                        })
                        .child(div().font_weight(FontWeight::BOLD).child("颜色分组"))
                        .children(self.group_inputs.iter().enumerate().map(|(i, group)| {
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(Input::new(&group.query))
                                .child(
                                    div()
                                        .flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            div()
                                                .size(px(16.))
                                                .rounded(px(3.))
                                                .bg(rgb(self.options.groups[i].color)),
                                        )
                                        .child(div().w(px(86.)).child(Input::new(&group.color)))
                                        .child(
                                            Button::new(("group-up", i))
                                                .ghost()
                                                .compact()
                                                .label("↑")
                                                .on_click(cx.listener(move |s, _, w, cx| {
                                                    if i > 0 {
                                                        s.options.groups.swap(i, i - 1);
                                                        s.reset_group_inputs(w, cx);
                                                        s.rebuild(cx);
                                                    }
                                                })),
                                        )
                                        .child(
                                            Button::new(("group-down", i))
                                                .ghost()
                                                .compact()
                                                .label("↓")
                                                .on_click(cx.listener(move |s, _, w, cx| {
                                                    if i + 1 < s.options.groups.len() {
                                                        s.options.groups.swap(i, i + 1);
                                                        s.reset_group_inputs(w, cx);
                                                        s.rebuild(cx);
                                                    }
                                                })),
                                        )
                                        .child(
                                            Button::new(("group-delete", i))
                                                .ghost()
                                                .compact()
                                                .label("×")
                                                .on_click(cx.listener(move |s, _, w, cx| {
                                                    s.options.groups.remove(i);
                                                    s.reset_group_inputs(w, cx);
                                                    s.rebuild(cx);
                                                })),
                                        ),
                                )
                        }))
                        .when(!self.group_error.is_empty(), |s| {
                            s.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(0xe4a66a))
                                    .child(self.group_error.clone()),
                            )
                        })
                        .child(
                            Button::new("graph-add-group")
                                .ghost()
                                .label("新建分组")
                                .on_click(cx.listener(|s, _, w, cx| {
                                    s.options.groups.push(ColorGroup::default());
                                    s.reset_group_inputs(w, cx);
                                    s.rebuild(cx);
                                })),
                        )
                        .child(div().font_weight(FontWeight::BOLD).child("显示"))
                        .child(
                            Button::new("graph-labels")
                                .ghost()
                                .compact()
                                .label(if self.options.labels {
                                    "显示名称 ✓"
                                } else {
                                    "显示名称"
                                })
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.options.labels = !s.options.labels;
                                    cx.emit(GraphEvent::Settings(s.options.clone()));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("graph-arrows")
                                .ghost()
                                .compact()
                                .label(if self.options.arrows {
                                    "显示箭头 ✓"
                                } else {
                                    "显示箭头"
                                })
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.options.arrows = !s.options.arrows;
                                    cx.emit(GraphEvent::Settings(s.options.clone()));
                                    cx.notify();
                                })),
                        )
                        .children(
                            ["节点大小", "连线宽度", "文字大小"]
                                .into_iter()
                                .enumerate()
                                .map(|(i, title)| {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(title)
                                        .child(Slider::new(&self.sliders[i]))
                                }),
                        )
                        .child(div().font_weight(FontWeight::BOLD).child("力"))
                        .children(
                            ["向心力", "排斥力", "连接强度", "连接距离"]
                                .into_iter()
                                .enumerate()
                                .map(|(i, title)| {
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(title)
                                        .child(Slider::new(&self.sliders[i + 3]))
                                }),
                        )
                        .child(
                            Button::new("graph-recalculate")
                                .ghost()
                                .label("重新计算布局")
                                .on_click(cx.listener(|s, _, _, cx| s.recalculate(cx))),
                        ),
                )
            })
            .into_any_element()
    }
    pub fn new(
        index: Arc<Index>,
        root: Option<PathBuf>,
        light: bool,
        mut options: Options,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        options.root = root;
        options.normalize();
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索路径、正文或 tag:标签")
                .default_value(options.query.clone())
        });
        let subscription = cx.observe(&query, |this, query, cx| {
            let text = query.read(cx).value().to_string();
            if this.options.query != text {
                this.options.query = text;
                this.rebuild(cx);
            }
        });
        let mut sliders = vec![];
        let mut subscriptions = vec![];
        for (i, (value, min, max)) in [
            (options.node_size, 0.5, 3.),
            (options.link_width, 0.25, 4.),
            (options.text_scale, 0.5, 2.),
            (options.center_force, 0., 5.),
            (options.repel_force, 0., 5.),
            (options.link_force, 0., 5.),
            (options.link_distance, 0.2, 3.),
        ]
        .into_iter()
        .enumerate()
        {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(0.05)
                    .default_value(value)
            });
            subscriptions.push(
                cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                    if let SliderEvent::Change(value) = event {
                        match i {
                            0 => this.options.node_size = value.start(),
                            1 => this.options.link_width = value.start(),
                            2 => this.options.text_scale = value.start(),
                            3 => this.options.center_force = value.start(),
                            4 => this.options.repel_force = value.start(),
                            5 => this.options.link_force = value.start(),
                            _ => this.options.link_distance = value.start(),
                        }
                        if i >= 3 {
                            this.queue_layout(cx);
                        }
                        cx.emit(GraphEvent::Settings(this.options.clone()));
                        cx.notify();
                    } else if matches!(event, SliderEvent::Release(_)) {
                        if i >= 3 {
                            this.recalculate(cx);
                        }
                        cx.emit(GraphEvent::Settings(this.options.clone()));
                    }
                }),
            );
            sliders.push(slider);
        }
        let mut this = Self {
            index,
            options,
            graph: Arc::default(),
            query,
            _query: subscription,
            task: None,
            revision: 0,
            loading: false,
            light,
            bounds: Rc::new(Cell::new(Bounds::default())),
            scale: 1.,
            pan: point(px(0.), px(0.)),
            drag: None,
            hover: None,
            focus: cx.focus_handle(),
            sliders,
            _sliders: subscriptions,
            group_inputs: vec![],
            group_error: String::new(),
            layout_epoch: 0,
            applied_layout_epoch: 0,
            force_task: None,
        };
        this.reset_group_inputs(window, cx);
        this.rebuild(cx);
        this
    }
    pub fn set_index(&mut self, index: Arc<Index>, light: bool, cx: &mut Context<Self>) {
        self.light = light;
        if !Arc::ptr_eq(&self.index, &index) {
            self.index = index;
            self.rebuild(cx);
        }
        cx.notify();
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        cx.emit(GraphEvent::Settings(self.options.clone()));
        self.revision += 1;
        let revision = self.revision;
        let index = self.index.clone();
        let options = self.options.clone();
        let layout_epoch = self.layout_epoch;
        let reset_layout = layout_epoch != self.applied_layout_epoch;
        self.loading = true;
        let work = cx
            .background_executor()
            .spawn(async move { Graph::build(&index, &options) });
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut graph = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                let first = this.graph.nodes.is_empty();
                let positions: std::collections::BTreeMap<_, _> = this
                    .graph
                    .nodes
                    .iter()
                    .map(|n| (&n.path, n.position))
                    .collect();
                if !reset_layout {
                    for node in &mut graph.nodes {
                        if let Some(position) = positions.get(&node.path) {
                            node.position = *position;
                        }
                    }
                }
                this.graph = Arc::new(graph);
                this.applied_layout_epoch = layout_epoch;
                this.loading = false;
                this.hover = None;
                this.drag = None;
                if first {
                    this.fit();
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn fit(&mut self) {
        let mut x = 80f32;
        let mut y = 80f32;
        for n in &self.graph.nodes {
            x = x.max(n.position[0].abs() + 30.);
            y = y.max(n.position[1].abs() + 30.);
        }
        let bounds = self.bounds.get();
        let width = if bounds.size.width > px(0.) {
            f32::from(bounds.size.width)
        } else {
            600.
        };
        let height = if bounds.size.height > px(0.) {
            f32::from(bounds.size.height)
        } else {
            400.
        };
        self.scale = (width / (2. * x)).min(height / (2. * y)).clamp(0.03, 2.);
        self.pan = point(px(0.), px(0.));
    }
    fn screen(&self, i: usize) -> Point<Pixels> {
        let b = self.bounds.get();
        let p = self.graph.nodes[i].position;
        b.center() + self.pan + point(px(p[0] * self.scale), px(p[1] * self.scale))
    }
    fn hit(&self, p: Point<Pixels>) -> Option<usize> {
        self.graph
            .nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(i, _)| {
                let d = self.screen(*i) - p;
                f32::from(d.x).hypot(f32::from(d.y)) < (7. * self.options.node_size).max(9.)
            })
            .map(|(i, _)| i)
    }
    fn zoom(&mut self, factor: f32, at: Point<Pixels>) {
        let old = self.scale;
        self.scale = (old * factor).clamp(0.02, 8.);
        let d = at - self.bounds.get().center();
        self.pan = d - (d - self.pan) * (self.scale / old);
    }
    fn release(&mut self, cx: &mut Context<Self>) {
        if let Some(GraphDrag {
            node: Some(i),
            moved: false,
            ..
        }) = self.drag.take()
        {
            let n = &self.graph.nodes[i];
            cx.emit(GraphEvent::Open(n.path.clone(), n.missing));
        }
        cx.notify();
    }
}
impl Render for GraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let graph = self.graph.clone();
        let bounds = self.bounds.clone();
        let entity_id = cx.entity_id();
        let scale = self.scale;
        let pan = self.pan;
        let hover = self.hover;
        let light = self.light;
        let root = self.options.root.clone();
        let options = self.options.clone();
        let paint_graph = graph.clone();
        let drawing = canvas(
            move |b, _, cx| {
                if bounds.get() != b {
                    bounds.set(b);
                    cx.notify(entity_id);
                }
            },
            move |b, _, window, cx| {
                let pos = |i: usize| {
                    let p = paint_graph.nodes[i].position;
                    b.center() + pan + point(px(p[0] * scale), px(p[1] * scale))
                };
                let mut edges = PathBuilder::stroke(px(options.link_width));
                for &(a, c) in &paint_graph.edges {
                    edges.move_to(pos(a));
                    edges.line_to(pos(c));
                }
                if let Ok(path) = edges.build() {
                    window.paint_path(path, rgb(if light { 0xc7c7c7 } else { 0x454545 }));
                }
                if options.arrows {
                    let mut arrows = PathBuilder::fill();
                    for &(a, c) in &paint_graph.edges {
                        let delta = pos(c) - pos(a);
                        let length = f32::from(delta.x).hypot(f32::from(delta.y));
                        if length < 20. {
                            continue;
                        }
                        let unit = delta / length;
                        let tip = pos(c) - unit * (4. * options.node_size + 2.);
                        let base = tip - unit * 7.;
                        let perpendicular = point(-unit.y, unit.x) * 3.;
                        arrows.move_to(tip);
                        arrows.line_to(base + perpendicular);
                        arrows.line_to(base - perpendicular);
                        arrows.close();
                    }
                    if let Ok(path) = arrows.build() {
                        window.paint_path(path, rgb(if light { 0x999999 } else { 0x777777 }));
                    }
                }
                for (i, n) in paint_graph.nodes.iter().enumerate() {
                    let p = pos(i);
                    if !b.contains(&p) {
                        continue;
                    }
                    let selected = hover == Some(i) || root.as_ref() == Some(&n.path);
                    let radius = if selected {
                        px(7. * options.node_size)
                    } else {
                        px(4. * options.node_size)
                    };
                    let color = rgb(if selected {
                        0x3f9aca
                    } else if let Some(color) = n.color {
                        color
                    } else if n.missing {
                        0x666666
                    } else if light {
                        0x777777
                    } else {
                        0xaaaaaa
                    });
                    window.paint_quad(
                        fill(
                            Bounds::new(p - point(radius, radius), size(radius * 2., radius * 2.)),
                            color,
                        )
                        .corner_radii(radius),
                    );
                    if selected
                        || (options.labels
                            && scale > 0.65 / options.text_scale
                            && paint_graph.nodes.len() < 500)
                    {
                        let label: SharedString = n.label.clone().into();
                        let line = window.text_system().shape_line(
                            label.clone(),
                            px(12. * options.text_scale),
                            &[TextRun {
                                len: label.len(),
                                font: window.text_style().font(),
                                color: rgb(if light { 0x333333 } else { 0xcccccc }).into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        let _ = line.paint(
                            p + point(px(9.), px(-8.)),
                            px(18.),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            },
        )
        .size_full();
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(crate::theme::palette(self.light).background)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(if self.options.root.is_some() {
                        "局部关系图"
                    } else {
                        "关系图谱"
                    })
                    .child(
                        Button::new("graph-fit")
                            .compact()
                            .label("适应")
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.fit();
                                cx.notify()
                            })),
                    )
                    .child(
                        Button::new("graph-close")
                            .compact()
                            .label("返回笔记")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(GraphEvent::Close))),
                    ),
            )
            .child(
                div()
                    .id("graph-canvas")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(drawing)
                    .child(self.controls(cx))
                    .on_scroll_wheel(cx.listener(|s, e: &ScrollWheelEvent, _, cx| {
                        let delta = f32::from(e.delta.pixel_delta(px(20.)).y);
                        s.zoom((delta * 0.002).exp(), e.position);
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|s, e: &MouseDownEvent, window, cx| {
                            window.focus(&s.focus, cx);
                            s.drag = Some(GraphDrag {
                                start: e.position,
                                last: e.position,
                                node: s.hit(e.position),
                                moved: false,
                            });
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                        if let Some(GraphDrag {
                            start,
                            last,
                            node,
                            moved,
                        }) = s.drag
                        {
                            let delta = e.position - last;
                            let distance = e.position - start;
                            let moved =
                                moved || f32::from(distance.x).hypot(f32::from(distance.y)) > 3.;
                            if moved {
                                if let Some(i) = node {
                                    let graph = Arc::make_mut(&mut s.graph);
                                    graph.nodes[i].position[0] += f32::from(delta.x) / s.scale;
                                    graph.nodes[i].position[1] += f32::from(delta.y) / s.scale;
                                } else {
                                    s.pan += delta;
                                }
                            }
                            s.drag = Some(GraphDrag {
                                start,
                                last: e.position,
                                node,
                                moved,
                            });
                        } else {
                            s.hover = s.hit(e.position);
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, cx| s.release(cx)))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|s, _, _, cx| {
                            s.drag = None;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .child(if self.loading {
                        "正在计算关系图…".into()
                    } else if let Some(error) = &graph.error {
                        error.clone()
                    } else {
                        format!(
                            "{} 个节点 · {} 条连接 · 滚轮缩放，拖动平移或移动节点，点击打开笔记",
                            graph.nodes.len(),
                            graph.edges.len()
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn graph_force_release_recomputes_positions(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut index = Index::default();
        index.update("a.md".into(), "[[b]]".into());
        index.update("b.md".into(), "".into());
        let options = Options {
            center_force: 0.,
            repel_force: 0.,
            link_force: 5.,
            link_distance: 0.2,
            ..Default::default()
        };
        let handle =
            cx.add_window(|w, cx| GraphView::new(Arc::new(index), None, false, options, w, cx));
        cx.run_until_parked();
        let distance = |g: &GraphView| {
            let a = g.graph.nodes[0].position;
            let b = g.graph.nodes[1].position;
            (a[0] - b[0]).hypot(a[1] - b[1])
        };
        let before = handle
            .update(cx, |g, _, cx| {
                let before = distance(g);
                g.sliders[6].update(cx, |_, cx| {
                    cx.emit(SliderEvent::Change(3f32.into()));
                    cx.emit(SliderEvent::Release(3f32.into()));
                });
                before
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |g, _, cx| {
                assert!(distance(g) > before + 50.);
                assert_eq!(g.applied_layout_epoch, g.layout_epoch);
                assert!(g.layout_epoch > 0);
                g.options.link_distance = 0.2;
                g.recalculate(cx);
                g.rebuild(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |g, _, _| {
                assert!((distance(g) - before).abs() < 0.01);
                assert_eq!(g.applied_layout_epoch, g.layout_epoch);
            })
            .unwrap();
        handle
            .update(cx, |g, _, cx| {
                g.sliders[6].update(cx, |_, cx| cx.emit(SliderEvent::Change(3f32.into())));
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        handle
            .update(cx, |g, _, _| assert!(distance(g) > before + 50.))
            .unwrap();
    }
    #[gpui::test]
    fn graph_restores_settings_and_slider_events_update_display(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut index = Index::default();
        index.update("a.md".into(), "[[b]]".into());
        index.update("b.md".into(), "".into());
        let options = Options {
            query: "path:a".into(),
            groups: vec![ColorGroup {
                query: "path:a".into(),
                color: 0xff0000,
            }],
            node_size: 2.,
            link_width: 3.,
            arrows: true,
            ..Default::default()
        };
        let handle =
            cx.add_window(|w, cx| GraphView::new(Arc::new(index), None, false, options, w, cx));
        cx.run_until_parked();
        handle
            .update(cx, |g, _, cx| {
                assert_eq!(g.graph.nodes.len(), 1);
                assert_eq!(g.sliders[0].read(cx).value().start(), 2.);
                g.sliders[0].update(cx, |_, cx| cx.emit(SliderEvent::Change(1.5.into())));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |g, _, _| {
                assert_eq!(g.options.node_size, 1.5);
                assert!(g.options.arrows);
            })
            .unwrap();
        handle
            .update(cx, |g, w, cx| {
                g.group_inputs[0].color.update(cx, |input, cx| {
                    input.set_selected_range(0..input.value().len(), cx);
                    input.replace("#12ABEF", w, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |g, _, _| {
                assert_eq!(g.options.groups[0].color, 0x12abef);
                assert_eq!(g.graph.nodes[0].color, Some(0x12abef));
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(430.), px(320.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
    }
    #[gpui::test]
    fn graph_click_drag_zoom_and_latest_filter_work(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut index = Index::default();
        index.update("a.md".into(), "[[b]]".into());
        index.update("b.md".into(), "".into());
        let handle = cx.add_window(|w, cx| {
            GraphView::new(Arc::new(index), None, false, Options::default(), w, cx)
        });
        let received = Rc::new(Cell::new(0));
        let capture = received.clone();
        let _subscription = handle
            .update(cx, |_, _, cx| {
                cx.subscribe(&cx.entity(), move |_, _, event: &GraphEvent, _| {
                    if matches!(event, GraphEvent::Open(..)) {
                        capture.set(capture.get() + 1);
                    }
                })
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let position = handle.update(&mut visual, |g, _, _| g.screen(0)).unwrap();
        visual.simulate_click(position, Modifiers::default());
        assert_eq!(received.get(), 1);
        handle
            .update(&mut visual, |g, _, _| {
                let before = g.screen(0);
                g.zoom(1.5, before);
                let after = g.screen(0);
                assert!((before.x - after.x).abs() < px(0.01));
                assert!((before.y - after.y).abs() < px(0.01));
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let position = handle.update(&mut visual, |g, _, _| g.screen(0)).unwrap();
        visual.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(
            position + point(px(30.), px(20.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        visual.simulate_mouse_up(
            position + point(px(30.), px(20.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        assert_eq!(received.get(), 1);
        handle
            .update(&mut visual, |g, _, cx| {
                assert!((g.screen(0).x - position.x - px(30.)).abs() < px(0.1));
                g.options.query = "path:a".into();
                g.rebuild(cx);
                g.options.query = "path:b".into();
                g.rebuild(cx);
            })
            .unwrap();
        visual.run_until_parked();
        handle
            .update(&mut visual, |g, _, _| {
                assert_eq!(g.graph.nodes.len(), 1);
                assert_eq!(g.graph.nodes[0].path, PathBuf::from("b.md"));
            })
            .unwrap();
    }
}
