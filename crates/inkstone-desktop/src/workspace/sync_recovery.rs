//! Sync backups inside the shared file recovery entry point.
use super::settings_ui::SettingsFocusTarget;
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::sync::recovery::{self, Inventory};

const RECORDS_PER_PAGE: usize = 5;

#[derive(Default)]
pub(super) struct State {
    generation: u64,
    page: usize,
    pager_focus: std::cell::OnceCell<FocusHandle>,
    inventory: Inventory,
    loading: bool,
    message: String,
}
impl Workspace {
    pub(super) fn refresh_sync_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.ui.sync_recovery = State {
            generation: self.generation,
            loading: true,
            ..Default::default()
        };
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx
            .background_executor()
            .spawn(async move { recovery::inventory(&vault) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let state = &mut this.ui.sync_recovery;
                state.loading = false;
                match result {
                    Ok(inventory) => state.inventory = inventory,
                    Err(error) => state.message = format!("无法读取同步备份：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn restore_sync_backup(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.generation != self.ui.sync_recovery.generation
            || self.ui.file_operation
            || self.ui.pending_file_writes > 0
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
        self.ui.pending_file_writes += 1;
        self.ui.sync_recovery.message = "正在恢复副本……".into();
        let task = cx
            .background_executor()
            .spawn(async move { recovery::restore_copy(&vault, &entry, &reserved) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
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
            .child(div().text_sm().child(format!(
                "{} 条 · {:.1} MiB · 不自动清理；恢复后保留备份",
                state.inventory.entries.len(),
                state.inventory.bytes as f64 / 1048576.
            )))
            .when(state.loading, |s| s.child("正在读取同步备份……"))
            .when(!state.loading && state.inventory.entries.is_empty(), |s| {
                s.child("暂无同步备份")
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
                                    .child(SettingsFocusTarget::new(
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
                                let time: chrono::DateTime<chrono::Local> = entry.modified.into();
                                let description = format!(
                                    "同步备份 · {} · {} · {:.1} KiB",
                                    entry.original.display(),
                                    time.format("%Y-%m-%d %H:%M:%S"),
                                    entry.bytes as f64 / 1024.
                                );
                                div()
                                    .debug_selector(move || format!("sync-recovery-record-{i}"))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .flex_shrink_0()
                                    .child(div().whitespace_normal().child(description.clone()))
                                    .child(SettingsFocusTarget::new(
                                        (ElementId::from(("restore-sync-backup", i)), "focus"),
                                        &self.ui.recovery_scroll,
                                        Button::new(("restore-sync-backup", i))
                                            .compact()
                                            .label("恢复为副本")
                                            .accessibility_label(format!(
                                                "恢复为副本：{description}"
                                            ))
                                            .disabled(
                                                self.ui.file_operation
                                                    || self.ui.pending_file_writes > 0
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
                w.ui.pending_file_writes = 1;
                w.restore_sync_backup(0, cx);
                assert_eq!(w.ui.pending_file_writes, 1);
                w.ui.pending_file_writes = 0;
                w.restore_sync_backup(0, cx);
                w.restore_sync_backup(0, cx);
                assert_eq!(w.ui.pending_file_writes, 1);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |w, _, _| {
                assert_eq!(w.ui.pending_file_writes, 0);
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
                    if let Some(target) = visual.debug_bounds("settings-focused-control") {
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
                        assert_eq!(w.ui.pending_file_writes, 0);
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
                        visual.debug_bounds("settings-focused-control"),
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
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
