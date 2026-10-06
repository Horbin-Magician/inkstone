//! Background metadata-only capacity for the configured backup destination.
use super::*;
use gpui_component::{Disableable, button::*};
use inkstone_core::vault::backup::capacity::{self, Inventory};

#[derive(Default)]
pub(super) struct State {
    request: u64,
    directory: Option<PathBuf>,
    loading: bool,
    inventory: Option<Inventory>,
    message: String,
}
impl Workspace {
    pub(super) fn refresh_backup_capacity(&mut self, cx: &mut Context<Self>) {
        let directory = self.ui.prefs.backup.directory.clone();
        let request = self.ui.backup.capacity.request.wrapping_add(1);
        self.ui.backup.capacity = State {
            request,
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
        let task = cx
            .background_executor()
            .spawn(async move { capacity::list(&scan) });
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
                    Ok(inventory) => state.inventory = Some(inventory),
                    Err(error) => state.message = format!("无法读取备份容量：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn backup_capacity_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.backup.capacity;
        let current = state.directory == self.ui.prefs.backup.directory;
        div().flex().flex_col().gap_2()
            .child("整库备份容量")
            .child(Button::new("backup-capacity-refresh").label("刷新备份容量")
                .disabled(self.ui.prefs.backup.directory.is_none() || current && state.loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_backup_capacity(cx))))
            .child("统计此备份位置中的所有来源。仅统计可读取备份的正文和清单，不含异常项、额外文件及磁盘分配开销。容量列表不代表内容已校验；恢复前仍会完整校验。备份不会自动清理。")
            .when(current && state.loading, |s| s.child("正在读取备份容量……"))
            .when(current && !state.message.is_empty(), |s| s.child(state.message.clone()))
            .when_some(state.inventory.as_ref().filter(|_| current), |panel, inventory| {
                panel.child(format!("{} 份备份 · 正文 {:.2} MiB · 清单 {:.2} KiB · {} 项异常未计入", inventory.entries.len(), inventory.payload_bytes as f64 / 1048576., inventory.manifest_bytes as f64 / 1024., inventory.unreadable))
                    .when(!inventory.entries.is_empty(), |panel| panel.child(uniform_list(
                        "backup-capacity-list", inventory.entries.len(), cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            let Some(inventory) = &this.ui.backup.capacity.inventory else { return Vec::new(); };
                            range.filter_map(|i| inventory.entries.get(i).map(|entry| {
                                let time = chrono::DateTime::from_timestamp(entry.created.min(i64::MAX as u64) as i64, 0).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "时间未知".into());
                                let path = entry.directory.clone();
                                div().h(px(84.)).flex().flex_col().gap_1().overflow_hidden()
                                    .child(div().truncate().child(format!("{} · {time}", entry.source_name)))
                                    .child(format!("{} 个文件 · 正文 {:.2} MiB · 清单 {} 字节", entry.files, entry.payload_bytes as f64 / 1048576., entry.manifest_bytes))
                                    .child(Button::new(("backup-capacity-reveal", i)).compact().label("显示备份目录").tooltip(path.to_string_lossy().to_string()).on_click(move |_, _, cx| cx.reveal_path(&path)))
                                    .into_any_element()
                            })).collect()
                        })
                    ).h(px(180.))))
            }).into_any_element()
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
        inkstone_core::vault::backup::create(&vault, &root.join("backups/one")).unwrap();
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
                assert_eq!(inventory.entries.len(), 1);
                assert_eq!(inventory.payload_bytes, 4);
                assert!(!w.ui.backup.capacity.loading);
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
                assert!(!w.ui.backup.capacity.loading);
                w.ui.prefs.backup.directory = None;
                w.refresh_backup_capacity(cx);
                assert!(w.ui.backup.capacity.inventory.is_none());
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
