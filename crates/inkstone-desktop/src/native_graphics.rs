//! Native Markdown plugins. Parsing measures formulas; painting reads cached
//! images and asks the editor to rasterize only what is on screen.
use crate::theme::MIN_UI_FONT_SIZE;
use gpui::{prelude::*, *};
use gpui_base::text::{
    InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode, MarkdownParseContext,
    MarkdownPlugin,
};
use inkstone_core::graphics::{self, Kind};
use markdown_parser::mdast::Node;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

const CACHE_LIMIT: usize = 64 * 1024 * 1024;
const MEASURE_LIMIT: usize = 4096;
/// Keep raster work off the UI thread and off the rest of a long note.
const RASTER_BATCH: usize = 4;
#[derive(Clone)]
pub(crate) struct Prepared {
    pub kind: Kind,
    pub image: Arc<RenderImage>,
    pub asset: graphics::Graphic,
    pub width: f32,
    pub height: f32,
    pub baseline: Option<f32>,
}

pub(crate) type GraphicResult = Arc<Result<Prepared, String>>;
pub(crate) type MeasuredGraphic = Arc<Result<graphics::Graphic, String>>;
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: String,
    kind: Kind,
    light: bool,
    font: u32,
    dpi: u32,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct AssetKey {
    source: String,
    kind: Kind,
    light: bool,
}
struct Entry {
    result: Arc<Result<Prepared, String>>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<Key, Entry>,
    assets: HashMap<AssetKey, Arc<Result<graphics::Graphic, String>>>,
    inflight: HashSet<Key>,
    bytes: usize,
    tick: u64,
}
#[derive(Clone)]
pub(crate) struct Service {
    renderer: Arc<SvgRenderer>,
    raster_lock: Arc<Mutex<()>>,
    cache: Arc<Mutex<Cache>>,
}
impl Global for Service {}

pub(crate) struct RasterPermit {
    service: Service,
    key: Key,
}
impl Drop for RasterPermit {
    fn drop(&mut self) {
        self.service.finish(&self.key);
    }
}

pub(crate) fn scaled(kind: Kind, font: f32) -> f32 {
    font / if kind == Kind::Mermaid {
        16.
    } else {
        graphics::FONT_SIZE
    }
}

