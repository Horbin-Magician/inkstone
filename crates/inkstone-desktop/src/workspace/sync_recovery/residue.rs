//! Explicit descriptor preservation; no automatic archive or note save.
use super::missing::text;
use super::*;

impl Workspace {
    pub(super) fn can_retain_sync_residue(&self) -> bool {
        let state = &self.ui.sync_recovery;
        self.vault.is_some()
            && state.generation == self.generation
            && !state.loading
            && !state.cleaning
            && !state.retaining
            && self.file_writes.can_start_exclusive_operation()
    }
    pub(super) fn request_sync_residue_retention(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.can_retain_sync_residue() {
            self.ui.sync_recovery.retain_confirmation = self
                .ui
                .sync_recovery
                .inventory
                .missing_payloads
                .get(index)
                .cloned();
            cx.notify();
        }
    }
    fn execute_sync_residue_retention(&mut self, cx: &mut Context<Self>) {
        if !self.can_retain_sync_residue() {
            return;
        }
        let Some(expected) = self.ui.sync_recovery.retain_confirmation.clone() else {
            return;
        };
        if !self
            .ui
            .sync_recovery
            .inventory
            .missing_payloads
            .contains(&expected)
        {
            return;
        }
        let Some(ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        let vault = self.vault.clone().unwrap();
        let generation = self.generation;
        let request = self.ui.sync_recovery.request;
        let state = &mut self.ui.sync_recovery;
        state.retain_confirmation = None;
        state.preview = None;
        state.cleanup_confirmation = None;
        state.retaining = true;
        state.message = "正在核对并保留描述文件……".into();
        let task = cx
            .background_executor()
            .spawn(async move { recovery::residue::retain(&vault, &expected) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation || this.ui.sync_recovery.request != request || !this.ui.sync_recovery.retaining { return; }
                this.ui.sync_recovery.retaining = false;
                this.refresh_sync_recovery(cx);
                let message = match result {
                    Ok(record) => format!("描述文件已保留归档：{}。原字节及占用保留，未恢复缺失正文。", record.metadata.display()),
                    Err(error) => format!("保留归档未完成：{error}。请检查刷新后的列表，记录可能已移动；不要手动覆盖。"),
                };
                this.ui.sync_recovery.message = message.clone();
                this.notifications.publish(message);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(super) fn sync_residue_panel(&self, cx: &Context<Self>) -> AnyElement {
        let state = &self.ui.sync_recovery;
        let mut panel = div().flex().flex_col().gap_2().min_w_0();
        if let Some(record) = &state.retain_confirmation {
            panel = panel.child(text("sync-residue-warning", format!(
                "确认保留归档描述文件 {}（原路径 {}，{} 字节）？请先停止其他用户、设备及外部工具修改此库。此操作不会找回正文；只把描述文件改名为 .retained，原字节和占用继续保留。该记录不再阻止其他备份清理；正文重现或记录变化则拒绝处理。", record.metadata.display(), record.original.display(), record.metadata_bytes)))
                .child(FocusReveal::new("sync-residue-confirm-focus", &self.ui.recovery_scroll,
                    Button::new("sync-residue-confirm").label("确认保留归档").disabled(!self.can_retain_sync_residue())
                        .on_click(cx.listener(|this, _, window, cx| { this.execute_sync_residue_retention(cx); window.focus(&this.ui.modal_focus,cx); }))))
                .child(FocusReveal::new("sync-residue-cancel-focus", &self.ui.recovery_scroll,
                    Button::new("sync-residue-cancel").label("取消归档")
                        .on_click(cx.listener(|this, _, window, cx| { this.ui.sync_recovery.retain_confirmation=None; window.focus(&this.ui.modal_focus,cx); cx.notify(); }))));
        }
        let records = &state.inventory.retained_descriptors;
        let Some(vault) = self.vault.as_ref().filter(|_| !records.is_empty()) else {
            return panel.into_any_element();
        };
        let pages = records.len().div_ceil(RECORDS_PER_PAGE);
        let page = state.retained_page.min(pages - 1);
        let focus = state
            .retained_focus
            .get_or_init(|| cx.focus_handle())
            .clone();
        panel.child(text("sync-retained-help", format!("已保留 {} 条描述归档；仍计入备份占用，不自动删除。归档没有正文，不能直接恢复笔记。第 {} / {pages} 页。", records.len(),page+1)))
            .child(div().track_focus(&focus).flex().flex_wrap().gap_2().children([false,true].into_iter().map(|next| {
                let focus=focus.clone();
                let id=("sync-retained-page",usize::from(next));
                div().debug_selector(move || format!("sync-retained-page-{next}"))
                    .child(FocusReveal::new((ElementId::from(id),"focus"), &self.ui.recovery_scroll,
                        Button::new(id).label(if next {"归档下一页"} else {"归档上一页"})
                            .disabled(if next {page+1>=pages} else {page==0})
                            .on_click(cx.listener(move |this,_,window,cx| {
                                let state=&mut this.ui.sync_recovery;
                                let last=state.inventory.retained_descriptors.len().saturating_sub(1)/RECORDS_PER_PAGE;
                                state.retained_page=if next {state.retained_page.saturating_add(1).min(last)} else {state.retained_page.saturating_sub(1)};
                                window.focus(&focus,cx);cx.notify();
                            }))))
            })))
            .children(records.iter().enumerate().skip(page*RECORDS_PER_PAGE).take(RECORDS_PER_PAGE).map(|(i,record)| {
                let description=format!("保留描述归档：{} · 原路径：{} · 原描述路径：{} · 预期正文：{} · {} 字节 · {}",
                    record.metadata.display(),record.original.display(),record.original_metadata.display(),record.backup.display(),record.bytes,
                    chrono::DateTime::<chrono::Local>::from(record.modified).format("%Y-%m-%d %H:%M:%S"));
                let path=vault.root.join(&record.metadata);
                div().flex().flex_col().gap_1().min_w_0().debug_selector(move || format!("sync-retained-record-{i}"))
                    .child(text(("sync-retained-text",i),description.clone()))
                    .child(FocusReveal::new(("sync-retained-reveal-focus",i), &self.ui.recovery_scroll,
                        Button::new(("sync-retained-reveal",i)).label("定位归档描述")
                            .accessibility_label(description).on_click(move |_,_,cx| cx.reveal_path(&path))))
            })).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    fn exercise(cx: &mut TestAppContext, stale: bool, changed: bool) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-residue-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("note.md"), b"current").unwrap();
        let metadata = vault.root.join(".inkstone-sync-lost.backup.json");
        let bytes=serde_json::to_vec(&serde_json::json!({"original":"note.md","backup":".inkstone-sync-lost.backup","sha256":"0".repeat(64),"protected":true})).unwrap();
        std::fs::write(&metadata, &bytes).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab("note.md".into(), Some("current".into()), false, window, cx);
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |editor, cx| editor.set_value("未保存 📝", window, cx));
                w.flush_document_views(window, cx);
                w.refresh_sync_recovery(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.execute_sync_residue_retention(cx);
                assert_eq!(w.file_writes.pending(), 0);
                let ticket = w.file_writes.begin();
                w.request_sync_residue_retention(0, cx);
                assert!(w.ui.sync_recovery.retain_confirmation.is_none());
                w.file_writes.finish(ticket);
                w.request_sync_residue_retention(0, cx);
                assert!(w.ui.sync_recovery.retain_confirmation.is_some());
                w.refresh_sync_recovery(cx);
                assert!(w.ui.sync_recovery.retain_confirmation.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.request_sync_residue_retention(0, cx);
                if changed {
                    std::fs::write(vault.root.join(".inkstone-sync-lost.backup"), b"returned")
                        .unwrap();
                }
                w.execute_sync_residue_retention(cx);
                w.execute_sync_residue_retention(cx);
                w.refresh_sync_recovery(cx);
                assert!(w.ui.sync_recovery.retaining);
                assert_eq!(w.file_writes.pending(), 1);
                assert!(w.file_writes.operation_active());
                if stale {
                    w.generation = w.generation.wrapping_add(1);
                    w.ui.sync_recovery = State {
                        generation: w.generation,
                        message: "new context".into(),
                        ..Default::default()
                    };
                }
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(!w.ui.sync_recovery.retaining);
                assert!(w.tabs[0].save.persistence.is_dirty());
                assert_eq!(w.tabs[0].save.editor.read(cx).value(), "未保存 📝");
                if stale {
                    assert_eq!(w.ui.sync_recovery.message, "new context");
                } else if changed {
                    assert!(w.ui.sync_recovery.message.contains("未完成"));
                    assert_eq!(w.ui.sync_recovery.inventory.entries.len(), 1);
                    assert!(w.ui.sync_recovery.inventory.retained_descriptors.is_empty());
                } else {
                    assert!(w.ui.sync_recovery.message.contains("已保留归档"));
                    assert!(w.ui.sync_recovery.inventory.missing_payloads.is_empty());
                    assert_eq!(w.ui.sync_recovery.inventory.retained_descriptors.len(), 1);
                    assert_eq!(
                        w.ui.sync_recovery.inventory.stored_bytes,
                        bytes.len() as u64
                    );
                }
            })
            .unwrap();
        assert_eq!(
            std::fs::read(vault.root.join("note.md")).unwrap(),
            b"current"
        );
        let retained = metadata.with_file_name(".inkstone-sync-lost.backup.json.retained");
        assert_eq!(
            std::fs::read(if changed { metadata } else { retained }).unwrap(),
            bytes
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn explicit_retention_refreshes_inventory_without_saving_notes(cx: &mut TestAppContext) {
        exercise(cx, false, false);
    }
    #[gpui::test]
    fn reappearing_body_rejects_archive_and_refreshes_recovery(cx: &mut TestAppContext) {
        exercise(cx, false, true);
    }
    #[gpui::test]
    fn stale_retention_callback_only_releases_its_ticket(cx: &mut TestAppContext) {
        exercise(cx, true, false);
    }
}
