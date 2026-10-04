use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
};
use inkstone_core::vault::backup;

#[derive(Default)]
pub(super) struct State {
    pub pending: Option<PathBuf>,
    pub busy: bool,
    picker_open: bool,
    last_attempt: u64,
    output: Option<PathBuf>,
    message: String,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn backup_name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "inkstone-backup-{}-{}-{}-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

impl Workspace {
    fn backup_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message.clone();
        self.ui.backup.message = message;
        cx.notify();
    }
    pub(super) fn request_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() || self.ui.backup.busy || self.ui.backup.pending.is_some() {
            return;
        }
        if let Some(parent) = &self.ui.prefs.backup.directory {
            self.ui.backup.pending = Some(parent.join(backup_name()));
            self.ui.backup.last_attempt = now();
            self.save_all(window, cx);
            self.persist_workspace(cx);
            self.backup_message("等待笔记与设置保存后开始备份……".into(), cx);
            self.tick_backups(window, cx);
        } else {
            self.choose_backup_directory(true, window, cx);
        }
    }
    fn choose_backup_directory(
        &mut self,
        start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.backup.busy || self.ui.backup.picker_open {
            return;
        }
        self.ui.backup.picker_open = true;
        let generation = self.generation;
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择笔记库之外的备份存放文件夹".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = dialog.await;
            let _ = this.update(cx, |this, _| {
                if this.generation == generation {
                    this.ui.backup.picker_open = false;
                }
            });
            if let Ok(Ok(Some(paths))) = result
                && let Some(path) = paths.first()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation != generation {
                        return;
                    }
                    let Some(vault) = &this.vault else { return };
                    match std::fs::canonicalize(path) {
                        Ok(path) if !path.starts_with(&vault.root) => {
                            this.ui.prefs.backup.directory = Some(path);
                            this.ui.prefs.backup.last_success = 0;
                            this.persist_workspace(cx);
                            if start {
                                this.request_backup(window, cx);
                            }
                            cx.notify();
                        }
                        _ => this
                            .backup_message("备份位置必须是笔记库之外的可访问文件夹。".into(), cx),
                    }
                });
            }
        })
        .detach();
    }
    pub(super) fn tick_backups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.ui.backup.busy {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let time = now();
        let prefs = &self.ui.prefs.backup;
        if self.ui.backup.pending.is_none()
            && prefs.interval_hours > 0
            && time.saturating_sub(prefs.last_success) >= u64::from(prefs.interval_hours) * 3600
            && time.saturating_sub(self.ui.backup.last_attempt) >= 3600
            && let Some(parent) = &prefs.directory
        {
            self.ui.backup.pending = Some(parent.join(backup_name()));
            self.ui.backup.last_attempt = time;
        }
        if self.ui.backup.pending.is_none() {
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| t.save.conflict.get() || t.save.error.borrow().is_some())
            || self.ui.persist_error.is_some()
        {
            self.ui.backup.pending = None;
            let detail = self
                .ui
                .persist_error
                .clone()
                .or_else(|| self.tabs.iter().find_map(|t| t.save.error.borrow().clone()))
                .unwrap_or_else(|| "笔记存在外部修改冲突".into());
            self.backup_message(format!("备份未开始：请先处理保存错误。{detail}"), cx);
            return;
        }
        if self.ui.file_operation
            || self.ui.pending_file_writes > 0
            || self.ui.persisting
            || self
                .tabs
                .iter()
                .any(|t| t.save.saving.get() || self.has_pending_input(t.id, window, cx))
        {
            return;
        }
        self.flush_document_views(window, cx);
        if self.tabs.iter().any(|t| t.save.dirty.get()) {
            self.save_all(window, cx);
            return;
        }
        let destination = self.ui.backup.pending.take().unwrap();
        self.ui.backup.busy = true;
        self.ui.pending_file_writes += 1;
        self.backup_message("正在备份并校验笔记库文件……".into(), cx);
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let result = backup::create(&vault, &destination);
            (destination, result)
        });
        cx.spawn(async move |this, cx| {
            let (destination, result) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.ui.backup.busy = false;
                match result {
                    Ok(manifest) => {
                        this.ui.prefs.backup.last_success = now();
                        this.ui.backup.output = Some(destination.clone());
                        this.backup_message(
                            format!(
                                "备份完成并通过校验：{} 个文件 · {}",
                                manifest.files.len(),
                                destination.display()
                            ),
                            cx,
                        );
                        this.persist_workspace(cx);
                    }
                    Err(error) => this.backup_message(
                        format!("备份失败：{error}。未生成完成的备份；请重试。"),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }
    pub(super) fn choose_backup_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.backup.busy || self.ui.backup.picker_open {
            return;
        }
        self.ui.backup.picker_open = true;
        let generation = self.generation;
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择包含 manifest.json 的墨砚备份文件夹".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = dialog.await;
            let _ = this.update(cx, |this, _| {
                if this.generation == generation { this.ui.backup.picker_open = false; }
            });
            let Ok(Ok(Some(paths))) = result else { return };
            let Some(source) = paths.first().cloned() else { return };
            let valid = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.backup.busy { return false; }
                this.ui.backup.busy = true;
                this.backup_message("正在校验所选备份……".into(), cx);
                true
            }).unwrap_or(false);
            if !valid { return; }
            let read_source = source.clone();
            let result = cx.background_executor().spawn(async move { backup::inspect(&read_source) }).await;
            let prompt = this.update_in(cx, |this, window, cx| {
                if this.generation != generation { return None; }
                this.ui.backup.busy = false;
                match result {
                    Ok(manifest) => {
                        let detail = format!("来源：{}\n文件：{}\n大小：{:.1} MiB\n所有文件已通过内容校验。接下来选择一个新目录，现有笔记库不会被覆盖。", manifest.source_name, manifest.files.len(), manifest.bytes() as f64 / 1048576.);
                        let prompt = window.prompt(PromptLevel::Info, "恢复备份为新笔记库", Some(&detail), &["选择恢复位置", "取消"], cx);
                        Some((manifest, prompt))
                    }
                    Err(error) => { this.backup_message(format!("备份校验失败：{error}"), cx); None }
                }
            }).ok().flatten();
            let Some((manifest, prompt)) = prompt else { return };
            if prompt.await != Ok(0) { return; }
            let dialog = this.update(cx, |this, cx| {
                (this.generation == generation).then(|| cx.prompt_for_new_path(source.parent().unwrap_or(&source), Some("恢复的笔记库")))
            }).ok().flatten();
            let Some(dialog) = dialog else { return };
            if let Ok(Ok(Some(destination))) = dialog.await {
                let _ = this.update(cx, |this, cx| this.restore_backup(source, destination, manifest, generation, cx));
            }
        }).detach();
    }
    fn restore_backup(
        &mut self,
        source: PathBuf,
        destination: PathBuf,
        manifest: backup::Manifest,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation || self.ui.backup.busy {
            return;
        }
        if let Some(vault) = &self.vault
            && destination
                .parent()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p.starts_with(&vault.root))
        {
            self.backup_message("请在当前笔记库之外选择新的恢复目录。".into(), cx);
            return;
        }
        self.ui.backup.busy = true;
        self.ui.pending_file_writes += 1;
        self.backup_message("正在恢复到新目录并核对内容……".into(), cx);
        let task = cx.background_executor().spawn(async move {
            let result = backup::restore(&source, &destination, &manifest);
            (destination, result)
        });
        cx.spawn(async move |this, cx| {
            let (destination, result) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                this.ui.backup.busy = false;
                match result {
                    Ok(_) => {
                        this.ui.backup.output = Some(destination.clone());
                        this.backup_message(
                            format!(
                                "已恢复为新笔记库：{}。可通过“打开另一个笔记库”使用。",
                                destination.display()
                            ),
                            cx,
                        );
                    }
                    Err(error) => {
                        this.backup_message(format!("恢复失败：{error}。现有笔记库保持不变。"), cx)
                    }
                }
            });
        })
        .detach();
    }
    pub(super) fn backup_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let prefs = &self.ui.prefs.backup;
        let destination = prefs
            .directory
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "尚未设置".into());
        let last =
            chrono::DateTime::from_timestamp(prefs.last_success.min(i64::MAX as u64) as i64, 0)
                .filter(|_| prefs.last_success > 0)
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "尚无成功备份".into());
        div().flex().flex_1().min_h_0().child(self.settings_nav(cx)).child(
            self.settings_content().gap_3()
                .child("备份与恢复")
                .child(format!("存放位置：{destination}"))
                .child(Button::new("backup-directory").label("选择备份位置").disabled(self.ui.backup.busy || self.vault.is_none())
                    .on_click(cx.listener(|this, _, w, cx| this.choose_backup_directory(false, w, cx))))
                .child(format!("最近成功：{last}"))
                .child("自动备份仅在本应用打开时运行；失败后至少间隔一小时再尝试。备份覆盖库内文件、附件、配置和空目录，不包含库外的应用恢复历史。")
                .children([(0, "仅手动"), (24, "每天"), (168, "每周")].into_iter().map(|(hours, label)| {
                    Button::new(("backup-interval", hours as usize)).label(label)
                        .when(prefs.interval_hours == hours, |b| b.primary())
                        .disabled(self.ui.backup.busy || prefs.directory.is_none() && hours > 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ui.prefs.backup.interval_hours = hours;
                            this.persist_workspace(cx);
                            cx.notify();
                        }))
                }))
                .child(Button::new("backup-now").primary().label("立即备份并校验")
                    .disabled(self.ui.backup.busy || self.ui.backup.pending.is_some() || self.vault.is_none())
                    .on_click(cx.listener(|this, _, w, cx| this.request_backup(w, cx))))
                .child(Button::new("backup-restore").label("从备份恢复为新笔记库").disabled(self.ui.backup.busy)
                    .on_click(cx.listener(|this, _, w, cx| this.choose_backup_restore(w, cx))))
                .when(!self.ui.backup.message.is_empty(), |s| s.child(self.ui.backup.message.clone()))
                .when_some(self.ui.backup.output.clone(), |s, path| s.child(Button::new("backup-reveal").label("在文件管理器中显示结果")
                    .on_click(move |_, _, cx| cx.reveal_path(&path))))
                .child("备份使用普通目录与校验清单。保留数量由你管理；符号链接或复制期间检测到文件变化时会拒绝完成备份。")
                .child("笔记版本历史保留策略（与整库备份独立）")
                .child("仅清理成功保存的旧历史，始终保留每篇最新记录；未完成草稿与受保护的冲突记录不清理。设置保存后，在后续笔记保存维护时生效。")
                .child(div().flex().flex_wrap().gap_2().children([(30, "30 天"), (90, "90 天"), (365, "一年"), (0, "不限时间")].into_iter().map(|(days, label)| {
                    Button::new(("history-retention-days", days as usize)).label(label)
                        .when(self.ui.prefs.history.days == days, |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| { this.ui.prefs.history.days = days; this.persist_workspace(cx); cx.notify(); }))
                })))
                .child(div().flex().flex_wrap().gap_2().children([(128, "128 MiB"), (512, "512 MiB"), (2048, "2 GiB"), (0, "不限容量")].into_iter().map(|(max_mib, label)| {
                    Button::new(("history-retention-size", max_mib as usize)).label(label)
                        .when(self.ui.prefs.history.max_mib == max_mib, |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| { this.ui.prefs.history.max_mib = max_mib; this.persist_workspace(cx); cx.notify(); }))
                })))
        ).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn backup_waits_for_saves_restores_to_new_vault_and_records_success(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(backup_name());
        std::fs::create_dir_all(root.join("vault/empty")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(root.join("vault/note.md"), "old").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("vault"), window, cx)
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
            .update(cx, |w, window, cx| {
                w.ui.prefs.backup.directory = Some(root.clone());
                let doc = w.tabs[w.active.unwrap()].save.clone();
                doc.editor
                    .update(cx, |s, cx| s.replace_all("中文 new 😀\r\n", window, cx));
                w.request_backup(window, cx);
            })
            .unwrap();
        for _ in 0..6 {
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| w.tick(window, cx))
                .unwrap();
        }
        cx.run_until_parked();
        let backup_path = handle
            .update(cx, |w, _, _| {
                assert!(
                    !w.ui.backup.busy && w.ui.backup.pending.is_none(),
                    "{}; settings={:?}; notes={:?}",
                    w.ui.backup.message,
                    w.ui.persist_error,
                    w.tabs
                        .iter()
                        .map(|t| t.save.error.borrow().clone())
                        .collect::<Vec<_>>()
                );
                assert!(
                    w.ui.prefs.backup.last_success > 0,
                    "{}; settings={:?}; notes={:?}",
                    w.ui.backup.message,
                    w.ui.persist_error,
                    w.tabs
                        .iter()
                        .map(|t| t.save.error.borrow().clone())
                        .collect::<Vec<_>>()
                );
                w.ui.backup.output.clone().unwrap()
            })
            .unwrap();
        let manifest = backup::inspect(&backup_path).unwrap();
        assert_eq!(
            std::fs::read(backup_path.join("files/note.md")).unwrap(),
            "中文 new 😀\r\n".as_bytes()
        );
        assert!(
            manifest
                .files
                .iter()
                .any(|e| e.path == std::path::Path::new(".inkstone-workspace.json"))
        );
        handle
            .update(cx, |w, _, cx| {
                w.restore_backup(
                    backup_path.clone(),
                    root.join("restored"),
                    manifest,
                    w.generation,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read(root.join("restored/note.md")).unwrap(),
            "中文 new 😀\r\n".as_bytes()
        );
        assert!(root.join("restored/empty").is_dir());
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.ui.backup.busy);
                assert_eq!(w.ui.pending_file_writes, 0);
                assert_eq!(w.vault.as_ref().unwrap().root, root.join("vault"));
                w.ui.prefs.backup.interval_hours = 24;
                let before = w.ui.backup.output.clone();
                w.tick_backups(window, cx);
                assert!(w.ui.backup.pending.is_none());
                assert_eq!(w.ui.backup.output, before);
                w.watcher = None;
                for e in w
                    .vault
                    .as_ref()
                    .unwrap()
                    .history(std::path::Path::new("note.md"))
                    .unwrap()
                {
                    std::fs::remove_file(e.journal).unwrap();
                }
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn automatic_backup_failures_are_throttled_and_do_not_bypass_conflicts(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(backup_name());
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/note.md"), "original").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("vault"), window, cx)
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
            .update(cx, |w, window, cx| {
                w.ui.prefs.backup.directory = Some(root.clone());
                w.ui.prefs.backup.interval_hours = 24;
                w.tabs[w.active.unwrap()].save.conflict.set(true);
                w.tick_backups(window, cx);
                assert!(w.ui.backup.pending.is_none() && !w.ui.backup.busy);
                assert!(w.ui.backup.message.contains("保存错误"));
                let last = w.ui.backup.last_attempt;
                w.tick_backups(window, cx);
                assert_eq!(last, w.ui.backup.last_attempt);
                assert_eq!(w.ui.prefs.backup.last_success, 0);
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
