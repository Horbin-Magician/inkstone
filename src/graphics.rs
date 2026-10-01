//! Native math and diagram preparation. No UI, JavaScript, or external tools.
use anyhow::{Context, Result, ensure};
use std::sync::Arc;

pub const FONT_SIZE: f32 = 20.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    InlineMath,
    BlockMath,
    Mermaid,
}

#[derive(Clone, Debug)]
pub struct Graphic {
    pub svg: Arc<[u8]>,
    pub width: f32,
    pub height: f32,
    pub baseline: Option<f32>,
}

pub fn prepare(kind: Kind, source: &str, light: bool) -> Result<Graphic> {
    ensure!(source.len() <= 65_536, "渲染内容超过 64 KiB 上限");
    std::panic::catch_unwind(|| match kind {
        Kind::InlineMath | Kind::BlockMath => math(source, kind == Kind::BlockMath, light),
        Kind::Mermaid => diagram(source, light),
    })
    .map_err(|_| anyhow::anyhow!("渲染器无法处理此内容"))?
}

fn math(source: &str, display: bool, light: bool) -> Result<Graphic> {
    use ratex_types::{color::Color, math_style::MathStyle};
    let ast = ratex_parser::parse(source).context("公式语法错误")?;
    let options = ratex_layout::LayoutOptions {
        style: if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        },
        color: Color::from_hex(if light { "#222222" } else { "#dadada" }).unwrap(),
        ..Default::default()
    };
    let layout = ratex_layout::layout(&ast, &options);
    let list = ratex_layout::to_display_list(&layout);
    let padding = 2.;
    let width = list.width as f32 * FONT_SIZE + padding * 2.;
    let height = list.total_height() as f32 * FONT_SIZE + padding * 2.;
    validate_size(width, height)?;
    let svg = ratex_svg::render_to_svg_with_color_syntax(
        &list,
        &ratex_svg::SvgOptions {
            font_size: f64::from(FONT_SIZE),
            padding: f64::from(padding),
            embed_glyphs: true,
            ..Default::default()
        },
        ratex_svg::SvgColorSyntax::Rgb,
    );
    Ok(Graphic {
        svg: svg.into_bytes().into(),
        width,
        height,
        baseline: Some(list.height as f32 * FONT_SIZE + padding),
    })
}

fn diagram(source: &str, light: bool) -> Result<Graphic> {
    use mermaid_rs_renderer::{RenderOptions, Theme};
    let options = RenderOptions {
        theme: if light {
            Theme::modern()
        } else {
            Theme::dark()
        },
        ..Default::default()
    };
    let mut parsed = mermaid_rs_renderer::parse_mermaid_strict(source).context("图表语法错误")?;
    // 0.3.1 parses timeline sections but its SVG renderer omits their labels.
    // Keep group names at the first period of each section in the native layout.
    if parsed.graph.kind == mermaid_rs_renderer::DiagramKind::Timeline {
        let mut previous = None;
        for event in &mut parsed.graph.timeline.events {
            if event.section != previous {
                if let Some(section) = &event.section {
                    event.time = format!("{section}<br/>{}", event.time);
                }
                previous = event.section.clone();
            }
        }
    }
    let layout =
        mermaid_rs_renderer::compute_layout(&parsed.graph, &options.theme, &options.layout);
    let svg = mermaid_rs_renderer::render_svg(&layout, &options.theme, &options.layout);
    static VIEWBOX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"viewBox="[\d.\-]+\s+[\d.\-]+\s+([\d.]+)\s+([\d.]+)""#).unwrap()
    });
    let dimensions = VIEWBOX.captures(&svg).context("图表没有有效尺寸")?;
    let width: f32 = dimensions[1].parse()?;
    let height: f32 = dimensions[2].parse()?;
    validate_size(width, height)?;
    Ok(Graphic {
        svg: svg.into_bytes().into(),
        width,
        height,
        baseline: None,
    })
}

