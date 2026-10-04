use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
};
use std::rc::Rc;

pub(super) struct Review {
    document: Rc<document::DocumentState>,
    path: PathBuf,
    baseline: Option<String>,
    local: String,
    disk: Option<String>,
    ready: bool,
    message: String,
    preview: Entity<TextareaState>,
}

impl Workspace {
    pub(super) fn open_conflict_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(id) = self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id) else {
            return;
        };
        if self.has_pending_input(id, window, cx) || self.ui.file_operation {
            self.status = "请完成当前编辑或文件操作后比较。".into();
            cx.notify();
            return;
        }
        self.flush_document_views(window, cx);
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        if tab.save.saving.get() || tab.path.as_os_str().is_empty() {
            return;
        }
        let document = tab.save.clone();
        let path = document.path.borrow().clone();
        let baseline = document.baseline.borrow().clone();
        let local = document.editor.read(cx).value().to_string();
        self.close_overlays(window, cx);
        let preview = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).rows(16);
            s.set_readonly(true, cx);
            s
        });
        self.ui.conflict_review = Some(Review {
            document,
            path: path.clone(),
            baseline,
            local: local.clone(),
            disk: None,
            ready: false,
            message: "正在读取磁盘版本……".into(),
            preview,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let disk = vault.read(&path)?;
            let preview = if disk.as_deref() == Some(&local) {
                "本地正文与磁盘内容一致。".into()
            } else {
                let diff = similar::TextDiff::configure()
                    .timeout(Duration::from_millis(100))
                    .diff_lines(disk.as_deref().unwrap_or(""), &local);
                recovery::preview_text(
                    &diff
                        .unified_diff()
                        .context_radius(3)
                        .header("磁盘版本", "本地编辑")
                        .to_string(),
                )
            };
            Ok::<_, VaultError>((disk, preview))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(review) = &mut this.ui.conflict_review else {
                    return;
                };
                match result {
                    Ok((disk, preview)) => {
                        review.message = if disk.is_none() {
                            "磁盘文件已不存在；保留本地会重新创建文件。".into()
                        } else {
                            String::new()
                        };
                        review.disk = disk;
                        review.ready = true;
                        review
                            .preview
                            .update(cx, |s, cx| s.set_value(preview, window, cx));
                    }
                    Err(error) => review.message = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resolve_review(&mut self, keep_local: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if self.ui.file_operation || self.ui.pending_file_writes > 0 {
            return;
        }
        self.flush_document_views(window, cx);
        let Some(review) = &self.ui.conflict_review else {
            return;
        };
        if !review.ready || (!keep_local && review.disk.is_none()) {
            return;
        }
        let document = review.document.clone();
        let path = review.path.clone();
        let baseline = review.baseline.clone();
        let local = review.local.clone();
        let disk = review.disk.clone();
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| Rc::ptr_eq(&t.save, &document))
            .map(|t| t.id)
            .collect();
        if tabs.is_empty()
            || document.saving.get()
            || *document.path.borrow() != path
            || *document.baseline.borrow() != baseline
            || document.editor.read(cx).value().as_ref() != local
            || tabs
                .iter()
                .any(|id| self.has_pending_input(*id, window, cx))
        {
            if let Some(review) = &mut self.ui.conflict_review {
                review.ready = false;
                review.message = "比较后本地内容已变化，请重新比较。".into();
            }
            cx.notify();
            return;
        }
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        document.saving.set(true);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let expected_local = local.clone();
        let changed = path.clone();
        let task = cx.background_executor().spawn(async move {
            vault.resolve_conflict(
                &path,
                baseline.as_deref(),
                &local,
                disk.as_deref(),
                keep_local.then_some(local.as_str()),
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.file_operation = false;
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                document.saving.set(false);
                if this.generation != generation {
                    return;
                }
                this.flush_document_views(window, cx);
                let still_current = document.editor.read(cx).value().as_ref() == expected_local
                    && !document.editor.read(cx).is_composing()
                    && !this
                        .tabs
                        .iter()
                        .filter(|t| Rc::ptr_eq(&t.save, &document))
                        .any(|t| this.has_pending_input(t.id, window, cx));
                match result {
                    Ok(text) => {
                        document.baseline.replace(Some(text.clone()));
                        this.note_indexed_change(changed.clone(), text.clone(), cx);
                        if still_current {
                            document.conflict.set(false);
                            document.error.replace(None);
                            document
                                .editor
                                .update(cx, |s, cx| s.replace_all(text, window, cx));
                            this.document_changed(document.editor.clone(), window, cx);
                            this.status = "冲突已处理，处理前的本地草稿已保留在文件恢复中。".into();
                            if this.ui.recovery_refresh == request {
                                this.close_overlays(window, cx);
                            }
                        } else {
                            document.conflict.set(true);
                            document.dirty.set(true);
                            document
                                .error
                                .replace(Some("处理期间产生了新输入，已保留，请重新比较。".into()));
                            if let Some(review) = &mut this.ui.conflict_review {
                                review.ready = false;
                                review.message =
                                    "处理期间产生了新输入，已保留，请重新比较。".into();
                            }
                        }
                    }
                    Err(error) => {
                        let message = format!("未完成处理：{error}。请重新比较或另存副本。");
                        document.conflict.set(true);
                        document.error.replace(Some(message.clone()));
                        this.status = message.clone();
                        if let Some(review) = &mut this.ui.conflict_review {
                            review.ready = false;
                            review.message = message;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn conflict_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = &self.ui.conflict_review else {
            return div().into_any_element();
        };
        let disabled = !review.ready || self.ui.file_operation;
        div().id("conflict-content").overflow_y_scroll().flex().flex_col().min_h_0().gap_2()
            .child(review.path.to_string_lossy().to_string())
            .child("差异：减号为磁盘内容，加号为本地编辑。")
            .when(!review.message.is_empty(), |s| s.child(review.message.clone()))
            .child(Textarea::new(&review.preview).readonly(true).h(px(300.)))
            .child("处理前会保留本地草稿；保存时再次检查磁盘版本。需要手动合并时，可关闭此窗口在正文中编辑，再重新比较。")
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("conflict-refresh").label("重新比较").disabled(self.ui.file_operation)
                    .on_click(cx.listener(|this, _, w, cx| this.open_conflict_review(w, cx))))
                .child(Button::new("conflict-local").primary().label("保留本地并保存").disabled(disabled)
                    .on_click(cx.listener(|this, _, w, cx| this.resolve_review(true, w, cx))))
                .child(Button::new("conflict-disk").label("采用磁盘版本").disabled(disabled || review.disk.is_none())
                    .on_click(cx.listener(|this, _, w, cx| this.resolve_review(false, w, cx)))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn conflict_review_rejects_stale_disk_and_preserves_edits_during_resolution(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-conflict-ui-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("note.md"), "base").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.open_note("note.md".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.watcher = None;
                w.watch_events = None;
                let doc = w.tabs[w.active.unwrap()].save.clone();
                doc.conflict.set(true);
                doc.editor
                    .update(cx, |s, cx| s.replace_all("local😀", window, cx));
                w.document_changed(doc.editor.clone(), window, cx);
                std::fs::write(root.join("note.md"), "external").unwrap();
                w.open_conflict_review(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.conflict_review.as_ref().unwrap().ready);
                std::fs::write(root.join("note.md"), "newer external").unwrap();
                w.resolve_review(true, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("note.md")).unwrap(),
            "newer external"
        );
        handle
            .update(cx, |w, window, cx| {
                let doc = w.tabs[w.active.unwrap()].save.clone();
                assert!(doc.conflict.get());
                assert_eq!(doc.editor.read(cx).value().as_ref(), "local😀");
                w.open_conflict_review(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.resolve_review(false, window, cx);
                let doc = w.tabs[w.active.unwrap()].save.clone();
                doc.editor
                    .update(cx, |s, cx| s.replace_all("continued 中文", window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let doc = w.tabs[w.active.unwrap()].save.clone();
                assert_eq!(doc.editor.read(cx).value().as_ref(), "continued 中文");
                assert!(doc.conflict.get());
                w.open_conflict_review(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.resolve_review(false, window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(w.ui.conflict_review.is_none());
                let doc = w.tabs[w.active.unwrap()].save.clone();
                assert!(!doc.conflict.get() && !doc.dirty.get() && !doc.saving.get());
                assert_eq!(doc.editor.read(cx).value().as_ref(), "newer external");
                let vault = w.vault.as_ref().unwrap();
                assert!(
                    vault
                        .recoveries()
                        .unwrap()
                        .iter()
                        .any(|e| e.record.draft == "continued 中文")
                );
                for e in vault.history(std::path::Path::new("note.md")).unwrap() {
                    std::fs::remove_file(e.journal).unwrap();
                }
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
