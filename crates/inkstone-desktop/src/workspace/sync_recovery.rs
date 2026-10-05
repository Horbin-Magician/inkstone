//! Sync backups inside the shared file recovery entry point.
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::sync::recovery::{self, Inventory};

#[derive(Default)]
pub(super) struct State {
    generation: u64,
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
                panel.child(
                    uniform_list(
                        "sync-recovery-entries",
                        state.inventory.entries.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            let state = &this.ui.sync_recovery;
                            range
                                .map(|i| {
                                    let entry = &state.inventory.entries[i];
                                    let time: chrono::DateTime<chrono::Local> =
                                        entry.modified.into();
                                    div()
                                        .h(px(84.))
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .overflow_hidden()
                                        .child(
                                            div().truncate().child(
                                                entry.original.to_string_lossy().to_string(),
                                            ),
                                        )
                                        .child(div().text_sm().child(format!(
                                            "{} · {:.1} KiB",
                                            time.format("%Y-%m-%d %H:%M:%S"),
                                            entry.bytes as f64 / 1024.
                                        )))
                                        .child(
                                            Button::new(("restore-sync-backup", i))
                                                .compact()
                                                .label("恢复为副本")
                                                .tooltip(
                                                    entry.original.to_string_lossy().to_string(),
                                                )
                                                .disabled(
                                                    this.ui.file_operation
                                                        || this.ui.pending_file_writes > 0
                                                        || state.loading,
                                                )
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.restore_sync_backup(i, cx)
                                                })),
                                        )
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h(px(240.))
                    .w_full(),
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
