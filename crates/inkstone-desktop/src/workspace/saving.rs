//! Save scheduling, recovery journaling, and conflict copies.

use super::*;

impl Workspace {
    pub(super) fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() {
            return;
        }
        self.flush_document_views(window, cx);
        for tab in &mut self.tabs {
            tab.save.persistence.retry();
        }
        self.save_pending(window, cx);
    }
    pub(super) fn save_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        for tab in &mut self.tabs {
            let editor = tab.save.editor.clone();
            if editor.update(cx, |state, cx| {
                state.marked_text_range(window, cx).is_some()
            }) {
                continue;
            }
            if tab.save.persistence.is_dirty()
                && (tab.save.persistence.has_conflict() || tab.save.persistence.error().is_some())
            {
                let text = tab.save.editor.read(cx).value().to_string();
                if tab.save.persistence.begin_journal(&text) {
                    let vault = vault.clone();
                    let path = tab.save.path.borrow().clone();
                    let baseline = tab.save.persistence.baseline().clone();
                    let save = tab.save.clone();
                    let task = cx
                        .background_executor()
                        .spawn(async move { vault.journal(&path, baseline.as_deref(), &text) });
                    cx.spawn(async move |this, cx| {
                        if let Err(error) = task.await {
                            let _ = this.update(cx, |this, cx| {
                                if let Some(tab) = this
                                    .tabs
                                    .iter_mut()
                                    .find(|t| std::rc::Rc::ptr_eq(&t.save, &save))
                                {
                                    tab.save.persistence.journal_failed();
                                }
                                this.notifications
                                    .publish(format!("恢复副本写入失败：{error}"));
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
            }
            if !tab.save.persistence.begin() {
                continue;
            }
            let generation = self.generation;
            let save = tab.save.clone();
            let path = tab.save.path.borrow().clone();
            let baseline = tab.save.persistence.baseline().clone();
            let text = tab.save.editor.read(cx).value().to_string();
            let vault = vault.clone();
            let draft = save.draft.borrow().as_ref().map(|state| state.io.clone());
            let job = super::save_coordinator::SaveJob::new(vault, path, baseline, text, draft);
            let task = cx.background_executor().spawn(async move { job.run() });
            cx.spawn_in(window, async move |this, cx| {
                let super::save_coordinator::SaveOutcome {
                    result,
                    cleanup_error: cleanup,
                } = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation != generation {
                        return;
                    }
                    let saved = {
                        let Some(tab) = this
                            .tabs
                            .iter_mut()
                            .find(|t| std::rc::Rc::ptr_eq(&t.save, &save))
                        else {
                            return;
                        };
                        match result {
                            Ok(receipt) => {
                                if let Some(draft) = tab.save.draft.borrow_mut().as_mut() {
                                    draft.completed =
                                        if cleanup.is_none() { Some(None) } else { None };
                                }
                                if let Some(error) = cleanup {
                                    this.notifications
                                        .publish(format!("正文已保存，草稿清理将重试：{error}"));
                                }
                                let saved_path = tab.path.clone();
                                let saved_text = receipt.text.clone();
                                tab.save
                                    .persistence
                                    .saved(receipt.text, tab.save.editor.read(cx).value().as_ref());
                                if !this.files.contains(&tab.path) {
                                    this.files.push(tab.path.clone());
                                    this.files.sort();
                                }
                                Some((saved_path, saved_text))
                            }
                            Err(error) => {
                                this.notifications.publish(error.to_string());
                                tab.save.persistence.failed(
                                    matches!(
                                        error,
                                        VaultError::Conflict { .. }
                                            | VaultError::RaceConflict { .. }
                                    ),
                                    this.notifications.text().to_owned(),
                                );
                                None
                            }
                        }
                    };
                    if let Some((path, text)) = saved {
                        this.note_indexed_change(path, text, cx);
                        this.schedule_auto_sync(true);
                    }
                    this.finish_pending_closes(window, cx);
                    this.finish_pending_navigation(window, cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }
    pub(super) fn save_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active.and_then(|i| self.tabs.get(i)).map(|tab| tab.id) else {
            return;
        };
        if self.has_pending_input(id, window, cx) {
            self.notifications
                .publish("请完成当前编辑后再另存。".into());
            cx.notify();
            return;
        }
        self.document_view_changed(id, window, cx);
        self.sync_from_split(window, cx);
        let tab = self.tabs.iter().find(|tab| tab.id == id).unwrap();
        if tab.save.persistence.is_saving() {
            return;
        }
        let document = tab.save.clone();
        let mut path = document.path.borrow().clone();
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        path.set_file_name(format!("{stem}-副本-{timestamp}.md"));
        self.relocate_document(&document, path);
        document.persistence.prepare_copy();
        self.sync_reference_contexts(cx);
        self.save_all(window, cx);
        cx.notify();
    }
}