impl Service {
    pub(crate) fn get(cx: &mut App) -> Self {
        if !cx.has_global::<Self>() {
            cx.set_global(Self {
                renderer: Arc::new(cx.svg_renderer()),
                raster_lock: Arc::new(Mutex::new(())),
                cache: Arc::new(Mutex::new(Cache::default())),
            });
        }
        cx.global::<Self>().clone()
    }
    fn key(kind: Kind, source: &str, light: bool, font: f32, dpi: f32) -> Key {
        Key {
            source: source.into(),
            kind,
            light,
            font: font.to_bits(),
            dpi: dpi.to_bits(),
        }
    }
    /// Layout size only. Cold measurements belong on the background parser;
    /// do not hold the cache lock while computing them, since paint uses it too.
    pub(crate) fn measure_only(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
    ) -> Arc<Result<graphics::Graphic, String>> {
        let key = AssetKey {
            source: source.into(),
            kind,
            light,
        };
        if let Some(hit) = self.cache.lock().unwrap().assets.get(&key) {
            return hit.clone();
        }
        let result = Arc::new(graphics::measure(kind, source, light).map_err(|e| format!("{e:#}")));
        let mut cache = self.cache.lock().unwrap();
        // Another parse or raster worker may have filled this entry meanwhile.
        // In particular, never replace an SVG with a measurement-only result.
        if let Some(hit) = cache.assets.get(&key) {
            return hit.clone();
        }
        if cache.assets.len() >= MEASURE_LIMIT {
            cache.assets.clear();
        }
        cache.assets.insert(key, result.clone());
        result
    }
    pub(crate) fn cached(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
        font: f32,
        dpi: f32,
    ) -> Option<GraphicResult> {
        let key = Self::key(kind, source, light, font, dpi);
        let mut cache = self.cache.lock().unwrap();
        cache.tick += 1;
        let tick = cache.tick;
        cache.entries.get_mut(&key).map(|entry| {
            entry.used = tick;
            entry.result.clone()
        })
    }
    pub(crate) fn is_pending(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
        font: f32,
        dpi: f32,
    ) -> bool {
        let key = Self::key(kind, source, light, font, dpi);
        self.cache.lock().unwrap().inflight.contains(&key)
    }
    #[cfg(test)]
    pub(crate) fn raster_count(&self) -> usize {
        self.cache.lock().unwrap().entries.len()
    }
    /// Reserve a raster slot. `None` means it is cached, already running, or
    /// the current batch is full.
    pub(crate) fn begin(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
        font: f32,
        dpi: f32,
    ) -> Option<RasterPermit> {
        let key = Self::key(kind, source, light, font, dpi);
        let mut cache = self.cache.lock().unwrap();
        if cache.entries.contains_key(&key)
            || cache.inflight.contains(&key)
            || cache.inflight.len() >= RASTER_BATCH
        {
            return None;
        }
        cache.inflight.insert(key.clone());
        Some(RasterPermit {
            service: self.clone(),
            key,
        })
    }
    fn finish(&self, key: &Key) {
        self.cache.lock().unwrap().inflight.remove(key);
    }
    pub(crate) fn prepare(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
        font: f32,
        dpi: f32,
    ) -> Arc<Result<Prepared, String>> {
        if let Some(hit) = self.cached(kind, source, light, font, dpi) {
            return hit;
        }
        let measured = self.measure_only(kind, source, light);
        let key = Self::key(kind, source, light, font, dpi);
        let result = (|| {
            let measured = measured
                .as_ref()
                .as_ref()
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .clone();
            let asset = if measured.svg.is_empty() {
                let mut filled = measured;
                filled.svg = graphics::render_svg(kind, source, light)?;
                let filled = Arc::new(Ok(filled));
                self.cache.lock().unwrap().assets.insert(
                    AssetKey {
                        source: source.into(),
                        kind,
                        light,
                    },
                    filled.clone(),
                );
                filled.as_ref().as_ref().unwrap().clone()
            } else {
                measured
            };
            let scale = scaled(kind, font);
            let pixels = f64::from(asset.width)
                * f64::from(asset.height)
                * f64::from(scale * dpi * gpui::SMOOTH_SVG_SCALE_FACTOR).powi(2);
            anyhow::ensure!(
                pixels.is_finite() && pixels * 4. <= CACHE_LIMIT as f64,
                "图形超过 64 MiB 渲染预算，请减小内容或字号"
            );
            let _raster = self.raster_lock.lock().unwrap();
            let image = self.renderer.render_single_frame(&asset.svg, scale * dpi)?;
            Ok::<_, anyhow::Error>(Prepared {
                kind,
                image,
                width: asset.width * scale,
                height: asset.height * scale,
                baseline: asset.baseline.map(|b| b * scale),
                asset,
            })
        })()
        .map_err(|e| format!("{e:#}"));
        let bytes = result.as_ref().map_or(source.len(), |r| {
            let size = r.image.size(0);
            (i32::from(size.width) as usize)
                .saturating_mul(i32::from(size.height) as usize)
                .saturating_mul(4)
                + r.asset.svg.len()
                + source.len()
        });
        let result = Arc::new(result);
        if bytes <= CACHE_LIMIT {
            let mut cache = self.cache.lock().unwrap();
            while cache.bytes.saturating_add(bytes) > CACHE_LIMIT {
                let oldest = cache
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.used)
                    .map(|(k, _)| k.clone());
                let Some(oldest) = oldest else { break };
                cache.bytes -= cache.entries.remove(&oldest).unwrap().bytes;
            }
            // Another view may have prepared the same expression concurrently.
            if let Some(entry) = cache.entries.get(&key) {
                return entry.result.clone();
            }
            cache.tick += 1;
            let used = cache.tick;
            cache.bytes += bytes;
            cache.entries.insert(
                key,
                Entry {
                    result: result.clone(),
                    bytes,
                    used,
                },
            );
        }
        result
    }
}

