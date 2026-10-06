//! Inspection and copy-only recovery of incomplete cleanup records.
use super::*;
use inkstone_core::vault::backup::interrupted::{self, Preview, State as FileState};

#[derive(Default)]
pub(super) struct State {
    request: u64,
    source: Option<PathBuf>,
    preview: Option<Preview>,
    loading: bool,
    message: String,
    page: usize,
}

impl Workspace {
    pub(super) fn inspect_partial_backup(&mut self, index: usize, cx: &mut Context<Self>) {
        let capacity = &self.ui.backup.capacity;
        if self.ui.backup.busy
            || self.ui.backup.picker_open
            || capacity.loading
            || capacity.directory != self.ui.prefs.backup.directory
        {
            return;
        }
        let Some(directory) = capacity.directory.clone() else {
            return;
        };
        let Some(source) = capacity
            .inventory
            .as_ref()
            .and_then(|i| i.interrupted.get(index))
            .cloned()
        else {
            return;
        };
        let request = capacity.partial.request.wrapping_add(1);
        let epoch = capacity.request;
        let generation = self.generation;
        self.ui.backup.capacity.partial = State {
            request,
            source: Some(source.clone()),
            loading: true,
            ..Default::default()
        };
        let task = cx
            .background_executor()
            .spawn(async move { interrupted::inspect(&directory, &source) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if !this.partial_backup_current(generation, epoch, request) {
                    return;
                }
                let state = &mut this.ui.backup.capacity.partial;
                state.loading = false;
                match result {
                    Ok(preview) => state.preview = Some(preview),
                    Err(error) => state.message = format!("无法校验剩余文件：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn partial_backup_current(&self, generation: u64, epoch: u64, request: u64) -> bool {
        self.generation == generation
            && self.ui.backup.capacity.request == epoch
            && self.ui.backup.capacity.partial.request == request
            && self.ui.backup.capacity.directory == self.ui.prefs.backup.directory
    }

    fn choose_partial_destination(&mut self, cx: &mut Context<Self>) {
        if self.ui.backup.busy || self.ui.backup.picker_open || self.ui.backup.pending.is_some() {
            return;
        }
        let state = &self.ui.backup.capacity;
        let Some(directory) = state.directory.clone() else {
            return;
        };
        if state.partial.preview.is_none() {
            return;
        }
        let (generation, epoch, request) = (self.generation, state.request, state.partial.request);
        self.ui.backup.picker_open = true;
        let dialog = cx.prompt_for_new_path(&directory, Some("恢复的剩余文件"));
        cx.spawn(async move |this, cx| {
            let result = dialog.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.ui.backup.picker_open = false;
                if this.partial_backup_current(generation, epoch, request)
                    && let Ok(Ok(Some(destination))) = result
                {
                    this.restore_partial_backup(destination, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn restore_partial_backup(&mut self, destination: PathBuf, cx: &mut Context<Self>) {
        if self.ui.backup.busy || self.ui.backup.picker_open || self.ui.backup.pending.is_some() {
            return;
        }
        let capacity = &self.ui.backup.capacity;
        if capacity.loading || capacity.directory != self.ui.prefs.backup.directory {
            return;
        }
        let (Some(directory), Some(source), Some(preview)) = (
            capacity.directory.clone(),
            capacity.partial.source.clone(),
            capacity.partial.preview.clone(),
        ) else {
            return;
        };
        if let Some(vault) = &self.vault
            && destination
                .parent()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p.starts_with(&vault.root))
        {
            self.ui.backup.capacity.partial.message = "请在当前笔记库之外选择新的恢复目录。".into();
            cx.notify();
            return;
        }
        let (generation, epoch, request) =
            (self.generation, capacity.request, capacity.partial.request);
        self.ui.backup.busy = true;
        self.ui.backup.capacity.partial.message = "正在重新校验并复制有效文件……".into();
        let ticket = self.file_writes.begin();
        let task = cx.background_executor().spawn(async move {
            let result = interrupted::restore_verified(&directory, &source, &destination, &preview);
            (destination, result)
        });
        cx.spawn(async move |this, cx| {
            let (destination, result) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation {
                    return;
                }
                this.ui.backup.busy = false;
                if !this.partial_backup_current(generation, epoch, request) {
                    return;
                }
                this.ui.backup.capacity.partial.message = match result {
                    Ok(count) => format!(
                        "已恢复 {count} 个有效文件到 {}。原残留仍保留；这不是完整备份。",
                        destination.display()
                    ),
                    Err(error) => format!("未提交恢复副本：{error}。请重新校验。"),
                };
                this.notifications
                    .publish(this.ui.backup.capacity.partial.message.clone());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn partial_backup_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.ui.backup.capacity.partial;
        let busy =
            self.ui.backup.busy || self.ui.backup.picker_open || self.ui.backup.pending.is_some();
        div().flex().flex_col().min_w_0().gap_2()
            .when_some(state.source.as_ref(), |s, path| s.child(div().whitespace_normal().child(format!("剩余文件校验：{}", path.display()))))
            .when(state.loading, |s| s.child("正在校验剩余文件……"))
            .when(!state.message.is_empty(), |s| s.child(div().whitespace_normal().child(state.message.clone())))
            .when_some(state.preview.as_ref(), |s, preview| {
                let verified = preview.files.iter().filter(|s| **s == FileState::Verified).count();
                let pages = preview.files.len().div_ceil(RECORDS_PER_PAGE).max(1);
                s.child(format!("{} 个文件中 {verified} 个通过校验。缺失、损坏及清单外文件不会恢复；原残留不删除。", preview.files.len()))
                    .children(preview.manifest.files.iter().zip(&preview.files).skip(state.page * RECORDS_PER_PAGE).take(RECORDS_PER_PAGE).map(|(entry, status)| {
                        let label = match status { FileState::Verified => "可恢复".into(), FileState::Missing => "缺失".into(), FileState::Unavailable(reason) => format!("不可恢复：{reason}") };
                        div().whitespace_normal().child(format!("{} · {label}", entry.path.display()))
                    }))
                    .child(format!("第 {} / {pages} 页", state.page + 1))
                    .children([false, true].into_iter().map(|next| FocusReveal::new((ElementId::from(("partial-page", usize::from(next))), "focus"), &self.ui.settings_scroll,
                        Button::new(("partial-page", usize::from(next))).label(if next { "下一页文件" } else { "上一页文件" })
                            .disabled(if next { state.page + 1 >= pages } else { state.page == 0 })
                            .on_click(cx.listener(move |this, _, _, cx| { let page = &mut this.ui.backup.capacity.partial.page; *page = if next { (*page + 1).min(pages - 1) } else { page.saturating_sub(1) }; cx.notify(); })))))
                    .child(FocusReveal::new((ElementId::from("partial-restore"), "focus"), &self.ui.settings_scroll,
                        Button::new("partial-restore").label(format!("将 {verified} 个有效文件恢复到新目录……")).disabled(busy || verified == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.choose_partial_destination(cx)))))
            }).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn partial_recovery_keeps_dirty_notes_and_rejects_stale_inspection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-partial-ui-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::create_dir_all(root.join("backups")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("keep.md"), "磁盘内容").unwrap();
        std::fs::write(vault.root.join("missing.md"), "missing").unwrap();
        let source = root.join("backups/.inkstone-backup-cleanup-ui");
        inkstone_core::vault::backup::create(&vault, &source).unwrap();
        std::fs::remove_file(source.join("files/missing.md")).unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(
                    PathBuf::from("keep.md"),
                    Some("磁盘内容".into()),
                    false,
                    window,
                    cx,
                );
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("未保存内容", window, cx));
                w.flush_document_views(window, cx);
                assert!(w.tabs.last().unwrap().save.persistence.is_dirty());
                w.ui.prefs.backup.directory = Some(root.join("backups"));
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.inspect_partial_backup(0, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(
                    w.ui.backup.capacity.partial.preview.as_ref().unwrap().files,
                    vec![FileState::Verified, FileState::Missing]
                );
                w.restore_partial_backup(vault.root.join("forbidden"), cx);
                assert_eq!(w.file_writes.pending(), 0);
                assert!(!vault.root.join("forbidden").exists());
                w.restore_partial_backup(root.join("copy"), cx);
                w.restore_partial_backup(root.join("duplicate"), cx);
                assert_eq!(w.file_writes.pending(), 1);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                assert!(!w.ui.backup.busy);
                assert!(w.ui.backup.capacity.partial.message.contains("已恢复 1"));
                assert_eq!(
                    w.current_pane()
                        .unwrap()
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .as_ref(),
                    "未保存内容"
                );
                assert!(w.tabs.last().unwrap().save.persistence.is_dirty());
                w.inspect_partial_backup(0, cx);
                w.refresh_backup_capacity(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(w.ui.backup.capacity.partial.preview.is_none());
                w.inspect_partial_backup(0, cx);
                w.generation += 1;
                w.ui.backup.capacity.partial = State::default();
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.ui.backup.capacity.partial.preview.is_none());
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("copy/keep.md")).unwrap(),
            "磁盘内容"
        );
        assert!(!root.join("copy/missing.md").exists());
        assert!(!root.join("duplicate").exists());
        assert_eq!(
            std::fs::read_to_string(vault.root.join("keep.md")).unwrap(),
            "磁盘内容"
        );
        assert!(source.join("files/keep.md").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
