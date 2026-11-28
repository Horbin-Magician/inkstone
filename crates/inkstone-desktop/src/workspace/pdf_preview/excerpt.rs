use super::*;

#[derive(Clone)]
pub(in crate::workspace) struct SourceAnchor {
    pub(super) page: i32,
    pub(super) target: AnchorTarget,
}

#[derive(Clone)]
pub(super) enum AnchorTarget {
    Text { bytes: Range<usize>, digest: String },
    Area(Area),
}

impl SourceAnchor {
    pub(in crate::workspace) fn parse(href: &str) -> Option<Self> {
        let fragment = href.split_once('#')?.1.strip_prefix("page=")?;
        let (number, selection) = fragment.split_once('&')?;
        let page = number.parse::<i32>().ok()?.checked_sub(1)?;
        if page < 0 {
            return None;
        }
        if let Some(value) = selection.strip_prefix("area=") {
            return Some(Self::area(page, Area::parse(value)?));
        }
        let selection = selection.strip_prefix("inkstone=")?;
        let mut fields = selection.split('-');
        let start = fields.next()?.parse().ok()?;
        let end = fields.next()?.parse().ok()?;
        let digest = fields.next()?;
        if start >= end
            || fields.next().is_some()
            || digest.len() != 64
            || !digest.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self {
            page,
            target: AnchorTarget::Text {
                bytes: start..end,
                digest: digest.to_owned(),
            },
        })
    }

    pub(super) fn area(page: i32, area: Area) -> Self {
        Self {
            page,
            target: AnchorTarget::Area(area),
        }
    }

    pub(super) fn matches(&self, page: i32, text: &str) -> bool {
        self.page == page
            && match &self.target {
                AnchorTarget::Text { bytes, digest } => text
                    .get(bytes.clone())
                    .is_some_and(|quote| inkstone_agent::tools::hash(quote) == *digest),
                AnchorTarget::Area(_) => true,
            }
    }
}

impl Reader {
    pub(in crate::workspace) fn show_excerpt(&mut self, anchor: SourceAnchor) {
        self.navigate(if self.count > 0 {
            anchor.page.min(self.count - 1)
        } else {
            anchor.page
        });
        self.source_anchor = Some(anchor);
    }
}

pub(in crate::workspace) struct Excerpt {
    target: usize,
    mark: Mark,
    png: Option<Vec<u8>>,
}

impl EventEmitter<Excerpt> for Reader {}

impl Excerpt {
    pub(super) fn new(
        target: usize,
        mark: &Mark,
        page: Option<&Arc<Page>>,
    ) -> anyhow::Result<Self> {
        let png = match mark.style {
            Style::Area(area) => {
                let page = page.ok_or_else(|| anyhow::anyhow!("请先定位到区域所在页"))?;
                Some(area.png(&page.image)?)
            }
            _ => None,
        };
        Ok(Self {
            target,
            mark: mark.clone(),
            png,
        })
    }

    fn markdown(&self, note: &Path, pdf: &Path, image_path: Option<&Path>) -> String {
        use inkstone_core::locations::{LinkFormat, attachment_link};
        let mark = &self.mark;
        let page = i64::from(mark.page) + 1;
        let mut link = attachment_link(note, pdf, &[], LinkFormat::Relative, true, false);
        // Extend the already escaped attachment destination inside its closing parenthesis.
        link.pop();
        let fragment = match mark.style {
            Style::Area(area) => area.fragment(),
            _ => format!(
                "inkstone={}-{}-{}",
                mark.bytes.start,
                mark.bytes.end,
                inkstone_agent::tools::hash(&mark.quote)
            ),
        };
        let source = format!("{link}#page={page}&{fragment})");
        let mut excerpt = String::from("\n\n");
        if let Some(path) = image_path {
            let embed = attachment_link(note, path, &[], LinkFormat::Relative, true, true);
            excerpt.push_str(&format!("> {embed}\n"));
        }
        for line in mark.quote.lines() {
            excerpt.push_str("> ");
            excerpt.push_str(&literal(line));
            excerpt.push('\n');
        }
        if !mark.comment.is_empty() {
            excerpt.push_str(">\n> 批注：");
            excerpt.push_str(
                &mark
                    .comment
                    .lines()
                    .map(literal)
                    .collect::<Vec<_>>()
                    .join("\n> "),
            );
            excerpt.push('\n');
        }
        excerpt.push_str(&format!("> {source} · 第 {page} 页\n\n"));
        excerpt
    }
}

fn literal(line: &str) -> String {
    let mut text = String::new();
    for ch in line.chars() {
        if "\\`*_[]<>#!|~".contains(ch) {
            text.push('\\');
        }
        text.push(ch);
    }
    text
}

impl Workspace {
    pub(in crate::workspace) fn pdf_note_target(&self) -> Option<&Tab> {
        self.views
            .split
            .as_ref()
            .map(|s| s.source)
            .into_iter()
            .chain(self.views.last_note)
            .find_map(|id| {
                self.tabs.iter().find(|t| {
                    t.id == id
                        && t.preview.is_none()
                        && t.path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                })
            })
    }

