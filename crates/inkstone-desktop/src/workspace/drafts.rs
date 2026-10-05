//! Background recovery writes never save the authoritative Markdown document.
use super::*;
use inkstone_core::vault::drafts::DraftSession;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use super::save_coordinator::DraftIo;

pub(super) struct DraftState {
    path: PathBuf,
    pub io: Arc<DraftIo>,
    observed: SharedString,
    changed: Instant,
    pending_since: Instant,
    retry: Instant,
    busy: bool,
    pub completed: Option<Option<SharedString>>,
}

impl Workspace {
    pub(super) fn tick_drafts(&mut self, cx: &mut Context<Self>) {
        self.tick_drafts_at(Instant::now(), cx);
    }

    fn tick_drafts_at(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        for tab in &self.tabs {
            let document = tab.save.clone();
            if document.saving.get() || document.editor.read(cx).is_composing() {
                continue;
            }
            let text = document.editor.read(cx).value();
            let path = document.path.borrow().clone();
            let target = document.dirty.get().then_some(text.clone());
            let mut slot = document.draft.borrow_mut();
            if slot.as_ref().is_none_or(|s| s.path != path) {
                if target.is_none() {
                    continue;
                }
                let session = match DraftSession::new(vault.clone(), path.clone()) {
                    Ok(session) => session,
                    Err(error) => {
                        self.status = format!("草稿保护无法启动：{error}");
                        continue;
                    }
                };
                *slot = Some(DraftState {
                    path,
                    io: Arc::new(DraftIo {
                        session: Mutex::new(session),
                        revision: AtomicU64::new(0),
                    }),
                    observed: text.clone(),
                    changed: now,
                    pending_since: now,
                    retry: now,
                    busy: false,
                    completed: None,
                });
            }
            let state = slot.as_mut().unwrap();
            if state.observed != text {
                if state.completed.as_ref().is_some_and(|completed| {
                    completed
                        .as_ref()
                        .is_none_or(|text| *text == state.observed)
                }) {
                    state.pending_since = now;
                }
                state.observed = text;
                state.changed = now;
            }
            if state.busy || now < state.retry || state.completed.as_ref() == Some(&target) {
                continue;
            }
            if target.is_some()
                && now.duration_since(state.changed) < Duration::from_millis(800)
                && now.duration_since(state.pending_since) < Duration::from_secs(5)
            {
                continue;
            }
            state.busy = true;
            let io = state.io.clone();
            let revision = io.revision.load(Ordering::SeqCst);
            let baseline = document.baseline.borrow().clone();
            let snapshot = target.clone();
            let worker = io.clone();
            let task = cx.background_executor().spawn(async move {
                let mut session = worker.session.lock().unwrap();
                if worker.revision.load(Ordering::SeqCst) != revision {
                    return Ok(());
                }
                match snapshot {
                    Some(text) => session.persist(baseline.as_deref(), &text),
                    None => session.clear(),
                }
            });
            let generation = self.generation;
            drop(slot);
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    let mut slot = document.draft.borrow_mut();
                    let Some(state) = slot.as_mut().filter(|s| Arc::ptr_eq(&s.io, &io)) else {
                        return;
                    };
                    state.busy = false;
                    if io.revision.load(Ordering::SeqCst) != revision {
                        return;
                    }
                    match result {
                        Ok(()) => {
                            state.completed = Some(target);
                            state.pending_since = Instant::now();
                        }
                        Err(error) => {
                            state.retry = Instant::now() + Duration::from_secs(5);
                            if this.generation == generation {
                                this.status = format!("草稿保护写入失败，将重试：{error}");
                                cx.notify();
                            }
                        }
                    }
                });
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn fixture(cx: &mut TestAppContext) -> (PathBuf, WindowHandle<Workspace>) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-draft-ui-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/note.md"), "original").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                w.add_tab("note.md".into(), Some("original".into()), false, window, cx);
            })
            .unwrap();
        (root, handle)
    }

    #[gpui::test]
    fn drafts_debounce_keep_markdown_and_clear_after_save(cx: &mut TestAppContext) {
        let (root, handle) = fixture(cx);
        let now = Instant::now();
        handle
            .update(cx, |w, window, cx| {
                let editor = w.tabs[0].save.editor.clone();
                editor.update(cx, |s, cx| s.set_value("first", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now, cx);
                editor.update(cx, |s, cx| s.set_value("latest 中文😀", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now + Duration::from_millis(500), cx);
                assert!(w.vault.as_ref().unwrap().recoveries().unwrap().is_empty());
                w.tick_drafts_at(now + Duration::from_secs(2), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let vault = w.vault.as_ref().unwrap();
                assert_eq!(vault.recoveries().unwrap()[0].record.draft, "latest 中文😀");
                assert_eq!(
                    vault
                        .read(std::path::Path::new("note.md"))
                        .unwrap()
                        .as_deref(),
                    Some("original")
                );
                w.save_all(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.vault.as_ref().unwrap().recoveries().unwrap().is_empty());
                assert!(!w.tabs[0].save.dirty.get());
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("vault/note.md")).unwrap(),
            "latest 中文😀"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn drafts_exclude_composition_and_queued_writes_cannot_resurrect(cx: &mut TestAppContext) {
        let (root, handle) = fixture(cx);
        let now = Instant::now();
        handle
            .update(cx, |w, window, cx| {
                let editor = w.tabs[0].save.editor.clone();
                editor.update(cx, |s, cx| {
                    s.set_value("committed", window, cx);
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                });
                w.tick_drafts_at(now, cx);
                w.tick_drafts_at(now + Duration::from_secs(6), cx);
                assert!(w.tabs[0].save.draft.borrow().is_none());
                editor.update(cx, |s, cx| s.set_value("committed 你", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now + Duration::from_secs(7), cx);
                w.tick_drafts_at(now + Duration::from_secs(9), cx);
                // Queue a save before the background draft completion is delivered.
                w.save_all(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.vault.as_ref().unwrap().recoveries().unwrap().is_empty());
                assert!(!w.tabs[0].save.dirty.get());
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn continuous_edits_checkpoint_and_return_to_baseline_clears(cx: &mut TestAppContext) {
        let (root, handle) = fixture(cx);
        let now = Instant::now();
        handle
            .update(cx, |w, window, cx| {
                for index in 0..=10 {
                    w.tabs[0]
                        .save
                        .editor
                        .clone()
                        .update(cx, |s, cx| s.set_value(format!("edit {index}"), window, cx));
                    w.flush_document_views(window, cx);
                    w.tick_drafts_at(now + Duration::from_millis(index * 500), cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.vault.as_ref().unwrap().recoveries().unwrap()[0]
                        .record
                        .draft,
                    "edit 10"
                );
                w.tabs[0]
                    .save
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("original", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now + Duration::from_secs(7), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.vault.as_ref().unwrap().recoveries().unwrap().is_empty())
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn draft_failure_retries_without_saving_or_losing_new_edits(cx: &mut TestAppContext) {
        let (root, handle) = fixture(cx);
        let now = Instant::now();
        std::fs::remove_dir(root.join("recovery")).unwrap();
        std::fs::write(root.join("recovery"), "blocked").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.tabs[0]
                    .save
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("first", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now, cx);
                w.tick_drafts_at(now + Duration::from_secs(2), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| assert!(w.status.contains("草稿保护写入失败")))
            .unwrap();
        std::fs::remove_file(root.join("recovery")).unwrap();
        std::fs::create_dir(root.join("recovery")).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.tabs[0]
                    .save
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("latest", window, cx));
                w.flush_document_views(window, cx);
                w.tick_drafts_at(now + Duration::from_secs(10), cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(
                    w.vault.as_ref().unwrap().recoveries().unwrap()[0]
                        .record
                        .draft,
                    "latest"
                )
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("vault/note.md")).unwrap(),
            "original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
