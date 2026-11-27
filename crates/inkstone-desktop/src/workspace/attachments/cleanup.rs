use super::*;

impl Workspace {
    pub(crate) fn open_unused_image_cleanup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_attachment_manager(window, cx);
        if let Some(manager) = &mut self.ui.attachment_manager {
            manager.kind = Some(Kind::Image);
            manager.unused = true;
        }
        cx.notify();
    }

    pub(super) fn request_unused_image_cleanup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(manager) = &self.ui.attachment_manager else {
            return;
        };
        if manager.loading
            || manager.inventory.unreadable > 0
            || !self.file_writes.can_start_exclusive_operation()
        {
            return;
        }
        let entries = manager.unused_images(cx);
        if entries.is_empty() {
            return;
        }
        let bytes: u64 = entries.iter().map(|e| e.bytes).sum();
        let paths = entries
            .iter()
            .take(20)
            .map(|e| e.path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let remaining = if entries.len() > 20 {
            format!("\n另有 {} 张，完整列表见附件管理。", entries.len() - 20)
        } else {
            String::new()
        };
        let detail = format!(
            "将当前筛选列表中的 {} 张图片（{:.1} KiB）移入回收站：\n\n{paths}{remaining}\n\n仅检查笔记中的直接引用；脚本、其他附件或库外应用仍可能使用这些图片。清理前会重新检查，可从“文件恢复”恢复。",
            entries.len(),
            bytes as f64 / 1024.
        );
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let prompt = window.prompt(
            PromptLevel::Warning,
            "清理未使用的图片？",
            Some(&detail),
            &["取消", "移入回收站"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if prompt.await != Ok(1) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation == generation
                    && this.ui.recovery_refresh == request
                    && this.ui.attachment_manager.is_some()
                {
                    this.cleanup_unused_images(entries, window, cx);
                }
            });
        })
        .detach();
    }

    fn cleanup_unused_images(
        &mut self,
        entries: Vec<Attachment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() {
            return;
        }
        if !self.file_writes.can_start_exclusive_operation() {
            self.notifications
                .publish("请等待当前文件操作完成后再清理图片。".into());
            cx.notify();
            return;
        }
        if self
            .tabs
            .iter()
            .any(|tab| self.has_pending_input(tab.id, window, cx))
        {
            self.notifications
                .publish("请完成输入法组词或脚注编辑后再清理图片。".into());
            cx.notify();
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        // An unnamed note has no stable base directory for relative references.
        if self.tabs.iter().any(|tab| {
            tab.image.is_none()
                && tab.path.as_os_str().is_empty()
                && !tab.save.text_snapshot(cx).trim().is_empty()
        }) {
            self.notifications
                .publish("请先保存未命名笔记，再清理图片，以便检查其中的图片引用。".into());
            cx.notify();
            return;
        }
        let drafts = self.file_draft_snapshots(cx);
        let Some(ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        if let Some(manager) = &mut self.ui.attachment_manager {
            manager.loading = true;
            manager.message.clear();
        }
        let task = cx.background_executor().spawn(async move {
            attachments::trash_unused_images(&vault, &entries, &document::owned_drafts(drafts))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation {
                    return;
                }
                let message = match result {
                    Ok(result) => {
                        if !result.trashed.is_empty() {
                            this.changed_paths.extend(result.trashed.iter().cloned());
                            this.structure_changed = true;
                            this.refresh_requested = true;
                            this.refresh(window, cx);
                            this.schedule_auto_sync(true);
                        }
                        let errors = result
                            .errors
                            .iter()
                            .map(|(path, error)| format!("{}：{error}", path.display()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!(
                            "已将 {} 张图片移入回收站，可从“文件恢复”恢复。{}",
                            result.trashed.len(),
                            if errors.is_empty() {
                                String::new()
                            } else {
                                format!("\n{} 张未清理：\n{errors}", result.errors.len())
                            }
                        )
                    }
                    Err(error) => format!("图片未清理：{error}"),
                };
                this.notifications.publish(message.clone());
                if this.ui.recovery_refresh == request {
                    this.refresh_attachments(window, cx);
                    if let Some(manager) = &mut this.ui.attachment_manager {
                        manager.message = message;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn image_cleanup_previews_filtered_files_confirms_and_refreshes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = tempfile::tempdir().unwrap();
        for name in [
            "unused-a.png",
            "unused-b.png",
            "keep.png",
            "used.png",
            "book.pdf",
        ] {
            std::fs::write(root.path().join(name), name).unwrap();
        }
        std::fs::write(root.path().join("note.md"), "![[used.png]]").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.path().into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.execute_command(116, window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let m = w.ui.attachment_manager.as_mut().unwrap();
                assert_eq!(m.kind, Some(Kind::Image));
                assert!(m.unused);
                assert_eq!(m.unused_images(cx).len(), 3);
                m.query
                    .update(cx, |input, cx| input.set_value("unused-", window, cx));
                assert_eq!(m.unused_images(cx).len(), 2);
                w.request_unused_image_cleanup(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let (title, detail) = cx.pending_prompt().unwrap();
        assert!(title.contains("清理未使用的图片"));
        assert!(detail.contains("2 张图片"));
        assert!(detail.contains("unused-a.png") && detail.contains("unused-b.png"));
        assert!(!detail.contains("keep.png") && !detail.contains("book.pdf"));
        cx.simulate_prompt_answer("取消");
        cx.run_until_parked();
        assert!(root.path().join("unused-a.png").exists());
        handle
            .update(cx, |w, window, cx| {
                w.request_unused_image_cleanup(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_prompt_answer("移入回收站");
        cx.run_until_parked();
        for name in ["unused-a.png", "unused-b.png"] {
            assert!(!root.path().join(name).exists());
        }
        for name in ["keep.png", "used.png", "book.pdf"] {
            assert!(root.path().join(name).exists());
        }
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                let m = w.ui.attachment_manager.as_ref().unwrap();
                assert!(!m.loading);
                assert!(m.unused_images(cx).is_empty());
                assert!(m.message.contains("2 张图片移入回收站"));
                assert!(!w.index.files.contains(&PathBuf::from("unused-a.png")));
                let vault = w.vault.as_ref().unwrap();
                let trash = vault.trash_entries().unwrap();
                assert_eq!(trash.len(), 2);
                for entry in trash {
                    vault.restore_trash(&entry).unwrap();
                }
                w.watcher = None;
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("unused-a.png")).unwrap(),
            "unused-a.png"
        );
    }

    #[gpui::test]
    fn image_cleanup_rechecks_edits_and_waits_for_unnamed_notes_and_file_operations(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("image.png"), "image").unwrap();
        std::fs::write(root.path().join("note.md"), "note").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.path().into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.open_note("note.md".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_unused_image_cleanup(window, cx))
            .unwrap();
        cx.run_until_parked();
        let entries = handle
            .update(cx, |w, window, cx| {
                let entries = w.ui.attachment_manager.as_ref().unwrap().unused_images(cx);
                let ticket = w.file_writes.begin();
                w.cleanup_unused_images(entries.clone(), window, cx);
                assert_eq!(w.file_writes.pending(), 1);
                assert!(w.notifications.text().contains("等待当前文件操作"));
                w.file_writes.finish(ticket);
                w.request_unused_image_cleanup(window, cx);
                // The confirmation must use the latest editor snapshot.
                w.tabs[0].save.editor.update(cx, |editor, cx| {
                    editor.replace_all("![[image.png]]", window, cx)
                });
                entries
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_prompt_answer("移入回收站");
        cx.run_until_parked();
        assert!(root.path().join("image.png").exists());
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(w.notifications.text().contains("已被笔记引用"));
                w.new_blank(window, cx);
                let blank = w
                    .tabs
                    .iter()
                    .find(|tab| tab.path.as_os_str().is_empty())
                    .unwrap();
                blank.save.editor.update(cx, |editor, cx| {
                    editor.replace_all("![draft](image.png)", window, cx)
                });
                w.cleanup_unused_images(entries, window, cx);
                assert!(w.notifications.text().contains("先保存未命名笔记"));
                assert_eq!(w.file_writes.pending(), 0);
                w.watcher = None;
            })
            .unwrap();
        assert!(root.path().join("image.png").exists());
    }

    #[gpui::test]
    fn image_cleanup_ignores_confirmation_after_panel_is_replaced(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("image.png"), "image").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.path().into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_unused_image_cleanup(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.request_unused_image_cleanup(window, cx);
                w.open_attachment_manager(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_prompt_answer("移入回收站");
        cx.run_until_parked();
        assert!(root.path().join("image.png").exists());
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.file_writes.pending(), 0);
                w.watcher = None;
            })
            .unwrap();
    }
}
