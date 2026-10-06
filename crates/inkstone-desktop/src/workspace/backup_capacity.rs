//! Background metadata-only capacity for the configured backup destination.
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::backup::{
    capacity::{self, Inventory},
    retention::{self, Decision, Preview},
};

#[derive(Default)]
pub(super) struct State {
    request: u64,
    directory: Option<PathBuf>,
    loading: bool,
    inventory: Option<Inventory>,
    message: String,
    keep: usize,
    preview: Option<Preview>,
    local_storage: Option<bool>,
}
impl Workspace {
    pub(super) fn refresh_backup_capacity(&mut self, cx: &mut Context<Self>) {
        let directory = self.ui.prefs.backup.directory.clone();
        let request = self.ui.backup.capacity.request.wrapping_add(1);
        let keep = self.ui.backup.capacity.keep.max(1);
        let keep = if self.ui.backup.capacity.keep == 0 {
            3
        } else {
            keep
        };
        self.ui.backup.capacity = State {
            request,
            keep,
            directory: directory.clone(),
            loading: directory.is_some(),
            ..Default::default()
        };
        cx.notify();
        let Some(directory) = directory else {
            return;
        };
        let generation = self.generation;
        let scan = directory.clone();
        let task = cx.background_executor().spawn(async move {
            let inventory = capacity::list(&scan)?;
            let preview = retention::preview(&inventory, keep, &Default::default())?;
            let local =
                inkstone_core::vault::backup::cleanup::supports_cleanup(&scan).unwrap_or(false);
            Ok::<_, std::io::Error>((inventory, preview, local))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation
                    || this.ui.backup.capacity.request != request
                    || this.ui.prefs.backup.directory.as_ref() != Some(&directory)
                {
                    return;
                }
                let state = &mut this.ui.backup.capacity;
                state.loading = false;
                match result {
                    Ok((inventory, preview, local)) => {
                        state.local_storage = Some(local);
                        state.inventory = Some(inventory);
                        state.preview = Some(preview);
                    }
                    Err(error) => state.message = format!("无法读取备份容量：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn retain_interrupted_backup(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.ui.backup.busy
            || self.ui.backup.pending.is_some()
            || self.ui.backup.capacity.loading
            || self.ui.backup.capacity.directory != self.ui.prefs.backup.directory
        {
            return;
        }
        let Some(directory) = self.ui.prefs.backup.directory.clone() else {
            return;
        };
        let Some(path) = self
            .ui
            .backup
            .capacity
            .inventory
            .as_ref()
            .and_then(|i| i.interrupted.get(index))
            .cloned()
        else {
            return;
        };
        self.ui.backup.busy = true;
        self.ui.pending_file_writes += 1;
        self.ui.backup.capacity.message = "正在完整校验并保留中断备份……".into();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            inkstone_core::vault::backup::cleanup::retain_interrupted(&directory, &path)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.ui.backup.busy = false;
                this.refresh_backup_capacity(cx);
                let message = match result {
                    Ok(path) => {
                        format!("已保留为受保护备份：{}。不会作为清理候选。", path.display())
                    }
                    Err(error) => {
                        format!("无法保留中断备份：{error}。残留内容未删除，请检查目录。")
                    }
                };
                this.ui.backup.capacity.message = message.clone();
                this.notifications.publish(message);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn backup_capacity_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.backup.capacity;
        let current = state.directory == self.ui.prefs.backup.directory;
        let available = !self.ui.backup.busy && self.ui.backup.pending.is_none();
        div().flex().flex_col().gap_2()
            .child("整库备份容量")
            .child(Button::new("backup-capacity-refresh").label("刷新备份容量")
                .disabled(self.ui.prefs.backup.directory.is_none() || current && state.loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_backup_capacity(cx))))
            .child("统计此备份位置中的所有来源。仅统计可读取备份的正文和清单，不含异常项、额外文件及磁盘分配开销。容量列表不代表内容已校验；恢复前仍会完整校验。备份不会自动清理。")
            .child("清理预览：按来源保留最新份数，同一截止时间的记录全部保留。来源不明或读取异常时保护记录；候选尚未执行内容校验。此处仅预览，不会删除备份。")
            .child(div().flex().flex_wrap().gap_2().children([1usize, 3, 5, 10].into_iter().map(|keep| {
                Button::new(("backup-retention-keep", keep)).label(format!("每个来源保留 {keep} 份"))
                    .when(state.keep == keep || state.keep == 0 && keep == 3, |b| b.primary())
                    .disabled(!available || self.ui.prefs.backup.directory.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.backup.capacity.keep = keep;
                        this.refresh_backup_capacity(cx);
                    }))
            })))
            .when(!available, |s| s.child("备份或恢复任务进行中，清理预览暂停。"))
            .when_some(state.preview.as_ref().filter(|_| current && available), |s, preview| s.child(format!("{} 份候选 · 正文与清单估算 {:.2} MiB（不等于实际释放空间）", preview.candidates, preview.candidate_bytes as f64 / 1048576.)))
            .when(current && state.local_storage == Some(false), |s| s.child("此位置是网络存储或无法确认的文件系统，已禁止清理；仍可查看、校验和恢复备份。"))
            .child("清理还要求此位置不被其他机器、用户或云盘同步工具同时修改；本地磁盘检测不代表这些条件已满足。")
            .when(current && state.loading, |s| s.child("正在读取备份容量……"))
            .when(current && !state.message.is_empty(), |s| s.child(state.message.clone()))
            .when_some(state.inventory.as_ref().filter(|_| current), |panel, inventory| {
                panel.when(!inventory.interrupted.is_empty(), |panel| panel
                    .child(format!("发现 {} 项清理中断记录；内容可能不完整，已暂停生成清理候选。请先检查，完整备份仍可恢复为新笔记库。", inventory.interrupted.len()))
                    .child(uniform_list("backup-cleanup-interrupted", inventory.interrupted.len(), cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let Some(inventory) = &this.ui.backup.capacity.inventory else { return Vec::new(); };
                        range.filter_map(|i| inventory.interrupted.get(i).map(|path| {
                            let path = path.clone();
                            div().h(px(84.)).flex().flex_col().gap_1().overflow_hidden()
                                .child(Button::new(("backup-cleanup-interrupted-retain", i)).label("校验并保留为备份").disabled(this.ui.backup.busy || this.ui.backup.pending.is_some() || this.ui.backup.capacity.loading).on_click(cx.listener(move |this, _, _, cx| this.retain_interrupted_backup(i, cx))))
                                .child(Button::new(("backup-cleanup-interrupted-reveal", i)).label("检查清理中断记录").tooltip(path.to_string_lossy().to_string()).on_click(move |_, _, cx| cx.reveal_path(&path)))
                                .into_any_element()
                        })).collect()
                    })).h(px(168.))))
                    .child(format!("{} 份备份 · 正文 {:.2} MiB · 清单 {:.2} KiB · {} 项异常未计入", inventory.entries.len(), inventory.payload_bytes as f64 / 1048576., inventory.manifest_bytes as f64 / 1024., inventory.unreadable))
                    .when(!inventory.entries.is_empty(), |panel| panel.child(uniform_list(
                        "backup-capacity-list", inventory.entries.len(), cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            let Some(inventory) = &this.ui.backup.capacity.inventory else { return Vec::new(); };
                            range.filter_map(|i| inventory.entries.get(i).map(|entry| {
                                let time = chrono::DateTime::from_timestamp(entry.created.min(i64::MAX as u64) as i64, 0).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "时间未知".into());
                                let path = entry.directory.clone();
                                div().h(px(108.)).flex().flex_col().gap_1().overflow_hidden()
                                    .child(div().truncate().child(format!("{} · {time}", entry.source_name)))
                                    .child(format!("{} 个文件 · 正文 {:.2} MiB · 清单 {} 字节", entry.files, entry.payload_bytes as f64 / 1048576., entry.manifest_bytes))
                                    .when(!this.ui.backup.busy && this.ui.backup.pending.is_none(), |row| row.when_some(this.ui.backup.capacity.preview.as_ref().and_then(|p| p.records.get(i)), |row, record| row.child(decision_label(record.decision))))
                                    .child(Button::new(("backup-capacity-reveal", i)).compact().label("显示备份目录").tooltip(path.to_string_lossy().to_string()).on_click(move |_, _, cx| cx.reveal_path(&path)))
                                    .into_any_element()
                            })).collect()
                        })
                    ).h(px(228.))))
            }).into_any_element()
    }
}

fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Candidate => "清理候选：超过保留份数，尚待完整校验",
        Decision::Recent => "保留：近期备份或截止时间并列",
        Decision::InUse => "保护：备份正在使用",
        Decision::Protected => "保护：从清理中断中保留的备份",
        Decision::UnknownSource => "保护：无法确认来源",
        Decision::IncompleteInventory => "保护：清单异常或存在未处理的清理中断记录",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn capacity_refresh_reports_backups_and_rejects_previous_directory(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-capacity-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::create_dir_all(root.join("backups")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("vault/note.md"), b"body").unwrap();
        let vault =
            inkstone_core::vault::Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        for created in 1..=4 {
            let directory = root.join(format!("backups/{created}"));
            let mut manifest = inkstone_core::vault::backup::create(&vault, &directory).unwrap();
            manifest.created = created;
            manifest.source_id = Some("a".repeat(64));
            std::fs::write(
                directory.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, cx| {
                w.ui.prefs.backup.directory = Some(root.join("backups"));
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                let inventory = w.ui.backup.capacity.inventory.as_ref().unwrap();
                assert_eq!(inventory.entries.len(), 4);
                assert_eq!(inventory.payload_bytes, 16);
                assert!(!w.ui.backup.capacity.loading);
                let preview = w.ui.backup.capacity.preview.as_ref().unwrap();
                assert_eq!((preview.keep_per_source, preview.candidates), (3, 1));
                assert_eq!(
                    preview
                        .records
                        .iter()
                        .filter(|r| r.decision == Decision::Candidate)
                        .count(),
                    1
                );
                w.ui.backup.capacity.keep = 1;
                w.refresh_backup_capacity(cx);
                w.ui.backup.capacity.keep = 5;
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                let preview = w.ui.backup.capacity.preview.as_ref().unwrap();
                assert_eq!((preview.keep_per_source, preview.candidates), (5, 0));
                inkstone_core::vault::backup::create(
                    &vault,
                    &root.join("backups/.inkstone-backup-cleanup-test"),
                )
                .unwrap();
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(
                    w.ui.backup
                        .capacity
                        .inventory
                        .as_ref()
                        .unwrap()
                        .interrupted
                        .len(),
                    1
                );
                let preview = w.ui.backup.capacity.preview.as_ref().unwrap();
                assert_eq!(preview.candidates, 0);
                assert!(
                    preview
                        .records
                        .iter()
                        .all(|r| r.decision == Decision::IncompleteInventory)
                );
                w.retain_interrupted_backup(0, cx);
                w.retain_interrupted_backup(0, cx);
                assert_eq!(w.ui.pending_file_writes, 1);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.ui.backup.busy);
                assert_eq!(w.ui.pending_file_writes, 0);
                let inventory = w.ui.backup.capacity.inventory.as_ref().unwrap();
                assert!(inventory.interrupted.is_empty());
                assert_eq!(inventory.entries.iter().filter(|e| e.protected).count(), 1);
                assert!(
                    w.ui.backup
                        .capacity
                        .preview
                        .as_ref()
                        .unwrap()
                        .records
                        .iter()
                        .any(|r| r.decision == Decision::Protected)
                );
                // The earlier request completes after a new directory is selected.
                w.refresh_backup_capacity(cx);
                w.ui.prefs.backup.directory = Some(root.join("empty"));
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.ui.backup.capacity.directory, Some(root.join("empty")));
                assert!(
                    w.ui.backup
                        .capacity
                        .inventory
                        .as_ref()
                        .unwrap()
                        .entries
                        .is_empty()
                );
                w.ui.prefs.backup.directory = Some(root.join("missing"));
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(w.ui.backup.capacity.message.contains("无法读取"));
                assert!(w.ui.backup.capacity.inventory.is_none());
                assert!(w.ui.backup.capacity.preview.is_none());
                assert!(!w.ui.backup.capacity.loading);
                w.ui.prefs.backup.directory = None;
                w.refresh_backup_capacity(cx);
                assert!(w.ui.backup.capacity.inventory.is_none());
                assert!(w.ui.backup.capacity.preview.is_none());
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