#[derive(Clone)]
struct FormulaNode {
    kind: Kind,
    value: String,
    measured: Arc<Result<graphics::Graphic, String>>,
}
#[derive(Clone)]
struct Plugin {
    block: bool,
    light: bool,
    font: f32,
    dpi: f32,
    service: Service,
    pane: WeakEntity<crate::editor::EditorPane>,
}
impl MarkdownPlugin for Plugin {
    fn name(&self) -> &str {
        if self.block {
            "inkstone-native-block"
        } else {
            "inkstone-native-inline"
        }
    }
    fn is_block(&self) -> bool {
        self.block
    }
    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let (kind, value) = match node {
            Node::InlineMath(n) if !self.block => (Kind::InlineMath, n.value.as_str()),
            Node::Math(n) if self.block => (Kind::BlockMath, n.value.as_str()),
            Node::Code(n)
                if self.block
                    && n.lang
                        .as_deref()
                        .is_some_and(|l| l.eq_ignore_ascii_case("mermaid")) =>
            {
                (Kind::Mermaid, n.value.as_str())
            }
            _ => return None,
        };
        let source = cx.node_source(node).unwrap_or(value);
        if kind == Kind::InlineMath
            && let Some(p) = node.position()
            && !inkstone_core::syntax::valid_inline_math(
                cx.source(),
                &(p.start.offset..p.end.offset),
            )
        {
            return None;
        }
        Some(
            MarkdownNode::new(
                self.name(),
                FormulaNode {
                    kind,
                    value: value.to_string(),
                    measured: self.service.measure_only(kind, value, self.light),
                },
            )
            .text(source.to_string())
            .markdown(source.to_string())
            .accessibility_label(format!(
                "{}：{value}",
                if kind == Kind::Mermaid {
                    "图表"
                } else {
                    "公式"
                }
            )),
        )
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut view = div()
            .id(("native-graphic", node.source_range().map_or(0, |r| r.start)))
            .w_full()
            .overflow_x_scroll();
        let Some(formula) = node.data::<FormulaNode>() else {
            return view.child(node.as_markdown().to_string());
        };
        if let Err(error) = formula.measured.as_ref() {
            return view
                .child(div().child(node.as_markdown().to_string()))
                .child(div().text_size(px(MIN_UI_FONT_SIZE)).child(error.clone()));
        }
        if formula.kind == Kind::BlockMath {
            view = view.flex();
        }
        self.ensure_raster(formula, window, cx);
        let measured = formula.measured.as_ref().as_ref().unwrap();
        let scale = scaled(formula.kind, self.font);
        let width = measured.width * scale;
        let height = measured.height * scale;
        let offset = node.source_range().map_or(0, |r| r.start);
        if let Some(Ok(prepared)) = self
            .service
            .cached(
                formula.kind,
                &formula.value,
                self.light,
                self.font,
                self.dpi,
            )
            .as_ref()
            .map(|r| r.as_ref())
        {
            view = view.child(if prepared.kind == Kind::Mermaid {
                div()
                    .id(("native-graphic-image", offset))
                    .debug_selector(move || format!("native-graphic-image-{offset}"))
                    .relative()
                    .w_full()
                    .max_w(px(prepared.width))
                    .aspect_ratio(prepared.width / prepared.height)
                    .child(img(prepared.image.clone()).absolute().inset_0().size_full())
                    .into_any_element()
            } else {
                img(prepared.image.clone())
                    .w(px(prepared.width))
                    .h(px(prepared.height))
                    .when(formula.kind == Kind::BlockMath, |image| {
                        image.flex_shrink_0().mx_auto()
                    })
                    .into_any_element()
            });
        } else if formula.kind == Kind::Mermaid {
            view = view.child(
                div()
                    .id(("native-graphic-image", offset))
                    .w_full()
                    .max_w(px(width))
                    .aspect_ratio(width / height.max(1.)),
            );
        } else {
            view = view.child(
                div()
                    .w(px(width))
                    .h(px(height))
                    .when(formula.kind == Kind::BlockMath, |placeholder| {
                        placeholder.flex_shrink_0().mx_auto()
                    }),
            );
        }
        view
    }
    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let formula = node.data::<FormulaNode>()?;
        let measured = match formula.measured.as_ref() {
            Ok(measured) => measured,
            Err(error) => {
                return Some(InlineElement::new(
                    div()
                        .child(node.as_markdown().to_string())
                        .child(div().text_size(px(MIN_UI_FONT_SIZE)).child(error.clone())),
                ));
            }
        };
        self.ensure_raster(formula, window, cx);
        let scale = f32::from(context.font_size()) / self.font;
        if let Some(Ok(prepared)) = self
            .service
            .cached(
                formula.kind,
                &formula.value,
                self.light,
                self.font,
                self.dpi,
            )
            .as_ref()
            .map(|r| r.as_ref())
        {
            return Some(
                InlineElement::new(
                    img(prepared.image.clone())
                        .w(px(prepared.width * scale))
                        .h(px(prepared.height * scale)),
                )
                .with_baseline(px(prepared.baseline.unwrap_or(prepared.height) * scale)),
            );
        }
        let width = measured.width * scaled(formula.kind, self.font) * scale;
        let height = measured.height * scaled(formula.kind, self.font) * scale;
        let baseline =
            measured.baseline.unwrap_or(measured.height) * scaled(formula.kind, self.font);
        Some(
            InlineElement::new(div().w(px(width)).h(px(height)))
                .with_baseline(px(baseline * scale)),
        )
    }
}

