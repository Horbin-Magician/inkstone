//! Sync backups inside the shared file recovery entry point.
use super::focus_reveal::FocusReveal;
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::sync::recovery::{self, Inventory, retention};

const RECORDS_PER_PAGE: usize = 5;

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    request: u64,
    preview: Option<retention::Preview>,
    cleanup_confirmation: Option<retention::Preview>,
    cleaning: bool,
    cancellation: Option<std::sync::Arc<inkstone_core::vault::sync::Cancellation>>,
    page: usize,
    pager_focus: std::cell::OnceCell<FocusHandle>,
    inventory: Inventory,
    loading: bool,
    message: String,
}
impl Workspace {
    pub(super) fn refresh_sync_recovery(&mut self, cx: &mut Context<Self>) {
        self.load_sync_recovery(false, cx);
    }
    fn preview_sync_retention(&mut self, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() || self.file_writes.pending() > 0 {
            return;
        }
        self.load_sync_recovery(true, cx);
    }
    fn load_sync_recovery(&mut self, preview: bool, cx: &mut Context<Self>) {
        if self.ui.sync_recovery.cleaning {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let request = self.ui.sync_recovery.request.wrapping_add(1);
        self.ui.sync_recovery = State {
            request,
            generation: self.generation,
            loading: true,
            ..Default::default()
        };
        let generation = self.generation;
        let recovery_request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let inventory = recovery::inventory(&vault)?;
            let preview = if preview {
                Some(retention::preview(&inventory, 1, &Default::default())?)
            } else {
                None
            };
            anyhow::Ok((inventory, preview))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.ui.recovery_refresh != recovery_request
                    || this.ui.sync_recovery.request != request
                {
                    return;
                }
                let state = &mut this.ui.sync_recovery;
                state.loading = false;
                match result {
                    Ok((inventory, preview)) => {
                        state.inventory = inventory;
                        state.preview = preview;
                    }
                    Err(error) => state.message = format!("无法读取同步备份：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn can_clean_sync_backups(&self) -> bool {
        let state = &self.ui.sync_recovery;
        self.vault.is_some()
            && state.generation == self.generation
            && !state.loading
            && !state.cleaning
            && self.file_writes.can_start_exclusive_operation()
            && state
                .preview
                .as_ref()
                .is_some_and(|preview| preview.candidates > 0)
    }
    fn request_sync_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.can_clean_sync_backups() {
            self.ui.sync_recovery.cleanup_confirmation = self.ui.sync_recovery.preview.clone();
            cx.notify();
        }
    }
    fn execute_sync_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.can_clean_sync_backups() {
            return;
        }
        let Some(preview) = self.ui.sync_recovery.cleanup_confirmation.take() else {
            return;
        };
        if self.ui.sync_recovery.preview.as_ref() != Some(&preview) {
            return;
        }
        let Some(ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        let vault = self.vault.clone().unwrap();
        let generation = self.generation;
        let cancellation = std::sync::Arc::new(inkstone_core::vault::sync::Cancellation::default());
        self.ui.sync_recovery.cancellation = Some(cancellation.clone());
        self.ui.sync_recovery.cleaning = true;
        self.ui.sync_recovery.preview = None;
        self.ui.sync_recovery.message = "正在核验并清理同步备份……".into();
        let task = cx.background_executor().spawn(async move {
            recovery::cleanup::prepare(&vault, &preview, &Default::default(), &cancellation)?
                .execute(&cancellation)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation { return; }
                this.ui.sync_recovery.cleaning = false;
                this.refresh_sync_recovery(cx);
                let message = match result {
                    Ok(report) => format!("已清理 {} 条同步备份，正文共 {} 字节。", report.removed, report.bytes),
                    Err(error) if recovery::cleanup::is_cancelled(&error) => "同步备份清理已停止。已完成的删除不会撤销，请检查刷新后的列表；剩余受保护记录仍可恢复为副本。".into(),
                    Err(error) => format!("同步备份清理未完成：{error}。部分候选可能已清理，请检查刷新后的列表；剩余受保护记录仍可恢复为副本。"),
                };
                this.ui.sync_recovery.message = message.clone();
                this.notifications.publish(message);
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn stop_sync_cleanup(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.ui.sync_recovery;
        if state.generation != self.generation || !state.cleaning {
            return;
        }
        if let Some(cancellation) = &state.cancellation {
            cancellation.request();
            state.message = "已请求停止，正在等待当前步骤结束；已完成的删除不会撤销。".into();
            // Keep the exclusive ticket until the background task actually exits.
            cx.notify();
        }
    }
    fn restore_sync_backup(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.generation != self.ui.sync_recovery.generation
            || self.file_writes.operation_active()
            || self.file_writes.pending() > 0
            || self.ui.sync_recovery.loading
        {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(entry) = self.ui.sync_recovery.inventory.entries.get(index).cloned() else {
            return;
        };
        let reserved = self
            .tabs
            .iter()
            .map(|tab| tab.path.clone())
            .collect::<Vec<_>>();
        let generation = self.generation;
        let write_ticket = self.file_writes.begin();
        self.ui.sync_recovery.preview = None;
        self.ui.sync_recovery.cleanup_confirmation = None;
        self.ui.sync_recovery.message = "正在恢复副本……".into();
        let task = cx
            .background_executor()
            .spawn(async move { recovery::restore_copy(&vault, &entry, &reserved) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(write_ticket);
                if this.generation != generation {
                    return;
                }
                let message = match result {
                    Ok(restored) => {
                        this.changed_paths.insert(restored.relative.clone());
                        this.refresh_requested = true;
                        this.schedule_auto_sync(true);
                        format!(
                            "已恢复副本：{}。原文件与备份均保留。{}",
                            restored.relative.display(),
                            if restored.matches_baseline {
                                ""
                            } else {
                                "备份与同步前基线不同，已保留实际内容。"
                            }
                        )
                    }
                    Err(error) => format!("恢复失败，请刷新后重试：{error}"),
                };
                this.ui.sync_recovery.message = message.clone();
                this.notifications.publish(message);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn sync_recovery_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.sync_recovery;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child("同步备份 · 恢复为副本")
            .child(div().text_sm().whitespace_normal().child(format!(
                "{} 条可恢复记录 · 正文 {:.1} MiB · 备份及描述文件共 {} 字节 · 不自动清理；恢复后保留备份",
                state.inventory.entries.len(),
                state.inventory.bytes as f64 / 1048576.,
                state.inventory.stored_bytes
            )))
            .when(state.inventory.unindexed_files > 0, |s| {
                s.child(div().whitespace_normal().child(format!(
                    "另有 {} 个备份文件缺少可用恢复记录（{} 字节），已计入占用并保留；不代表可安全清理。",
                    state.inventory.unindexed_files, state.inventory.unindexed_bytes
                )))
            })
            .when(state.inventory.unmeasured_files > 0, |s| {
                s.child(div().whitespace_normal().child(format!(
                    "{} 个备份或描述路径无法安全统计，显示的文件字节数不完整。",
                    state.inventory.unmeasured_files
                )))
            })
            .when(state.loading, |s| s.child("正在读取同步备份……"))
            .when(!state.loading && state.inventory.entries.is_empty(), |s| {
                s.child("暂无可直接恢复的同步备份记录")
            })
            .when(state.inventory.unreadable > 0, |s| {
                s.child(format!(
                    "{} 条记录或目录无法读取，其他记录仍可恢复。",
                    state.inventory.unreadable
                ))
            })
            .when(!state.message.is_empty(), |s| {
                s.child(state.message.clone())
            })
            .when(state.cleaning, |panel| panel.child(FocusReveal::new(
                "sync-cleanup-stop-focus", &self.ui.recovery_scroll,
                Button::new("sync-cleanup-stop")
                    .label(if state.cancellation.as_ref().is_some_and(|c| c.is_requested()) {
                        "正在停止清理……"
                    } else { "停止后续清理" })
                    .disabled(state.cancellation.as_ref().is_none_or(|c| c.is_requested()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.stop_sync_cleanup(cx);
                        window.focus(&this.ui.modal_focus, cx);
                    })),
            )))
            .child(FocusReveal::new(
                "sync-retention-preview-focus",
                &self.ui.recovery_scroll,
                Button::new("sync-retention-preview")
                    .label("预览保留规则（每篇保留最新 1 份）")
                    .disabled(state.loading || self.file_writes.operation_active() || self.file_writes.pending() > 0)
                    .on_click(cx.listener(|this, _, _, cx| this.preview_sync_retention(cx))),
            ))
            .when_some(state.preview.as_ref(), |panel, preview| {
                panel.child(div().whitespace_normal().child(format!(
                    "保留预览：{} 条待核验候选，正文共 {} 字节。每个原路径保留最新 1 份及同时间记录；清单不完整时全部保留。预览不会删除；清理需要另行确认并校验正文。",
                    preview.candidates, preview.candidate_bytes
                )))
            })
            .when(state.preview.as_ref().is_some_and(|p| p.candidates > 0), |panel| panel
                .child(FocusReveal::new("sync-cleanup-review-focus", &self.ui.recovery_scroll,
                    Button::new("sync-cleanup-review").label("清理预览中的同步备份……")
                        .disabled(!self.can_clean_sync_backups())
                        .on_click(cx.listener(|this, _, _, cx| this.request_sync_cleanup(cx))))))
            .when_some(state.cleanup_confirmation.as_ref(), |panel, preview| panel
                .child(div().whitespace_normal().child(format!(
                    "确认删除 {} 条旧同步备份（正文 {} 字节）？此操作无法撤销。每篇最新备份及受保护记录保留，原笔记不受影响。",
                    preview.candidates, preview.candidate_bytes)))
                .child(div().whitespace_normal().child("仅在已停止其他用户、设备或云盘工具修改此笔记库后执行；只支持可确认的本地存储。核验失败会停止后续清理。"))
                .child(FocusReveal::new("sync-cleanup-confirm-focus", &self.ui.recovery_scroll,
                    Button::new("sync-cleanup-confirm").label("确认清理旧同步备份")
                        .disabled(!self.can_clean_sync_backups())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.execute_sync_cleanup(cx);
                            window.focus(&this.ui.modal_focus, cx);
                        }))))
                .child(div().debug_selector(|| "sync-cleanup-cancel-row".into()).child(FocusReveal::new("sync-cleanup-cancel-focus", &self.ui.recovery_scroll,
                    Button::new("sync-cleanup-cancel").label("取消清理")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.ui.sync_recovery.cleanup_confirmation = None;
                            // The focused button is removed by this action.
                            window.focus(&this.ui.modal_focus, cx);
                            cx.notify();
                        }))))))
            .when(!state.inventory.entries.is_empty(), |panel| {
                let focus = state.pager_focus.get_or_init(|| cx.focus_handle()).clone();
                let pages = state.inventory.entries.len().div_ceil(RECORDS_PER_PAGE);
                panel
                    .child(
                        div()
                            .track_focus(&focus)
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(format!(
                                "同步备份：第 {} / {pages} 页，每页最多 {RECORDS_PER_PAGE} 条",
                                state.page + 1
                            ))
                            .children([false, true].into_iter().map(|next| {
                                let focus = focus.clone();
                                let id = ("sync-recovery-page", usize::from(next));
                                div()
                                    .debug_selector(move || format!("sync-recovery-page-{next}"))
                                    .child(FocusReveal::new(
                                        (ElementId::from(id), "focus"),
                                        &self.ui.recovery_scroll,
                                        Button::new(id)
                                            .label(if next {
                                                "同步备份下一页"
                                            } else {
                                                "同步备份上一页"
                                            })
                                            .disabled(if next {
                                                state.page + 1 >= pages
                                            } else {
                                                state.page == 0
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                let state = &mut this.ui.sync_recovery;
                                                let last =
                                                    state.inventory.entries.len().saturating_sub(1)
                                                        / RECORDS_PER_PAGE;
                                                state.page = if next {
                                                    state.page.saturating_add(1).min(last)
                                                } else {
                                                    state.page.saturating_sub(1)
                                                };
                                                // A boundary button becomes disabled; retain a stable navigation origin.
                                                window.focus(&focus, cx);
                                                cx.notify();
                                            })),
                                    ))
                            })),
                    )
                    .children(
                        state
                            .inventory
                            .entries
                            .iter()
                            .enumerate()
                            .skip(state.page * RECORDS_PER_PAGE)
                            .take(RECORDS_PER_PAGE)
                            .map(|(i, entry)| {
                                let description = format!(
                                    "{} · 原路径：{}{}",
                                    super::recovery_metadata::Metadata {
                                        source: super::recovery_metadata::Source::SyncBackup,
                                        modified: Some(entry.modified),
                                        bytes: Some(entry.bytes),
                                    }
                                    .label(),
                                    entry.original.display(),
                                    if entry.protected { " · 受保护备份" } else { "" },
                                );
                                div()
                                    .debug_selector(move || format!("sync-recovery-record-{i}"))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .flex_shrink_0()
                                    .child(div().whitespace_normal().child(description.clone()))
                                    .when_some(state.preview.as_ref().and_then(|p| p.records.get(i)), |row, record| {
                                        row.child(div().whitespace_normal().child(match record.decision {
                                            retention::Decision::Candidate => "待核验候选：旧版本，尚未核验正文完整性",
                                            retention::Decision::Recent => "保留：该原路径的最新记录（含同时间记录）",
                                            retention::Decision::Protected => "保留：受保护的恢复记录",
                                            retention::Decision::InUse => "保留：正在查看或恢复",
                                            retention::Decision::IncompleteInventory => "保留：备份清单不完整",
                                        }))
                                    })
                                    .child(FocusReveal::new(
                                        (ElementId::from(("restore-sync-backup", i)), "focus"),
                                        &self.ui.recovery_scroll,
                                        Button::new(("restore-sync-backup", i))
                                            .compact()
                                            .label("恢复为副本")
                                            .accessibility_label(format!(
                                                "恢复为副本：{description}"
                                            ))
                                            .disabled(
                                                self.file_writes.operation_active()
                                                    || self.file_writes.pending() > 0
                                                    || state.loading,
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.restore_sync_backup(i, cx)
                                            })),
                                    ))
                            }),
                    )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    fn fixture() -> (PathBuf, Vault) {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "inkstone-recovery-hub-{}-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("note.md"), "current").unwrap();
        std::fs::write(vault.root.join(".inkstone-sync-test.backup"), "backup").unwrap();
        std::fs::write(vault.root.join(".inkstone-sync-test.backup.json"), serde_json::to_vec(&serde_json::json!({"original":"note.md", "backup":".inkstone-sync-test.backup", "sha256":"0".repeat(64)})).unwrap()).unwrap();
        std::fs::write(vault.root.join("deleted.md"), "deleted").unwrap();
        vault
            .trash_note(std::path::Path::new("deleted.md"), "deleted")
            .unwrap();
        (root, vault)
    }
    fn cleanup_fixture() -> (PathBuf, Vault) {
        use sha2::{Digest, Sha256};
        let (root, vault) = fixture();
        for (name, body, seconds) in [("test", "backup", 2), ("old", "older", 1)] {
            let file = format!(".inkstone-sync-{name}.backup");
            let path = vault.root.join(&file);
            std::fs::write(&path, body).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new()
                        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seconds)),
                )
                .unwrap();
            std::fs::write(
                vault.root.join(format!("{file}.json")),
                serde_json::to_vec(&serde_json::json!({"original":"note.md", "backup":file,
                    "sha256":format!("{:x}", Sha256::digest(body.as_bytes()))}))
                .unwrap(),
            )
            .unwrap();
        }
        (root, vault)
    }

    #[gpui::test]
    fn cancelling_confirmation_keeps_keyboard_focus_in_recovery(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(|cx| cx.bind_keys([KeyBinding::new("escape", ClosePalette, None)]));
        let (root, vault) = cleanup_fixture();
        let mut workspace = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        visual.run_until_parked();
        visual.update(|window, cx| {
            workspace.update(cx, |w, cx| {
                w.vault = Some(vault.clone());
                w.ui.trash_open = true;
                w.ui.sync_recovery.generation = w.generation;
                w.ui.sync_recovery.inventory = recovery::inventory(&vault).unwrap();
                w.ui.sync_recovery.preview = Some(
                    retention::preview(&w.ui.sync_recovery.inventory, 1, &Default::default())
                        .unwrap(),
                );
                w.request_sync_cleanup(cx);
                window.focus(&w.ui.modal_focus, cx);
            })
        });
        fn key(visual: &mut VisualTestContext, value: &str) {
            let keystroke = Keystroke::parse(value).unwrap();
            visual.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(KeyUpEvent { keystroke });
            for _ in 0..2 {
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
        }
        let mut reached = false;
        for _ in 0..20 {
            key(visual, "tab");
            if let (Some(row), Some(target)) = (
                visual.debug_bounds("sync-cleanup-cancel-row"),
                visual.debug_bounds("focus-revealed-control"),
            ) && target.top() >= row.top()
                && target.bottom() <= row.bottom()
            {
                reached = true;
                break;
            }
        }
        assert!(reached);
        key(visual, "enter");
        visual.update(|window, cx| {
            let w = workspace.read(cx);
            assert!(w.ui.sync_recovery.cleanup_confirmation.is_none());
            assert!(
                w.ui.modal_focus.contains_focused(window, cx),
                "cancelled control left focus outside recovery"
            );
            assert_eq!(w.file_writes.pending(), 0);
        });
        key(visual, "escape");
        visual.update(|_, cx| assert!(!workspace.read(cx).ui.trash_open));
        assert!(vault.root.join(".inkstone-sync-old.backup").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn cleanup_requires_current_confirmation_and_preserves_dirty_notes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = cleanup_fixture();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab("note.md".into(), Some("current".into()), false, window, cx);
                w.tabs[0].save.editor.update(cx, |editor, cx| {
                    editor.set_value("unsaved 中文", window, cx)
                });
                w.flush_document_views(window, cx);
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.execute_sync_cleanup(cx);
                assert_eq!(w.file_writes.pending(), 0);
                let ticket = w.file_writes.begin();
                w.request_sync_cleanup(cx);
                assert!(w.ui.sync_recovery.cleanup_confirmation.is_none());
                w.file_writes.finish(ticket);
                w.request_sync_cleanup(cx);
                assert!(w.ui.sync_recovery.cleanup_confirmation.is_some());
                w.refresh_sync_recovery(cx);
                assert!(w.ui.sync_recovery.cleanup_confirmation.is_none());
                w.execute_sync_cleanup(cx);
                assert_eq!(w.file_writes.pending(), 0);
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.request_sync_cleanup(cx);
                w.execute_sync_cleanup(cx);
                w.execute_sync_cleanup(cx);
                w.restore_sync_backup(0, cx);
                assert_eq!(w.file_writes.pending(), 1);
                assert!(w.file_writes.operation_active());
                assert!(w.ui.sync_recovery.cleaning);
                assert!(w.ui.sync_recovery.preview.is_none());
                w.refresh_sync_recovery(cx);
                assert!(w.ui.sync_recovery.cleaning);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(!w.ui.sync_recovery.cleaning);
                assert!(!w.ui.sync_recovery.loading);
                assert!(
                    w.ui.sync_recovery.message.contains("已清理 1 条"),
                    "{}",
                    w.ui.sync_recovery.message
                );
                assert_eq!(w.ui.sync_recovery.inventory.entries.len(), 1);
                assert!(w.tabs[0].save.persistence.is_dirty());
                assert_eq!(w.tabs[0].save.editor.read(cx).value(), "unsaved 中文");
            })
            .unwrap();
        assert_eq!(
            std::fs::read(vault.root.join("note.md")).unwrap(),
            b"current"
        );
        assert_eq!(
            std::fs::read(vault.root.join(".inkstone-sync-test.backup")).unwrap(),
            b"backup"
        );
        assert!(!vault.root.join(".inkstone-sync-old.backup").exists());
        assert!(!vault.root.join("note 同步恢复.md").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn stopping_cleanup_keeps_ticket_until_completion_and_preserves_candidates(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (root, vault) = cleanup_fixture();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, cx| {
                w.vault = Some(vault.clone());
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.request_sync_cleanup(cx);
                w.execute_sync_cleanup(cx);
                let cancellation = w.ui.sync_recovery.cancellation.as_ref().unwrap().clone();
                assert!(!cancellation.is_requested());
                w.stop_sync_cleanup(cx);
                w.stop_sync_cleanup(cx);
                assert!(cancellation.is_requested());
                assert!(w.ui.sync_recovery.cleaning);
                assert!(w.file_writes.operation_active());
                assert_eq!(w.file_writes.pending(), 1);
                w.restore_sync_backup(0, cx);
                w.execute_sync_cleanup(cx);
                assert_eq!(w.file_writes.pending(), 1);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(!w.ui.sync_recovery.cleaning);
                assert!(w.ui.sync_recovery.cancellation.is_none());
                assert_eq!(w.ui.sync_recovery.inventory.entries.len(), 2);
                assert!(w.ui.sync_recovery.message.contains("清理已停止"));
                let message = w.ui.sync_recovery.message.clone();
                w.stop_sync_cleanup(cx);
                assert_eq!(w.ui.sync_recovery.message, message);
            })
            .unwrap();
        assert_eq!(
            std::fs::read(vault.root.join(".inkstone-sync-old.backup")).unwrap(),
            b"older"
        );
        assert_eq!(
            std::fs::read(vault.root.join("note.md")).unwrap(),
            b"current"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn cleanup_stale_disk_preview_fails_and_refreshes_without_deletion(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = cleanup_fixture();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, cx| {
                w.vault = Some(vault.clone());
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.request_sync_cleanup(cx);
                std::fs::write(vault.root.join(".inkstone-sync-test.backup"), b"changed").unwrap();
                w.execute_sync_cleanup(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(w.ui.sync_recovery.message.contains("清理未完成"));
                assert_eq!(w.ui.sync_recovery.inventory.entries.len(), 2);
                assert!(w.ui.sync_recovery.preview.is_none());
                assert!(w.ui.sync_recovery.cleanup_confirmation.is_none());
            })
            .unwrap();
        assert_eq!(
            std::fs::read(vault.root.join(".inkstone-sync-old.backup")).unwrap(),
            b"older"
        );
        assert_eq!(
            std::fs::read(vault.root.join("note.md")).unwrap(),
            b"current"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn old_cleanup_completion_releases_ticket_without_overwriting_new_vault_state(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (root, vault) = cleanup_fixture();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, cx| {
                w.vault = Some(vault.clone());
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.request_sync_cleanup(cx);
                w.execute_sync_cleanup(cx);
                assert!(!w.file_writes.can_start_exclusive_operation());
                // Normal switching waits for the ticket. Simulate stale completion
                // explicitly to verify the final callback's generation boundary.
                w.generation = w.generation.wrapping_add(1);
                w.ui.sync_recovery = State {
                    generation: w.generation,
                    message: "new vault".into(),
                    ..Default::default()
                };
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.file_writes.pending(), 0);
                assert_eq!(w.ui.sync_recovery.message, "new vault");
                assert!(w.ui.sync_recovery.inventory.entries.is_empty());
            })
            .unwrap();
        assert!(!vault.root.join(".inkstone-sync-old.backup").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn hub_loads_sync_backups_and_restores_without_overwriting(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = fixture();
        let window = cx.add_window(Workspace::new);
        window
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.execute_command(16, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |w, _, cx| {
                assert!(w.ui.trash_open);
                assert_eq!(w.ui.sync_recovery.inventory.entries.len(), 1);
                assert!(!w.ui.sync_recovery.loading);
                let pending = w.file_writes.begin();
                w.restore_sync_backup(0, cx);
                assert_eq!(w.file_writes.pending(), 1);
                w.file_writes.finish(pending);
                w.restore_sync_backup(0, cx);
                w.restore_sync_backup(0, cx);
                assert_eq!(w.file_writes.pending(), 1);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |w, _, _| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(
                    w.ui.sync_recovery.message.contains("已恢复副本"),
                    "{}",
                    w.ui.sync_recovery.message
                );
                assert_eq!(w.ui.trash.len(), 1);
                assert_eq!(w.ui.trash_metadata[&w.ui.trash[0].stored].bytes, Some(7));
                assert!(w.ui.sync_recovery.message.contains("与同步前基线不同"));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "current"
        );
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note 同步恢复.md")).unwrap(),
            "backup"
        );
        assert!(!vault.root.join("note 同步恢复 2.md").exists());
        assert!(vault.root.join(".inkstone-sync-test.backup").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn sync_backup_pages_keep_every_record_keyboard_reachable(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = fixture();
        let entry = recovery::inventory(&vault).unwrap().entries.remove(0);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.ui.trash_open = true;
                w.ui.sync_recovery.inventory.entries = (0..12)
                    .map(|i| {
                        let mut entry = entry.clone();
                        entry.original = PathBuf::from(format!("较长的恢复路径/第{i}篇笔记.md"));
                        entry
                    })
                    .collect();
                window.focus(&w.ui.modal_focus, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        fn key(v: &mut VisualTestContext, key: &str) {
            let keystroke = Keystroke::parse(key).unwrap();
            v.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            v.simulate_event(KeyUpEvent { keystroke });
            for _ in 0..2 {
                v.update(|w, cx| w.draw(cx).clear(cx));
            }
        }
        let selectors = [
            "sync-recovery-record-0",
            "sync-recovery-record-1",
            "sync-recovery-record-2",
            "sync-recovery-record-3",
            "sync-recovery-record-4",
            "sync-recovery-record-5",
            "sync-recovery-record-6",
            "sync-recovery-record-7",
            "sync-recovery-record-8",
            "sync-recovery-record-9",
            "sync-recovery-record-10",
            "sync-recovery-record-11",
        ];
        for rem in [16., 24.] {
            visual.update(|w, _| w.set_rem_size(px(rem)));
            handle
                .update(&mut visual, |w, window, cx| {
                    w.ui.sync_recovery.page = 0;
                    window.focus(&w.ui.modal_focus, cx);
                })
                .unwrap();
            for (step, page) in [0, 1, 2, 1, 0].into_iter().enumerate() {
                let navigation = if step < 2 { "tab" } else { "shift-tab" };
                let mut reached = std::collections::BTreeSet::new();
                for _ in 0..30 {
                    key(&mut visual, navigation);
                    if let Some(target) = visual.debug_bounds("focus-revealed-control") {
                        handle
                            .update(&mut visual, |w, _, _| {
                                let viewport = w.ui.recovery_scroll.bounds();
                                assert!(
                                    target.top() >= viewport.top()
                                        && target.bottom() <= viewport.bottom(),
                                    "rem={rem}: {target:?} outside {viewport:?}"
                                );
                            })
                            .unwrap();
                        for (i, selector) in selectors.iter().enumerate() {
                            if let Some(row) = visual.debug_bounds(selector)
                                && target.top() >= row.top()
                                && target.bottom() <= row.bottom()
                            {
                                reached.insert(i);
                            }
                        }
                    }
                }
                let start = page * RECORDS_PER_PAGE;
                let end = (start + RECORDS_PER_PAGE).min(12);
                assert_eq!(reached, (start..end).collect());
                for (i, selector) in selectors.iter().enumerate() {
                    assert_eq!(
                        visual.debug_bounds(selector).is_some(),
                        (start..end).contains(&i)
                    );
                }
                // Walk forward to the last page, then backward to the first.
                handle
                    .update(&mut visual, |w, _, _| {
                        assert_eq!(w.ui.sync_recovery.page, page);
                        assert_eq!(w.file_writes.pending(), 0);
                        assert!(w.ui.sync_recovery.message.is_empty());
                    })
                    .unwrap();
                if step == 4 {
                    break;
                }
                let forward = step < 2;
                let selector = if forward {
                    "sync-recovery-page-true"
                } else {
                    "sync-recovery-page-false"
                };
                let mut found = false;
                for _ in 0..30 {
                    key(&mut visual, navigation);
                    if let (Some(target), Some(button)) = (
                        visual.debug_bounds("focus-revealed-control"),
                        visual.debug_bounds(selector),
                    ) && target.top() >= button.top()
                        && target.bottom() <= button.bottom()
                        && target.left() >= button.left()
                        && target.right() <= button.right()
                    {
                        found = true;
                        break;
                    }
                }
                assert!(found);
                key(&mut visual, "enter");
                handle
                    .update(&mut visual, |w, window, cx| {
                        assert!(
                            w.ui.sync_recovery
                                .pager_focus
                                .get()
                                .unwrap()
                                .contains_focused(window, cx)
                        );
                    })
                    .unwrap();
            }
        }
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "current"
        );
        assert!(!vault.root.join("note 同步恢复.md").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn retention_preview_refreshes_inventory_and_never_saves_dirty_notes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = fixture();
        let old = vault.root.join(".inkstone-sync-old.backup");
        std::fs::write(&old, b"older").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        std::fs::write(vault.root.join(".inkstone-sync-old.backup.json"),
            serde_json::to_vec(&serde_json::json!({"original":"note.md", "backup":".inkstone-sync-old.backup", "sha256":"0".repeat(64)})).unwrap()).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab("note.md".into(), Some("current".into()), false, window, cx);
                w.tabs[0].save.editor.update(cx, |editor, cx| {
                    editor.set_value("unsaved 中文", window, cx)
                });
                w.flush_document_views(window, cx);
                let pending = w.file_writes.begin();
                w.preview_sync_retention(cx);
                assert!(!w.ui.sync_recovery.loading);
                w.file_writes.finish(pending);
                w.preview_sync_retention(cx);
                assert!(w.ui.sync_recovery.loading);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                let preview = w.ui.sync_recovery.preview.as_ref().unwrap();
                assert_eq!((preview.candidates, preview.candidate_bytes), (1, 5));
                assert!(
                    preview.records.iter().any(|r| r
                        .backup
                        .backup
                        .ends_with(".inkstone-sync-old.backup")
                        && r.decision == retention::Decision::Candidate)
                );
                assert!(w.tabs[0].save.persistence.is_dirty());
                assert_eq!(
                    w.tabs[0].save.editor.read(cx).value().as_ref(),
                    "unsaved 中文"
                );
                assert_eq!(w.file_writes.pending(), 0);
                // A same-vault refresh supersedes an in-flight preview, not just its rows.
                w.preview_sync_retention(cx);
                let request = w.ui.sync_recovery.request;
                w.refresh_sync_recovery(cx);
                assert_ne!(w.ui.sync_recovery.request, request);
                assert!(w.ui.sync_recovery.preview.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.ui.sync_recovery.loading);
                assert!(w.ui.sync_recovery.preview.is_none());
                assert!(w.tabs[0].save.persistence.is_dirty());
                // Unindexed data must suppress every candidate on a fresh preview.
                std::fs::write(
                    vault.root.join(".inkstone-sync-orphan.backup"),
                    b"protected",
                )
                .unwrap();
                w.preview_sync_retention(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                let preview = w.ui.sync_recovery.preview.as_ref().unwrap();
                assert_eq!(preview.candidates, 0);
                assert!(
                    preview
                        .records
                        .iter()
                        .all(|r| r.decision == retention::Decision::IncompleteInventory)
                );
            })
            .unwrap();
        assert_eq!(
            std::fs::read(vault.root.join("note.md")).unwrap(),
            b"current"
        );
        assert_eq!(std::fs::read(&old).unwrap(), b"older");
        assert_eq!(
            std::fs::read(vault.root.join(".inkstone-sync-test.backup")).unwrap(),
            b"backup"
        );
        assert_eq!(
            std::fs::read(vault.root.join(".inkstone-sync-orphan.backup")).unwrap(),
            b"protected"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn stale_inventory_cannot_populate_the_next_vault(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (root, vault) = fixture();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let other = Vault::open(root.join("other"), root.join("other-recovery")).unwrap();
        let window = cx.add_window(Workspace::new);
        window
            .update(cx, |w, _, cx| {
                w.vault = Some(vault.clone());
                w.refresh_trash(cx);
                w.preview_sync_retention(cx);
                w.generation = w.generation.wrapping_add(1);
                w.vault = Some(other);
                w.refresh_trash(cx);
                assert!(w.ui.sync_recovery.inventory.entries.is_empty());
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |w, _, _| {
                assert!(!w.ui.sync_recovery.loading);
                assert!(w.ui.sync_recovery.inventory.entries.is_empty());
                assert!(w.ui.trash_metadata.is_empty());
                assert!(w.ui.sync_recovery.preview.is_none());
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
