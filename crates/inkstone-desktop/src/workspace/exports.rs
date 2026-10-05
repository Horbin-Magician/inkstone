use super::*;
use inkstone_core::vault::export::{self, Format};

impl Workspace {
    pub(super) fn choose_export(
        &mut self,
        format: Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.exporting {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        let path = tab.path.clone();
        let generation = self.generation;
        let name = format!(
            "{}-{}",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            if format == Format::Html {
                "HTML导出"
            } else {
                "Markdown导出"
            }
        );
        let dialog =
            cx.prompt_for_new_path(vault.root.parent().unwrap_or(&vault.root), Some(&name));
        self.ui.exporting = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = dialog.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.ui.exporting = false;
                if let Ok(Ok(Some(destination))) = result {
                    this.start_export(path, destination, format, window, cx);
                }
            });
        })
        .detach();
    }
    fn start_export(
        &mut self,
        path: PathBuf,
        destination: PathBuf,
        format: Format,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if self.ui.exporting {
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| self.has_pending_input(t.id, window, cx))
        {
            self.notifications
                .publish("请先完成输入法组词或脚注编辑，再导出。".into());
            cx.notify();
            return;
        }
        self.flush_document_views(window, cx);
        let drafts = self
            .tabs
            .iter()
            .map(|t| (t.path.clone(), t.save.editor.read(cx).value().to_string()))
            .collect();
        self.ui.exporting = true;
        self.ui.pending_file_writes += 1;
        self.notifications
            .publish("正在导出当前编辑快照、关联笔记与附件……".into());
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { export::create(&vault, &path, drafts, &destination, format) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.ui.exporting = false;
                match result {
                    Ok(report) => {
                        this.notifications.publish(format!(
                            "已导出 {} 篇笔记、{} 个附件；{} 条说明见“导出说明.txt”。{}",
                            report.notes,
                            report.attachments,
                            report.warnings.len(),
                            if format == Format::Html {
                                "在浏览器中打印可另存为 PDF。"
                            } else {
                                "请携带整个导出目录。"
                            }
                        ));
                        #[cfg(not(test))]
                        cx.reveal_path(&report.entry);
                        #[cfg(not(test))]
                        if format == Format::Html {
                            let path = report.entry.to_string_lossy().replace('\\', "/");
                            let encoded = path
                                .split('/')
                                .map(|s| {
                                    #[cfg(windows)]
                                    if s.len() == 2
                                        && s.as_bytes()[0].is_ascii_alphabetic()
                                        && s.ends_with(':')
                                    {
                                        return s.to_owned();
                                    }
                                    percent_encoding::utf8_percent_encode(
                                        s,
                                        percent_encoding::NON_ALPHANUMERIC,
                                    )
                                    .to_string()
                                })
                                .collect::<Vec<_>>()
                                .join("/");
                            #[cfg(windows)]
                            let url = format!("file:///{encoded}");
                            #[cfg(not(windows))]
                            let url = format!("file://{encoded}");
                            cx.open_url(&url);
                        }
                    }
                    Err(error) => this
                        .notifications
                        .publish(format!("导出失败：{error}。现有文件未被覆盖。")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn export_uses_open_draft_without_saving_or_overwriting(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-export-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/a.md"), "disk").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("vault"), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note("a.md".into(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("# 未保存 😀\r\n", window, cx));
                w.start_export(
                    "a.md".into(),
                    root.join("export"),
                    Format::Markdown,
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("export/notes/a.md")).unwrap(),
            "# 未保存 😀\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("vault/a.md")).unwrap(),
            "disk"
        );
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.ui.exporting);
                assert_eq!(w.ui.pending_file_writes, 0);
                w.start_export(
                    "a.md".into(),
                    root.join("export"),
                    Format::Markdown,
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.notifications.text().contains("导出失败"));
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