impl Plugin {
    fn ensure_raster(&self, node: &FormulaNode, window: &mut Window, cx: &mut App) {
        if self
            .service
            .cached(node.kind, &node.value, self.light, self.font, self.dpi)
            .is_some()
            || self
                .service
                .is_pending(node.kind, &node.value, self.light, self.font, self.dpi)
        {
            return;
        }
        let pane = self.pane.clone();
        let kind = node.kind;
        let value = node.value.clone();
        let light = self.light;
        let font = self.font;
        let dpi = self.dpi;
        window.defer(cx, move |_, cx| {
            let _ = pane.update(cx, |pane, cx| {
                pane.enqueue_graphic(kind, value, light, font, dpi, cx);
            });
        });
    }
}

pub(crate) fn extensions(
    font: f32,
    dpi: f32,
    light: bool,
    strict_breaks: bool,
    pane: WeakEntity<crate::editor::EditorPane>,
    cx: &mut App,
) -> MarkdownExtensions {
    let service = Service::get(cx);
    let revision = u64::from(font.to_bits())
        ^ (u64::from(dpi.to_bits()) << 32)
        ^ u64::from(light)
        ^ (u64::from(strict_breaks) << 1);
    MarkdownExtensions::default()
        .frontmatter()
        .custom_task_markers(true)
        .soft_line_breaks(!strict_breaks)
        .plugin(Plugin {
            block: false,
            light,
            font,
            dpi,
            service: service.clone(),
            pane: pane.clone(),
        })
        .plugin(Plugin {
            block: true,
            light,
            font,
            dpi,
            service,
            pane,
        })
        .parser_revision(revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn diagrams_fit_reading_and_live_width_without_changing_aspect_or_source(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "```mermaid\nflowchart LR\nA[开始节点] --> B[处理节点] --> C[检查节点] --> D[完成节点]\n```\n\ntail";
        let handle = cx.add_window(|window, cx| {
            let mut pane = crate::editor::EditorPane::new(source, window, cx);
            pane.reading = true;
            pane
        });
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        let mut wide = None;
        for width in [1000., 480.] {
            visual.simulate_resize(size(px(width), px(700.)));
            for _ in 0..10 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            let bounds = visual.debug_bounds("native-graphic-image-0").unwrap();
            assert!(bounds.size.width <= px(width));
            if let Some(wide) = wide {
                let wide: Size<Pixels> = wide;
                assert!(bounds.size.width < wide.width);
                assert!(
                    (bounds.size.height - bounds.size.width * (wide.height / wide.width)).abs()
                        <= px(1.),
                    "wide={wide:?}, narrow={:?}",
                    bounds.size
                );
            } else {
                wide = Some(bounds.size);
            }
        }
        handle
            .update(&mut visual, |pane, _, cx| {
                pane.reading = false;
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(source.len()..source.len(), cx)
                });
                cx.notify();
            })
            .unwrap();
        for _ in 0..10 {
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |pane, window, cx| {
                let object = pane.editor.read(cx).display_objects()[0].clone();
                assert!(object.size.width <= px(480.));
                let service = Service::get(cx);
                let prepared = service.prepare(
                    Kind::Mermaid,
                    "flowchart LR\nA[开始节点] --> B[处理节点] --> C[检查节点] --> D[完成节点]",
                    pane.light,
                    pane.font_size,
                    window.scale_factor(),
                );
                let prepared = prepared.as_ref().as_ref().unwrap();
                assert!(
                    (f32::from(object.size.height) / prepared.height
                        - (f32::from(object.size.width) / prepared.width).min(1.))
                    .abs()
                        < 0.01
                );
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn concurrent_measurements_share_results_and_keep_completed_svg(cx: &mut TestAppContext) {
        let service = cx.update(Service::get);
        let source = r"\frac{a^2+b^2}{\sqrt{1+x}}";
        let gate = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let service = service.clone();
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    service.measure_only(Kind::InlineMath, source, false)
                })
            })
            .collect();
        let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert!(results[0].is_ok());
        assert!(results.iter().all(|r| Arc::ptr_eq(r, &results[0])));

        let prepared = service.prepare(Kind::InlineMath, source, false, 16., 2.);
        let prepared = prepared.as_ref().as_ref().unwrap();
        let measured = service.measure_only(Kind::InlineMath, source, false);
        assert!(Arc::ptr_eq(
            &measured.as_ref().as_ref().unwrap().svg,
            &prepared.asset.svg,
        ));
    }

    #[gpui::test]
    fn outlined_svg_rasterizes_at_dpi_and_cache_preserves_explicit_colors(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let service = Service::get(cx);
            let formula = r"\color{red}{\frac{x}{2}}";
            let first = service.prepare(Kind::InlineMath, formula, false, 16., 2.);
            let repeat = service.prepare(Kind::InlineMath, formula, false, 16., 2.);
            assert!(Arc::ptr_eq(&first, &repeat));
            let prepared = first.as_ref().as_ref().unwrap();
            let zoomed = service.prepare(Kind::InlineMath, formula, false, 24., 1.5);
            let zoomed = zoomed.as_ref().as_ref().unwrap();
            assert!(Arc::ptr_eq(&prepared.asset.svg, &zoomed.asset.svg));
            assert!((zoomed.width / prepared.width - 1.5).abs() < 0.001);
            assert!((zoomed.baseline.unwrap() / prepared.baseline.unwrap() - 1.5).abs() < 0.001);
            assert!(
                std::str::from_utf8(&prepared.asset.svg)
                    .unwrap()
                    .contains("rgb(255,0,0)")
            );
            assert!(i32::from(prepared.image.size(0).width) as f32 >= prepared.width * 2.);
            assert!(prepared.baseline.unwrap() <= prepared.height);
            let error = service.prepare(Kind::InlineMath, r"\unknowncommand", false, 16., 1.);
            assert!(error.is_err());
            assert!(service.cache.lock().unwrap().bytes <= CACHE_LIMIT);
        });
    }
}