    pub(in crate::workspace) fn insert_pdf_excerpt(
        &mut self,
        pdf_id: usize,
        excerpt: &Excerpt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(note) = self.pdf_note_target().filter(|t| t.id == excerpt.target) else {
            self.notifications
                .publish("目标笔记已关闭，请重新选择笔记。".into());
            cx.notify();
            return;
        };
        let Some(pdf) = self.tabs.iter().find(|t| t.id == pdf_id) else {
            return;
        };
        let note_path = note.path.clone();
        let pdf_path = pdf.path.clone();
        let pane = self
            .views
            .split
            .as_ref()
            .filter(|s| s.source == note.id)
            .map_or_else(|| note.pane.clone(), |s| s.pane.clone());
        if self.has_pending_input(excerpt.target, window, cx) {
            self.notifications
                .publish("请先完成笔记中的输入法组词。".into());
            cx.notify();
            return;
        }
        self.flush_document_views(window, cx);
        let editor = pane.read(cx).editor.read(cx);
        if !editor.is_editable() || editor.has_multiple_selections() {
            self.notifications
                .publish("请在可编辑的笔记中选择一个插入位置。".into());
            cx.notify();
            return;
        }
        let image_path = if let Some(bytes) = &excerpt.png {
            if self.file_writes.operation_active() || self.ui.link_update.is_some() {
                self.notifications
                    .publish("当前正在修改库文件，请稍后插入图表。".into());
                cx.notify();
                return;
            }
            let Some(vault) = &self.vault else {
                return;
            };
            let result = (|| -> anyhow::Result<PathBuf> {
                let folder = self
                    .ui
                    .prefs
                    .locations
                    .directory(Some(&note_path), true)
                    .map_err(|e| anyhow::anyhow!(e))?;
                let name = format!("图表-第{}页.png", i64::from(excerpt.mark.page) + 1);
                Ok(vault.store_attachment_to(&folder, &name, bytes)?)
            })();
            match result {
                Ok(path) => Some(path),
                Err(e) => {
                    self.notifications.publish(format!("保存图表附件失败：{e}"));
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let markdown = excerpt.markdown(&note_path, &pdf_path, image_path.as_deref());
        let inserted = pane.update(cx, |p, cx| {
            let applied = p.editor.update(cx, |input, cx| {
                let end = input.selected_range().end;
                let after = end + markdown.len();
                input.apply_source_edit(end..end, &markdown, after..after, window, cx)
            });
            if applied {
                p.reading = false;
                cx.notify();
            }
            applied
        });
        if image_path.is_some() {
            self.structure_changed = true;
            self.refresh_requested = true;
            self.refresh(window, cx);
        }
        self.notifications.publish(
            if inserted {
                "已将批注插入笔记。"
            } else {
                "请在可编辑的笔记中选择一个插入位置。"
            }
            .into(),
        );
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn pdf_excerpt_round_trips_through_the_note_editor(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let vault = Vault::open(root.clone(), temp.path().join("recovery")).unwrap();
        let pdf = PathBuf::from("论文 #1(终).pdf");
        let note = PathBuf::from("notes/研究.md");
        std::fs::write(
            root.join(&pdf),
            include_bytes!("../../../tests/fixtures/two-pages.pdf"),
        )
        .unwrap();
        std::fs::write(root.join(&note), "前后").unwrap();
        let window = cx.add_window(Workspace::new);
        let (reader, editor, id) = window
            .update(cx, |w, window, cx| {
                w.vault = Some(vault);
                w.add_tab(note.clone(), Some("前后".into()), false, window, cx);
                let editor = w.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |input, cx| input.set_selected_range(0.."前".len(), cx));
                w.open_preview_tab(pdf.clone(), window, cx);
                let Some(TabPreview::Pdf(reader)) = w.tabs[1].preview.clone() else {
                    panic!()
                };
                (reader, editor, w.tabs[0].id)
            })
            .unwrap();
        let mark = Mark {
            page: 1,
            bytes: 0..8,
            quote: "INKSTONE".into(),
            style: Style::Highlight,
            color: Color::Pink,
            comment: "两行\n[评论]".into(),
        };
        reader.update(cx, |_, cx| cx.emit(Excerpt::new(id, &mark, None).unwrap()));
        cx.run_until_parked();
        let href = editor.read_with(cx, |input, _| {
            let text = input.value();
            assert!(text.starts_with("前\n\n> INKSTONE"));
            assert!(text.ends_with("\n\n后"));
            assert!(text.contains("> 批注：两行\n> \\[评论\\]"));
            let links = inkstone_core::index::parse(&text).standard_links;
            assert_eq!(links.len(), 1);
            links[0].0.clone()
        });
        let anchor = SourceAnchor::parse(&href).unwrap();
        assert!(anchor.matches(1, "INKSTONE notes"));
        assert!(!anchor.matches(1, "inkstone notes"));
        assert!(!anchor.matches(0, "INKSTONE notes"));
        window
            .update(cx, |w, window, cx| {
                w.focus_primary(0, window, cx);
                w.follow_markdown_link(note, &href, false, window, cx);
                assert_eq!(w.active, Some(1));
                let r = reader.read(cx);
                assert_eq!(r.page, 1);
                assert!(
                    r.source_anchor
                        .as_ref()
                        .is_some_and(|a| a.matches(1, "INKSTONE notes"))
                );
            })
            .unwrap();
    }

    #[gpui::test]
    #[ignore = "requires packaged PDFium beside the test executable"]
    fn pdf_area_drag_inserts_a_crop_and_reopens_the_region(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(init);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let vault = Vault::open(root.clone(), dir.path().join("recovery")).unwrap();
        let pdf = PathBuf::from("报告 #1.pdf");
        let note = PathBuf::from("notes/图表.md");
        std::fs::write(
            root.join(&pdf),
            include_bytes!("../../../tests/fixtures/two-pages.pdf"),
        )
        .unwrap();
        std::fs::write(root.join(&note), "").unwrap();
        let workspace = cx.add_window(Workspace::new);
        let (pane, reader) = workspace
            .update(cx, |w, win, cx| {
                w.vault = Some(vault.clone());
                w.loading = false;
                w.ui.prefs.left_open = false;
                w.ui.prefs.right_open = false;
                w.add_tab(note.clone(), Some(String::new()), false, win, cx);
                let pane = w.tabs[0].pane.clone();
                w.open_preview_tab(pdf.clone(), win, cx);
                let Some(TabPreview::Pdf(reader)) = w.tabs[1].preview.clone() else {
                    panic!()
                };
                (pane, reader)
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(workspace.into(), cx);
        visual.simulate_resize(size(px(1200.), px(800.)));
        let wait = |v: &mut VisualTestContext, ready: fn(&Reader) -> bool| {
            for _ in 0..300 {
                v.executor().advance_clock(Duration::from_millis(15));
                v.run_until_parked();
                v.update(|w, cx| w.draw(cx).clear(cx));
                if reader.read_with(v, |r, _| ready(r)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("reader did not reach expected area state");
        };
        let click = |v: &mut VisualTestContext, id: &'static str| {
            v.update(|w, cx| w.draw(cx).clear(cx));
            let center = v
                .debug_bounds(id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .center();
            v.simulate_click(center, Modifiers::default());
        };
        wait(&mut visual, |r| r.pages.get(&0).is_some_and(|p| p.is_ok()));
        click(&mut visual, "pdf-area-tool");
        let bounds = visual.debug_bounds("pdf-page-0").unwrap();
        let a = bounds.origin + point(px(120.), px(105.));
        let b = bounds.origin + point(px(20.), px(15.));
        visual.simulate_mouse_down(a, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::default());
        visual.simulate_mouse_up(b, MouseButton::Left, Modifiers::default());
        let marks = Store::new(&root.join(&pdf)).read().unwrap();
        let Style::Area(area) = marks[0].style else {
            panic!()
        };
        assert_eq!(area, Area::between(a, b, bounds).unwrap());
        visual.update(|w, cx| {
            reader.update(cx, |r, cx| {
                r.comment.update(cx, |s, cx| s.set_value("样本分布", w, cx))
            })
        });
        click(&mut visual, "save-area");
        click(&mut visual, "pdf-insert-excerpt");
        visual.run_until_parked();
        let content = pane.read_with(&visual, |p, cx| p.editor.read(cx).value().to_string());
        assert!(
            content.contains("![") && content.contains("样本分布"),
            "{content}"
        );
        let parsed = inkstone_core::index::parse(&content);
        let href = parsed
            .standard_links
            .iter()
            .find(|(s, _)| s.contains("&area="))
            .unwrap()
            .0
            .clone();
        let image_path = vault
            .scan_files()
            .unwrap()
            .into_iter()
            .find(|p| p.extension().is_some_and(|e| e == "png"))
            .unwrap();
        let image = image::open(root.join(image_path)).unwrap();
        assert!(image.width() >= 99 && image.height() >= 89);
        assert!(SourceAnchor::parse("file.pdf#page=1&area=4-0-1-9").is_none());
        click(&mut visual, "delete-mark");
        assert!(Store::new(&root.join(&pdf)).read().unwrap().is_empty());
        workspace
            .update(&mut visual, |w, win, cx| {
                w.focus_primary(0, win, cx);
                pane.update(cx, |_, cx| cx.emit(EditorEvent::FollowMarkdownLink(href)));
            })
            .unwrap();
        wait(&mut visual, |r| {
            r.focused_area.is_some() && r.source_anchor.is_none()
        });
        let shown = visual.debug_bounds("pdf-page-0").unwrap();
        reader.read_with(&visual, |r, _| {
            assert_eq!(r.focused_area, Some(area));
            assert!(r.scroll.bounds().contains(&area.bounds(shown).center()));
        });
    }
}
