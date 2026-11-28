use super::*;

#[derive(Clone)]
pub(in crate::workspace) struct SourceAnchor {
    page: i32,
    pub(super) bytes: Range<usize>,
    digest: String,
}

impl SourceAnchor {
    pub(in crate::workspace) fn parse(href: &str) -> Option<Self> {
        let fragment = href.split_once('#')?.1.strip_prefix("page=")?;
        let (number, selection) = fragment.split_once("&inkstone=")?;
        let page = number.parse::<i32>().ok()?.checked_sub(1)?;
        let mut fields = selection.split('-');
        let start = fields.next()?.parse().ok()?;
        let end = fields.next()?.parse().ok()?;
        let digest = fields.next()?;
        if page < 0
            || start >= end
            || fields.next().is_some()
            || digest.len() != 64
            || !digest.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self {
            page,
            bytes: start..end,
            digest: digest.to_owned(),
        })
    }

    pub(super) fn matches(&self, page: i32, text: &str) -> bool {
        self.page == page
            && text
                .get(self.bytes.clone())
                .is_some_and(|quote| inkstone_agent::tools::hash(quote) == self.digest)
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
}

impl EventEmitter<Excerpt> for Reader {}

impl Excerpt {
    pub(super) fn new(target: usize, mark: &Mark) -> Self {
        Self {
            target,
            mark: mark.clone(),
        }
    }

    fn markdown(&self, note: &Path, pdf: &Path) -> String {
        use inkstone_core::locations::{LinkFormat, attachment_link};
        let mark = &self.mark;
        let page = i64::from(mark.page) + 1;
        let mut link = attachment_link(note, pdf, &[], LinkFormat::Relative, true, false);
        // Extend the already escaped attachment destination inside its closing parenthesis.
        link.pop();
        let source = format!(
            "{link}#page={page}&inkstone={}-{}-{})",
            mark.bytes.start,
            mark.bytes.end,
            inkstone_agent::tools::hash(&mark.quote)
        );
        let mut excerpt = String::from("\n\n");
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
        let markdown = excerpt.markdown(&note.path, &pdf.path);
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
        reader.update(cx, |_, cx| cx.emit(Excerpt::new(id, &mark)));
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
}