fn validate_size(width: f32, height: f32) -> Result<()> {
    ensure!(
        width.is_finite()
            && height.is_finite()
            && width > 0.
            && height > 0.
            && width <= 8192.
            && height <= 8192.,
        "渲染尺寸无效或超过 8192px 上限"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_math_covers_required_formulas_with_outlined_fonts_and_baselines() {
        let snapshot =
            crate::syntax::Snapshot::new(include_str!("../tests/fixtures/markdown/math.md"));
        fn visit(node: &markdown_parser::mdast::Node, formulas: &mut Vec<(Kind, String)>) {
            use markdown_parser::mdast::Node;
            match node {
                Node::InlineMath(n) if !n.value.contains("inkstoneUnknown") => {
                    formulas.push((Kind::InlineMath, n.value.clone()))
                }
                Node::Math(n) => formulas.push((Kind::BlockMath, n.value.clone())),
                _ => {}
            }
            if let Some(children) = node.children() {
                for n in children {
                    visit(n, formulas);
                }
            }
        }
        let mut formulas = vec![];
        visit(snapshot.ast.as_deref().unwrap(), &mut formulas);
        assert!(formulas.len() >= 9);
        for (kind, formula) in formulas {
            for light in [false, true] {
                let asset =
                    prepare(kind, &formula, light).unwrap_or_else(|e| panic!("{formula}: {e:#}"));
                let svg = std::str::from_utf8(&asset.svg).unwrap();
                assert!(svg.contains("<path"), "{formula}: {svg}");
                assert!(!svg.contains("<text"), "font outlines required: {formula}");
                let baseline = asset.baseline.unwrap();
                assert!(baseline > 0. && baseline <= asset.height);
            }
        }
        assert!(prepare(Kind::InlineMath, r"\inkstoneUnknown{x}", true).is_err());
        assert!(prepare(Kind::InlineMath, &"{".repeat(10000), true).is_err());
        assert!(prepare(Kind::InlineMath, &"x".repeat(65537), true).is_err());
    }

    #[test]
    fn native_diagrams_cover_eleven_kinds_and_chinese_labels() {
        let examples = [
            "flowchart TD\nA[开始] --> B{判断}\nB -->|是| C[结束]",
            "sequenceDiagram\nparticipant A as 用户\nparticipant B as 系统\nA->>B: 请求\nB-->>A: 响应",
            "classDiagram\nAnimal <|-- Duck\nAnimal : +eat()",
            "stateDiagram-v2\n[*] --> Ready\nReady --> Done\nDone --> [*]",
            "erDiagram\nUSER ||--o{ NOTE : owns\nUSER { int id }\nNOTE { int id }",
            "gantt\ntitle 计划\ndateFormat YYYY-MM-DD\nsection 开发\n工作 :a1, 2026-09-30, 3d",
            "pie title 比例\n\"完成\" : 70\n\"待办\" : 30",
            "mindmap\n  root((主题))\n    分支一\n    分支二",
            "gitGraph\ncommit\nbranch feature\ncheckout feature\ncommit\ncheckout main\nmerge feature",
            "timeline\ntitle 时间线\n2026 : 开始\n2027 : 完成",
            "journey\ntitle 工作\nsection 开始\n规划: 5: 用户\n实施: 3: 用户",
        ];
        for source in examples {
            for light in [false, true] {
                let asset = prepare(Kind::Mermaid, source, light)
                    .unwrap_or_else(|e| panic!("{source}: {e:#}"));
                assert!(
                    asset.svg.starts_with(b"<svg")
                        || std::str::from_utf8(&asset.svg).unwrap().contains("<svg")
                );
            }
        }
        assert!(prepare(Kind::Mermaid, "flowchart TD\ninvalid -->", true).is_err());
    }

    #[test]
    fn combined_diagrams_preserve_all_required_labels_in_both_themes() {
        let snapshot =
            crate::syntax::Snapshot::new(include_str!("../tests/fixtures/markdown/mermaid.md"));
        let expected: &[&[&str]] = &[
            &["分组", "开始", "判断", "结束", "是", "否"],
            &[
                "用户", "系统", "重试", "请求", "成功", "响应", "失败", "错误", "完成",
            ],
            &["Animal", "Duck", "Pond", "eat", "swim", "lives"],
            &["Ready", "Running", "Done", "开始", "完成", "重试"],
            &[
                "USER", "NOTE", "FOLDER", "owns", "contains", "name", "title",
            ],
            &["计划", "开发", "设计", "实施", "验收", "测试"],
            &["比例", "完成", "开发", "待办"],
            &["主题", "分支一", "子项", "分支二"],
            &["初始", "功能", "修复", "合并", "feature", "main"],
            &[
                "时间线",
                "开发",
                "2026",
                "开始",
                "设计",
                "2027",
                "实施",
                "验收",
                "2028",
                "完成",
            ],
            &[
                "工作", "开始", "规划", "用户", "系统", "实施", "开发", "验收",
            ],
        ];
        let diagrams: Vec<_> = snapshot
            .ast
            .as_deref()
            .unwrap()
            .children()
            .unwrap()
            .iter()
            .filter_map(|node| match node {
                markdown_parser::mdast::Node::Code(n) if n.lang.as_deref() == Some("mermaid") => {
                    Some(&n.value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(diagrams.len(), expected.len());
        for (source, labels) in diagrams.into_iter().zip(expected) {
            for light in [true, false] {
                let asset = prepare(Kind::Mermaid, source, light)
                    .unwrap_or_else(|e| panic!("{source}: {e:#}"));
                let svg = std::str::from_utf8(&asset.svg).unwrap();
                for label in *labels {
                    assert!(
                        svg.contains(label),
                        "missing {label:?} in diagram:\n{source}"
                    );
                }
            }
        }
    }
}
