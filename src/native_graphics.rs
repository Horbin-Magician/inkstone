//! Native Markdown plugins. Parsing prepares images on TextView's worker;
//! painting only reads cached resources and inherits line typography.
use gpui::{prelude::*, *};
use gpui_base::text::{
    InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode, MarkdownParseContext,
    MarkdownPlugin,
};
use inkstone::graphics::{self, Kind};
use markdown_parser::mdast::Node;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const CACHE_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Prepared {
    pub image: Arc<RenderImage>,
    pub asset: graphics::Graphic,
    pub width: f32,
    pub height: f32,
    pub baseline: Option<f32>,
}

pub(crate) type GraphicResult = Arc<Result<Prepared, String>>;
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: String,
    kind: Kind,
    light: bool,
    font: u32,
    dpi: u32,
}
struct Entry {
    result: Arc<Result<Prepared, String>>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct Cache {
    entries: HashMap<Key, Entry>,
    bytes: usize,
    tick: u64,
}
#[derive(Clone)]
pub(crate) struct Service {
    renderer: Arc<SvgRenderer>,
    cache: Arc<Mutex<Cache>>,
}
impl Global for Service {}

impl Service {
    pub(crate) fn get(cx: &mut App) -> Self {
        if !cx.has_global::<Self>() {
            cx.set_global(Self {
                renderer: Arc::new(cx.svg_renderer()),
                cache: Arc::new(Mutex::new(Cache::default())),
            });
        }
        cx.global::<Self>().clone()
    }
    pub(crate) fn prepare(
        &self,
        kind: Kind,
        source: &str,
        light: bool,
        font: f32,
        dpi: f32,
    ) -> Arc<Result<Prepared, String>> {
        let key = Key {
            source: source.into(),
            kind,
            light,
            font: font.to_bits(),
            dpi: dpi.to_bits(),
        };
        {
            let mut cache = self.cache.lock().unwrap();
            cache.tick += 1;
            let tick = cache.tick;
            if let Some(entry) = cache.entries.get_mut(&key) {
                entry.used = tick;
                return entry.result.clone();
            }
        }
        let result = (|| {
            let asset = graphics::prepare(kind, source, light)?;
            let scale = font
                / if kind == Kind::Mermaid {
                    16.
                } else {
                    graphics::FONT_SIZE
                };
            let pixels = f64::from(asset.width)
                * f64::from(asset.height)
                * f64::from(scale * dpi * gpui::SMOOTH_SVG_SCALE_FACTOR).powi(2);
            anyhow::ensure!(
                pixels.is_finite() && pixels * 4. <= CACHE_LIMIT as f64,
                "图形超过 64 MiB 渲染预算，请减小内容或字号"
            );
            let image = self.renderer.render_single_frame(&asset.svg, scale * dpi)?;
            Ok::<_, anyhow::Error>(Prepared {
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
struct Plugin {
    block: bool,
    light: bool,
    font: f32,
    dpi: f32,
    service: Service,
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
            && !inkstone::syntax::valid_inline_math(cx.source(), &(p.start.offset..p.end.offset))
        {
            return None;
        }
        Some(
            MarkdownNode::new(
                self.name(),
                self.service
                    .prepare(kind, value, self.light, self.font, self.dpi),
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
    fn render(&self, node: &MarkdownNode, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut view = div()
            .id(("native-graphic", node.source_range().map_or(0, |r| r.start)))
            .w_full()
            .overflow_x_scroll();
        match node
            .data::<Arc<Result<Prepared, String>>>()
            .map(|r| r.as_ref())
        {
            Some(Ok(r)) => view = view.child(img(r.image.clone()).w(px(r.width)).h(px(r.height))),
            Some(Err(error)) => {
                view = view
                    .child(div().child(node.as_markdown().to_string()))
                    .child(div().text_size(px(12.)).child(error.clone()))
            }
            None => view = view.child(node.as_markdown().to_string()),
        }
        view
    }
    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<InlineElement> {
        let result = node
            .data::<Arc<Result<Prepared, String>>>()
            .map(|r| r.as_ref());
        let r = match result {
            Some(Ok(r)) => r,
            Some(Err(error)) => {
                return Some(InlineElement::new(
                    div()
                        .child(node.as_markdown().to_string())
                        .child(div().text_size(px(12.)).child(error.clone())),
                ));
            }
            None => return None,
        };
        let scale = f32::from(context.font_size()) / self.font;
        Some(
            InlineElement::new(
                img(r.image.clone())
                    .w(px(r.width * scale))
                    .h(px(r.height * scale)),
            )
            .with_baseline(px(r.baseline.unwrap_or(r.height) * scale)),
        )
    }
}

pub(crate) fn extensions(
    font: f32,
    dpi: f32,
    light: bool,
    strict_breaks: bool,
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
        })
        .plugin(Plugin {
            block: true,
            light,
            font,
            dpi,
            service,
        })
        .parser_revision(revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn outlined_svg_rasterizes_at_dpi_and_cache_preserves_explicit_colors(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let service = Service::get(cx);
            let formula = r"\color{red}{\frac{x}{2}}";
            let first = service.prepare(Kind::InlineMath, formula, false, 16., 2.);
            let repeat = service.prepare(Kind::InlineMath, formula, false, 16., 2.);
            assert!(Arc::ptr_eq(&first, &repeat));
            let prepared = first.as_ref().as_ref().unwrap();
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
