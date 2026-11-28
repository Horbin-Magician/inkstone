//! Recognize an attachment independently of any reader tab.
use super::{attachment_properties::AttachmentProperties, pdf_preview::Paper, *};
use std::path::Path;

struct Recognition {
    properties: AttachmentProperties,
    source: String,
    lookup_error: Option<String>,
}

impl Workspace {
    pub(super) fn queue_new_papers(&mut self, files: &[PathBuf], cx: &mut Context<Self>) {
        for path in files.iter().filter(|p| pdf_preview::is_pdf(p)) {
            if self.index.files.binary_search(path).is_err() {
                self.queue_paper(path, false, cx);
            }
        }
    }

    pub(super) fn queue_paper(&mut self, path: &Path, manual: bool, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let relative = path.to_owned();
        let target = relative.clone();
        let retry_path = relative.clone();
        self.enqueue_task(
            format!("paper-{relative:?}"),
            format!("识别论文 · {}", relative.display()),
            move |progress| {
                progress.stage("读取 PDF 元数据")?;
                let full = vault.file_path(&relative)?;
                let properties = AttachmentProperties::open(&full);
                let existing = properties
                    .source
                    .as_ref()
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                if !manual && existing.is_some() {
                    return Ok(None);
                }
                let mut paper = pdf_preview::read_paper(&full).map_err(anyhow::Error::msg)?;
                let mut lookup_error = None;
                if let Some(doi) = paper.0.get("doi").and_then(|v| v.as_str()) {
                    progress.stage("通过 DOI 查询 Crossref")?;
                    match paper.clone().lookup(doi) {
                        Ok(record) => paper = record,
                        Err(error) => lookup_error = Some(format!("Crossref 查询失败：{error}")),
                    }
                }
                let source = merge_fields(existing.as_deref(), &paper)?;
                Ok(Some(Recognition {
                    properties,
                    source,
                    lookup_error,
                }))
            },
            move |this, result, _| {
                let Some(mut result) = result? else {
                    return Ok(());
                };
                // Revalidate the vault path after any network wait and before publishing.
                this.vault
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("笔记库已关闭"))?
                    .file_path(&target)?;
                if result
                    .properties
                    .source
                    .as_ref()
                    .ok()
                    .and_then(|s| s.as_deref())
                    != Some(&result.source)
                {
                    result.properties.save(result.source)?;

                    for tab in this
                        .tabs
                        .iter_mut()
                        .filter(|t| t.path == target && t.attachment_properties.is_some())
                    {
                        tab.attachment_properties = Some(result.properties.clone());
                    }
                }
                if let Some(error) = result.lookup_error {
                    anyhow::bail!("已保留本地属性。{error}");
                }
                Ok(())
            },
            move |this, _, cx| this.queue_paper(&retry_path, true, cx),
            cx,
        );
    }
}

fn merge_fields(existing: Option<&str>, paper: &Paper) -> anyhow::Result<String> {
    let Some(existing) = existing else {
        return Ok(paper.source());
    };
    let saved: std::collections::HashSet<_> = inkstone_core::properties::parse(existing)
        .into_iter()
        .map(|p| p.name)
        .collect();
    let mut source = existing.to_owned();
    for (key, value) in &paper.0 {
        if !saved.contains(*key) {
            source = inkstone_core::properties::edit(&source, None, key, &value.to_string())
                .map_err(anyhow::Error::msg)?;
        }
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use serde_json::json;

    #[test]
    fn recognition_only_fills_missing_fields() {
        let mut paper = Paper::default();
        paper.0.insert("title", json!("Publisher title"));
        paper.0.insert("authors", json!(["A", "B"]));
        let source = "---\n# user comment\ntitle: My title\ncustom: [one, two]\n---\n";
        let merged = merge_fields(Some(source), &paper).unwrap();
        assert!(merged.contains("# user comment\ntitle: My title\ncustom: [one, two]"));
        assert!(merged.contains("authors: [\"A\",\"B\"]"));
        assert_eq!(merge_fields(Some(&merged), &paper).unwrap(), merged);
    }

    #[gpui::test]
    #[ignore = "requires packaged PDFium beside the test executable"]
    fn recognition_uses_catalog_additions_and_explicit_requests(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let pdf = include_bytes!("../../tests/fixtures/two-pages.pdf");
        let old = PathBuf::from("existing.pdf");
        let new = PathBuf::from("added.PDF");
        std::fs::write(root.join(&old), pdf).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, win, cx| w.load_vault(root.clone(), win, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, win, cx| {
                w.open_preview_tab(old.clone(), win, cx);
                std::fs::write(root.join(&new), pdf).unwrap();
                w.structure_changed = true;
                w.refresh(win, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let sidecar = |p: &Path| inkstone_core::properties::sidecar(&root.join(p));
        assert!(!sidecar(&old).exists());
        assert!(
            std::fs::read_to_string(sidecar(&new))
                .unwrap()
                .contains("page_count: 2")
        );
        handle
            .update(cx, |w, _, cx| w.queue_paper(&old, true, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(sidecar(&old).is_file());
        handle
            .update(cx, |w, _, _| {
                assert!(
                    w.tabs
                        .iter()
                        .find(|t| t.path == old)
                        .unwrap()
                        .attachment_properties
                        .as_ref()
                        .unwrap()
                        .text()
                        .unwrap()
                        .contains("page_count: 2")
                );
            })
            .unwrap();
    }
}
