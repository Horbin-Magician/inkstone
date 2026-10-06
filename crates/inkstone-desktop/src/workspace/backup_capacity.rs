//! Background metadata-only capacity for the configured backup destination.
use super::settings_ui::SettingsFocusTarget;
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::backup::{
    capacity::{self, Inventory},
    retention::{self, Decision, Preview},
};

const RECORDS_PER_PAGE: usize = 5;

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
    cleanup_confirmation: Option<Preview>,
    inventory_page: usize,
    interrupted_page: usize,
    pager_focus: [std::cell::OnceCell<FocusHandle>; 2],
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
    fn can_clean_backups(&self) -> bool {
        let state = &self.ui.backup.capacity;
        !self.ui.backup.busy
            && !self.ui.backup.picker_open
            && self.ui.backup.pending.is_none()
            && !state.loading
            && state.directory.is_some()
            && state.directory == self.ui.prefs.backup.directory
            && state.local_storage == Some(true)
            && state.preview.as_ref().is_some_and(|p| p.candidates > 0)
    }
    fn request_backup_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.can_clean_backups() {
            self.ui.backup.capacity.cleanup_confirmation = self.ui.backup.capacity.preview.clone();
            cx.notify();
        }
    }
    fn execute_backup_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.can_clean_backups() {
            return;
        }
        let Some(preview) = self.ui.backup.capacity.cleanup_confirmation.take() else {
            return;
        };
        if self.ui.backup.capacity.preview.as_ref() != Some(&preview) {
            return;
        }
        let directory = self.ui.prefs.backup.directory.clone().unwrap();
        let generation = self.generation;
        self.ui.backup.busy = true;
        self.ui.pending_file_writes += 1;
        self.ui.backup.capacity.message = "正在校验并清理备份，请等待完成……".into();
        let task = cx.background_executor().spawn(async move {
            inkstone_core::vault::backup::cleanup::prepare(
                &directory,
                &preview,
                &Default::default(),
            )?
            .execute()
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
                    Ok(report) => format!(
                        "已清理 {} 份备份，正文和清单共 {:.2} MiB（逻辑容量）。",
                        report.removed,
                        report.logical_bytes as f64 / 1048576.
                    ),
                    Err(error) => {
                        format!("备份清理未完成：{error}。请检查刷新后的列表及中断记录。")
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
    fn backup_record_pager(
        &self,
        interrupted: bool,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = if interrupted {
            self.ui.backup.capacity.interrupted_page
        } else {
            self.ui.backup.capacity.inventory_page
        };
        let focus = self.ui.backup.capacity.pager_focus[usize::from(interrupted)]
            .get_or_init(|| cx.focus_handle())
            .clone();
        let pages = count.div_ceil(RECORDS_PER_PAGE).max(1);
        let kind = if interrupted {
            "中断记录"
        } else {
            "备份记录"
        };
        div()
            .track_focus(&focus)
            .flex()
            .flex_wrap()
            .gap_2()
            .child(format!(
                "{kind}：第 {} / {pages} 页，每页最多 {RECORDS_PER_PAGE} 条",
                page + 1
            ))
            .children([false, true].into_iter().map(|next| {
                let focus = focus.clone();
                let id = (
                    "backup-record-page",
                    usize::from(interrupted) * 2 + usize::from(next),
                );
                div()
                    .debug_selector(move || format!("backup-page-{}-{}", interrupted, next))
                    .child(SettingsFocusTarget::new(
                        (ElementId::from(id), "focus"),
                        &self.ui.settings_scroll,
                        Button::new(id)
                            .label(format!("{kind}{}", if next { "下一页" } else { "上一页" }))
                            .disabled(if next { page + 1 >= pages } else { page == 0 })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let state = &mut this.ui.backup.capacity;
                                let Some(inventory) = &state.inventory else {
                                    return;
                                };
                                let (page, count) = if interrupted {
                                    (&mut state.interrupted_page, inventory.interrupted.len())
                                } else {
                                    (&mut state.inventory_page, inventory.entries.len())
                                };
                                let last = count.saturating_sub(1) / RECORDS_PER_PAGE;
                                *page = if next {
                                    page.saturating_add(1).min(last)
                                } else {
                                    page.saturating_sub(1)
                                };
                                // The activated button can become disabled at a page boundary.
                                // Keep focus in the stable pager so Tab continues within settings.
                                window.focus(&focus, cx);
                                cx.notify();
                            })),
                    ))
            }))
            .into_any_element()
    }

    pub(super) fn backup_capacity_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.backup.capacity;
        let current = state.directory == self.ui.prefs.backup.directory;
        let available = !self.ui.backup.busy && self.ui.backup.pending.is_none();
        div().flex().flex_col().gap_2()
            .child("整库备份容量")
            .child(SettingsFocusTarget::new((ElementId::from("backup-capacity-refresh"), "focus"), &self.ui.settings_scroll, Button::new("backup-capacity-refresh").label("刷新备份容量")
                .disabled(self.ui.prefs.backup.directory.is_none() || current && state.loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_backup_capacity(cx)))))
            .child("统计此备份位置中的所有来源。仅统计可读取备份的正文和清单，不含异常项、额外文件及磁盘分配开销。容量列表不代表内容已校验；恢复前仍会完整校验。备份不会自动清理。")
            .child("清理预览：按来源保留最新份数，同一截止时间的记录全部保留。来源不明或读取异常时保护记录；候选尚未执行内容校验。预览本身不会删除备份；清理需要另行确认。")
            .child(div().flex().flex_wrap().gap_2().children([1usize, 3, 5, 10].into_iter().map(|keep| {
                SettingsFocusTarget::new((ElementId::from(("backup-retention-keep", keep)), "focus"), &self.ui.settings_scroll, Button::new(("backup-retention-keep", keep)).label(format!("每个来源保留 {keep} 份"))
                    .when(state.keep == keep || state.keep == 0 && keep == 3, |b| b.primary())
                    .disabled(!available || self.ui.prefs.backup.directory.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.backup.capacity.keep = keep;
                        this.refresh_backup_capacity(cx);
                    })))
            })))
            .when(!available, |s| s.child("备份或恢复任务进行中，清理预览暂停。"))
            .when_some(state.preview.as_ref().filter(|_| current && available), |s, preview| s.child(format!("{} 份候选 · 正文与清单估算 {:.2} MiB（不等于实际释放空间）", preview.candidates, preview.candidate_bytes as f64 / 1048576.)))
            .when(current && state.local_storage == Some(false), |s| s.child("此位置是网络存储或无法确认的文件系统，已禁止清理；仍可查看、校验和恢复备份。"))
            .child("清理还要求此位置不被其他机器、用户或云盘同步工具同时修改；本地磁盘检测不代表这些条件已满足。")
            .child(SettingsFocusTarget::new((ElementId::from("backup-cleanup-review"), "focus"), &self.ui.settings_scroll, Button::new("backup-cleanup-review").label("清理预览中的候选备份……")
                .disabled(!self.can_clean_backups())
                .on_click(cx.listener(|this, _, _, cx| this.request_backup_cleanup(cx)))))
            .when_some(state.cleanup_confirmation.as_ref().filter(|_| current && available), |s, preview| s
                .child(format!("将永久删除上述 {} 份候选备份，保留各来源最近 {} 份及受保护记录。无法撤销。位置：{}", preview.candidates, preview.keep_per_source, state.directory.as_ref().unwrap().display()))
                .child("仅在已停止其他机器、用户及云盘工具对此目录的写入后确认。执行前将重新校验全部备份；预览过期或校验失败会拒绝清理。")
                .child(SettingsFocusTarget::new((ElementId::from("backup-cleanup-confirm"), "focus"), &self.ui.settings_scroll, Button::new("backup-cleanup-confirm").label("已停止外部写入，确认永久清理")
                    .disabled(!self.can_clean_backups())
                    .on_click(cx.listener(|this, _, _, cx| this.execute_backup_cleanup(cx)))))
                .child(SettingsFocusTarget::new((ElementId::from("backup-cleanup-cancel"), "focus"), &self.ui.settings_scroll, Button::new("backup-cleanup-cancel").label("取消清理")
                    .on_click(cx.listener(|this, _, _, cx| { this.ui.backup.capacity.cleanup_confirmation = None; cx.notify(); })))))
            .when(current && state.loading, |s| s.child("正在读取备份容量……"))
            .when(current && !state.message.is_empty(), |s| s.child(state.message.clone()))
            .when_some(state.inventory.as_ref().filter(|_| current), |panel, inventory| {
                panel.when(!inventory.interrupted.is_empty(), |panel| panel
                    .child(format!("发现 {} 项清理中断记录；内容可能不完整，已暂停生成清理候选。请先检查，完整备份仍可恢复为新笔记库。", inventory.interrupted.len()))
                    .child(self.backup_record_pager(true, inventory.interrupted.len(), cx))
                    .children(inventory.interrupted.iter().enumerate().skip(state.interrupted_page * RECORDS_PER_PAGE).take(RECORDS_PER_PAGE).map(|(i, path)| {
                        let path = path.clone();
                        div().flex().flex_col().gap_1().min_w_0().debug_selector(move || format!("backup-interrupted-{i}"))
                            .child(div().whitespace_normal().child(path.to_string_lossy().to_string()))
                            .child(SettingsFocusTarget::new((ElementId::from(("backup-interrupted-retain-focus", i)), "focus"), &self.ui.settings_scroll,
                                Button::new(("backup-cleanup-interrupted-retain", i)).label("校验并保留为备份").accessibility_label(format!("校验并保留为备份：{}", path.display())).tooltip(path.to_string_lossy().to_string()).disabled(self.ui.backup.busy || self.ui.backup.pending.is_some() || self.ui.backup.capacity.loading).on_click(cx.listener(move |this, _, _, cx| this.retain_interrupted_backup(i, cx)))))
                            .child(SettingsFocusTarget::new((ElementId::from(("backup-interrupted-reveal-focus", i)), "focus"), &self.ui.settings_scroll,
                                Button::new(("backup-cleanup-interrupted-reveal", i)).label("检查清理中断记录").accessibility_label(format!("检查清理中断记录：{}", path.display())).tooltip(path.to_string_lossy().to_string()).on_click(move |_, _, cx| cx.reveal_path(&path))))
                    })))
                    .child(format!("{} 份备份 · 正文 {:.2} MiB · 清单 {:.2} KiB · {} 项异常未计入", inventory.entries.len(), inventory.payload_bytes as f64 / 1048576., inventory.manifest_bytes as f64 / 1024., inventory.unreadable))
                    .when(!inventory.entries.is_empty(), |panel| panel
                        .child(self.backup_record_pager(false, inventory.entries.len(), cx))
                        .children(inventory.entries.iter().enumerate().skip(state.inventory_page * RECORDS_PER_PAGE).take(RECORDS_PER_PAGE).map(|(i, entry)| {
                            let time = chrono::DateTime::from_timestamp(entry.created.min(i64::MAX as u64) as i64, 0).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "时间未知".into());
                            let path = entry.directory.clone();
                            div().flex().flex_col().gap_1().min_w_0()
                                .debug_selector(move || format!("backup-record-{i}"))
                                .child(div().whitespace_normal().child(format!("{} · {time}", entry.source_name)))
                                .child(format!("{} 个文件 · 正文 {:.2} MiB · 清单 {} 字节", entry.files, entry.payload_bytes as f64 / 1048576., entry.manifest_bytes))
                                .when(available, |row| row.when_some(state.preview.as_ref().and_then(|p| p.records.get(i)), |row, record| row.child(decision_label(record.decision))))
                                .child(SettingsFocusTarget::new((ElementId::from(("backup-capacity-reveal-focus", i)), "focus"), &self.ui.settings_scroll,
                                    Button::new(("backup-capacity-reveal", i)).compact().label("显示备份目录").accessibility_label(format!("显示备份目录：{} · {time} · {}", entry.source_name, path.display())).tooltip(path.to_string_lossy().to_string()).on_click(move |_, _, cx| cx.reveal_path(&path))))
                        })))
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
                w.ui.backup.capacity.keep = 1;
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                // An execution call without explicit confirmation has no effect.
                w.execute_backup_cleanup(cx);
                assert_eq!(w.ui.pending_file_writes, 0);
                w.request_backup_cleanup(cx);
                assert!(w.ui.backup.capacity.cleanup_confirmation.is_some());
                // Refresh invalidates the confirmation even if contents are unchanged.
                w.refresh_backup_capacity(cx);
                assert!(w.ui.backup.capacity.cleanup_confirmation.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.ui.backup.busy = true;
                w.request_backup_cleanup(cx);
                assert!(w.ui.backup.capacity.cleanup_confirmation.is_none());
                w.ui.backup.busy = false;
                w.ui.backup.picker_open = true;
                w.request_backup_cleanup(cx);
                assert!(w.ui.backup.capacity.cleanup_confirmation.is_none());
                w.ui.backup.picker_open = false;
                w.request_backup_cleanup(cx);
                w.execute_backup_cleanup(cx);
                w.execute_backup_cleanup(cx);
                assert_eq!(w.ui.pending_file_writes, 1);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.ui.backup.busy);
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(w.ui.backup.capacity.message.contains("已清理 3 份"));
                let inventory = w.ui.backup.capacity.inventory.as_ref().unwrap();
                assert_eq!(inventory.entries.len(), 2);
                assert_eq!(inventory.entries.iter().filter(|e| e.protected).count(), 1);
                assert!(root.join("backups/4/manifest.json").exists());
                assert_eq!(std::fs::read(root.join("vault/note.md")).unwrap(), b"body");
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
    #[gpui::test]
    fn backup_records_are_reachable_across_pages_without_running_operations(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                w.ui.settings = true;
                w.ui.settings_tab = 7;
                let directory = PathBuf::from("/ui-only-backup-fixture");
                w.ui.prefs.backup.directory = Some(directory.clone());
                w.ui.backup.capacity = State {
                    directory: Some(directory),
                    inventory: Some(Inventory {
                        entries: (0..12)
                            .map(|i| capacity::Summary {
                                directory: PathBuf::from(format!("/ui-only-backup-fixture/{i}")),
                                source_name: format!("测试笔记库 {i}"),
                                created: i as u64,
                                source_id: None,
                                protected: false,
                                files: 1,
                                payload_bytes: 4,
                                manifest_bytes: 100,
                            })
                            .collect(),
                        interrupted: (0..12)
                            .map(|i| {
                                PathBuf::from(format!("/ui-only-backup-fixture/interrupted-{i}"))
                            })
                            .collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        fn key(visual: &mut VisualTestContext, key: &str) {
            let keystroke = Keystroke::parse(key).unwrap();
            visual.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            visual.simulate_event(KeyUpEvent { keystroke });
            for _ in 0..2 {
                visual.update(|w, cx| w.draw(cx).clear(cx));
            }
        }
        let records = [
            "backup-record-0",
            "backup-record-1",
            "backup-record-2",
            "backup-record-3",
            "backup-record-4",
            "backup-record-5",
            "backup-record-6",
            "backup-record-7",
            "backup-record-8",
            "backup-record-9",
            "backup-record-10",
            "backup-record-11",
            "backup-record-12",
        ];
        let interrupted_records = [
            "backup-interrupted-0",
            "backup-interrupted-1",
            "backup-interrupted-2",
            "backup-interrupted-3",
            "backup-interrupted-4",
            "backup-interrupted-5",
            "backup-interrupted-6",
            "backup-interrupted-7",
            "backup-interrupted-8",
            "backup-interrupted-9",
            "backup-interrupted-10",
            "backup-interrupted-11",
            "backup-interrupted-12",
        ];
        for rem in [16., 24.] {
            visual.update(|w, _| w.set_rem_size(px(rem)));
            for interrupted in [false, true] {
                handle
                    .update(&mut visual, |w, window, cx| {
                        w.ui.backup.capacity.inventory_page = 0;
                        w.ui.backup.capacity.interrupted_page = 0;
                        window.focus(&w.ui.modal_focus, cx);
                    })
                    .unwrap();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                for page in 0..3 {
                    let start = page * RECORDS_PER_PAGE;
                    let end = (start + RECORDS_PER_PAGE).min(12);
                    let mut reached = std::collections::BTreeSet::new();
                    for _ in 0..65 {
                        key(&mut visual, "tab");
                        if let Some(target) = visual.debug_bounds("settings-focused-control") {
                            handle
                                .update(&mut visual, |w, _, _| {
                                    let viewport = w.ui.settings_scroll.bounds();
                                    assert!(
                                        target.top() >= viewport.top()
                                            && target.bottom() <= viewport.bottom(),
                                        "rem={rem} {target:?} outside {viewport:?}"
                                    );
                                })
                                .unwrap();
                            for i in start..end {
                                let selector = if interrupted {
                                    interrupted_records[i]
                                } else {
                                    records[i]
                                };
                                if let Some(row) = visual.debug_bounds(selector)
                                    && target.top() >= row.top()
                                    && target.bottom() <= row.bottom()
                                {
                                    reached.insert(i);
                                }
                            }
                        }
                    }
                    assert_eq!(
                        reached,
                        (start..end).collect(),
                        "rem={rem}, interrupted={interrupted}, page={page}"
                    );
                    assert!(
                        visual
                            .debug_bounds(records[if interrupted { 5 } else { end }])
                            .is_none()
                    );
                    if page < 2 {
                        let selector = if interrupted {
                            "backup-page-true-true"
                        } else {
                            "backup-page-false-true"
                        };
                        let mut found = false;
                        for _ in 0..65 {
                            key(&mut visual, "tab");
                            if visual
                                .debug_bounds("settings-focused-control")
                                .is_some_and(|b| Some(b) == visual.debug_bounds(selector))
                            {
                                found = true;
                                break;
                            }
                        }
                        assert!(found, "next-page button must be keyboard reachable");
                        key(&mut visual, "enter");
                        handle
                            .update(&mut visual, |w, _, _| {
                                assert_eq!(
                                    if interrupted {
                                        w.ui.backup.capacity.interrupted_page
                                    } else {
                                        w.ui.backup.capacity.inventory_page
                                    },
                                    page + 1
                                )
                            })
                            .unwrap();
                    }
                }
                // Return from the last page using Shift+Tab and Enter.
                let selector = if interrupted {
                    "backup-page-true-false"
                } else {
                    "backup-page-false-false"
                };
                let mut found = false;
                for _ in 0..65 {
                    key(&mut visual, "shift-tab");
                    if visual
                        .debug_bounds("settings-focused-control")
                        .is_some_and(|b| Some(b) == visual.debug_bounds(selector))
                    {
                        found = true;
                        break;
                    }
                }
                assert!(found);
                key(&mut visual, "enter");
                handle
                    .update(&mut visual, |w, _, _| {
                        assert_eq!(
                            if interrupted {
                                w.ui.backup.capacity.interrupted_page
                            } else {
                                w.ui.backup.capacity.inventory_page
                            },
                            1
                        );
                        assert!(!w.ui.backup.busy && !w.ui.backup.capacity.loading);
                        assert!(w.ui.backup.pending.is_none());
                        assert_eq!(w.ui.pending_file_writes, 0);
                    })
                    .unwrap();
            }
        }
    }

    #[gpui::test]
    fn backup_settings_tab_navigation_reveals_controls_without_starting_operations(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-backup-focus-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::create_dir_all(root.join("backups")).unwrap();
        std::fs::write(root.join("vault/note.md"), "keep").unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.ui.settings = true;
                w.ui.settings_tab = 7;
                w.ui.prefs.backup.directory = Some(root.join("backups"));
                // A UI-only preview enables the review/confirmation controls. No
                // backup data or deletion candidates are created by this fixture.
                let preview = Preview {
                    records: vec![],
                    candidates: 1,
                    candidate_bytes: 0,
                    keep_per_source: 3,
                };
                w.ui.backup.capacity = State {
                    directory: Some(root.join("backups")),
                    local_storage: Some(true),
                    preview: Some(preview.clone()),
                    cleanup_confirmation: Some(preview),
                    ..Default::default()
                };
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        for rem in [16., 24.] {
            visual.update(|window, _| window.set_rem_size(px(rem)));
            handle
                .update(&mut visual, |w, window, cx| {
                    w.ui.settings_scroll.set_offset(Point::default());
                    window.focus(&w.ui.modal_focus, cx);
                })
                .unwrap();
            visual.update(|window, cx| window.draw(cx).clear(cx));
            for key in ["tab", "shift-tab"] {
                let mut controls = Vec::new();
                for _ in 0..45 {
                    let keystroke = Keystroke::parse(key).unwrap();
                    visual.simulate_event(KeyDownEvent {
                        keystroke: keystroke.clone(),
                        is_held: false,
                        prefer_character_input: false,
                    });
                    visual.simulate_event(KeyUpEvent { keystroke });
                    for _ in 0..2 {
                        visual.update(|window, cx| window.draw(cx).clear(cx));
                    }
                    if let Some(target) = visual.debug_bounds("settings-focused-control") {
                        handle
                            .update(&mut visual, |w, window, cx| {
                                let viewport = w.ui.settings_scroll.bounds();
                                assert!(
                                    target.top() >= viewport.top()
                                        && target.bottom() <= viewport.bottom(),
                                    "rem={rem} {key}: {target:?} outside {viewport:?}"
                                );
                                let focus = window.focused(cx).unwrap();
                                if !controls.contains(&focus) {
                                    controls.push(focus);
                                }
                            })
                            .unwrap();
                    }
                }
                assert!(
                    controls.len() >= 22,
                    "rem={rem} {key}: only {} controls reached",
                    controls.len()
                );
            }
        }
        handle
            .update(&mut visual, |w, _, _| {
                assert!(!w.ui.backup.busy);
                assert!(!w.ui.backup.picker_open);
                assert!(w.ui.backup.pending.is_none());
                assert!(!w.ui.backup.capacity.loading);
                assert!(w.ui.backup.capacity.cleanup_confirmation.is_some());
                assert_eq!(w.ui.pending_file_writes, 0);
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("vault/note.md")).unwrap(),
            "keep"
        );
        assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
