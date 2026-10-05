//! Search focus, asynchronous query results, and pagination.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct Jobs {
    wait: Option<Task<()>>,
    open_after: Option<AnyWindowHandle>,
    cancellation: Option<Arc<AtomicBool>>,
}
impl Jobs {
    pub fn cancel(&mut self) {
        self.wait = None;
        self.open_after = None;
        if let Some(token) = self.cancellation.take() {
            token.store(true, Ordering::Release);
        }
    }
}
impl Drop for Jobs {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Workspace {
    pub(super) fn submit_search_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.read(cx).is_composing() {
            return;
        }
        if self.ui.search_loading {
            if self.search_jobs.wait.is_some() {
                self.run_search(cx);
            }
            if self.ui.search_loading {
                self.search_jobs.open_after = Some(window.window_handle());
            }
        } else {
            self.open_selected_result(window, cx);
        }
    }
    pub(super) fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_jobs.cancel();
        self.search_revision += 1;
        self.search_results.clear();
        self.ui.search_has_more = false;
        if self.search.read(cx).is_composing() {
            self.ui.search_loading = false;
            cx.notify();
            return;
        }
        if self.search.read(cx).value().trim().is_empty() {
            self.run_search(cx);
            return;
        }
        self.ui.search_loading = true;
        let generation = self.generation;
        self.search_jobs.wait = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.run_search(cx);
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn focus_search(
        &mut self,
        fulltext: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fulltext && !self.ui.quick_open {
            self.ui.prefs.search_query = self.search.read(cx).value().to_string();
        }
        self.fulltext = fulltext;
        let value = if fulltext {
            self.ui.prefs.search_query.clone()
        } else {
            String::new()
        };
        self.search
            .update(cx, |s, cx| s.set_value(value, window, cx));
        if fulltext {
            self.ui.focus_mode = false;
            self.ui.left_mode = 1;
            self.ui.prefs.left_open = true;
        }
        self.ui.quick_open = !fulltext;
        self.ui.selected = 0;
        self.command_open = false;
        self.ui.modal_scroll.set_offset(Point::default());
        self.search.update(cx, |state, cx| state.focus(window, cx));
        self.run_search(cx);
        cx.notify();
    }
    pub(super) fn close_quick_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.quick_open {
            return;
        }
        self.ui.quick_open = false;
        self.fulltext = true;
        let query = self.ui.prefs.search_query.clone();
        self.search
            .update(cx, |s, cx| s.set_value(query, window, cx));
        self.run_search(cx);
    }
    pub(super) fn run_search(&mut self, cx: &mut Context<Self>) {
        self.search_jobs.cancel();
        self.ui.search_drafts_changed = false;
        let previous_error = std::mem::take(&mut self.ui.search_error);
        self.notifications.clear_matching(&previous_error);
        self.search_revision += 1;
        let revision = self.search_revision;
        let generation = self.generation;
        let query = self.search.read(cx).value().to_string();
        if self.fulltext && !self.ui.quick_open {
            if self.ui.search_group_query != query {
                self.ui.search_collapsed.clear();
                self.ui.search_group_query = query.clone();
                self.ui.search_group_scroll.set_offset(Point::default());
            }
            self.ui.prefs.search_query = query.clone();
        }
        let index = self.index.clone();
        let drafts: std::collections::BTreeMap<_, _> = self
            .tabs
            .iter()
            .filter(|t| !t.save.editor.read(cx).is_composing())
            .filter_map(|t| {
                let text = t.save.editor.read(cx).value();
                (index
                    .notes
                    .get(&t.path)
                    .is_none_or(|n| n.text.as_str() != text.as_ref()))
                .then(|| (t.path.clone(), text))
            })
            .collect();
        let fulltext = self.fulltext;
        let case_sensitive = self.ui.prefs.search_case_sensitive;
        let sort_by = self.ui.prefs.search_sort_by;
        let descending = self.ui.prefs.search_descending;
        if fulltext && !self.ui.quick_open {
            let signature = (query.clone(), case_sensitive, sort_by, descending);
            if self.ui.search_signature.as_ref() != Some(&signature) {
                self.ui.search_signature = Some(signature);
                self.ui.search_limit = 200;
                self.ui.search_has_more = false;
            }
        }
        let limit = self.ui.search_limit;
        self.ui.search_loading = true;
        if fulltext
            && let Err(error) =
                inkstone_core::search::Query::parse_with_case(&query, case_sensitive)
        {
            self.search_results.clear();
            self.ui.search_loading = false;
            self.ui.search_has_more = false;
            self.ui.search_error = error.clone();
            self.notifications.publish(error);
            cx.notify();
            return;
        }
        let active = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        let token = Arc::new(AtomicBool::new(false));
        self.search_jobs.cancellation = Some(token.clone());
        let task = cx.background_executor().spawn(async move {
            let cancelled = || token.load(Ordering::Acquire);
            if cancelled() {
                return None;
            }
            let mut index = index;
            if !drafts.is_empty() {
                let index = Arc::make_mut(&mut index);
                for (path, text) in drafts {
                    if cancelled() {
                        return None;
                    }
                    index.update(path, text.to_string());
                }
            }
            if cancelled() {
                return None;
            }
            let backlinks = active.map(|p| index.backlinks(&p)).unwrap_or_default();
            let hits = if query.trim().is_empty() {
                Some(vec![])
            } else if fulltext {
                index
                    .search_limited_cancellable(
                        &query,
                        case_sensitive,
                        sort_by,
                        descending,
                        limit.saturating_add(1),
                        cancelled,
                    )
                    .unwrap_or_default()
            } else {
                index.filenames_cancellable(&query, cancelled)
            };
            let mut hits = hits?;
            if cancelled() {
                return None;
            }
            let more = fulltext && hits.len() > limit;
            if more {
                hits.truncate(limit);
            }
            Some((hits, backlinks, more))
        });
        cx.spawn(async move |this, cx| {
            let Some((hits, backlinks, more)) = task.await else {
                return;
            };
            let open = this
                .update(cx, |this, cx| {
                    if this.search_revision == revision && this.generation == generation {
                        this.search_results = hits;
                        this.ui.search_loading = false;
                        this.ui.search_has_more = more;
                        if this.backlinks != backlinks {
                            this.backlink_scroll.scroll_to_item(0, ScrollStrategy::Top);
                        }
                        this.backlinks = backlinks;
                        cx.notify();
                        return this.search_jobs.open_after.take();
                    }
                    None
                })
                .ok()
                .flatten();
            if let Some(window) = open {
                let _ = cx.update_window(window, |_, window, cx| {
                    this.update(cx, |this, cx| this.open_selected_result(window, cx))
                });
            }
        })
        .detach();
    }
    pub(super) fn load_more_search(&mut self, cx: &mut Context<Self>) {
        if !self.fulltext
            || self.ui.quick_open
            || self.ui.search_loading
            || !self.ui.search_has_more
        {
            return;
        }
        self.ui.search_limit = self.ui.search_limit.saturating_add(200);
        self.run_search(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn typing_debounces_latest_query_cancels_old_work_and_clears_immediately(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let old = handle
            .update(cx, |w, window, cx| {
                Arc::make_mut(&mut w.index).update("old.md".into(), "old text".into());
                Arc::make_mut(&mut w.index).update("latest.md".into(), "latest 中文😀".into());
                w.fulltext = true;
                w.search.update(cx, |s, cx| s.set_value("old", window, cx));
                w.run_search(cx);
                let old = w.search_jobs.cancellation.clone().unwrap();
                w.search.update(cx, |s, cx| {
                    s.set_value("lat", window, cx);
                    cx.emit(InputEvent::Change);
                });
                old
            })
            .unwrap();
        cx.run_until_parked();
        assert!(old.load(Ordering::Acquire));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.search_results.is_empty());
                assert!(
                    w.search_jobs.cancellation.is_none(),
                    "no worker starts during debounce"
                );
                w.search.update(cx, |s, cx| {
                    s.set_value("latest", window, cx);
                    cx.emit(InputEvent::Change);
                });
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.search_results.is_empty());
                assert!(w.ui.search_loading);
                assert!(w.search_jobs.cancellation.is_none());
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.search_results.len(), 1);
                assert_eq!(w.search_results[0].path, PathBuf::from("latest.md"));
                assert!(!w.ui.search_loading);
                w.search.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                    cx.emit(InputEvent::Change);
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.search_results.is_empty());
                assert!(!w.ui.search_loading);
                assert!(w.search_jobs.wait.is_none());
            })
            .unwrap();
    }
    #[gpui::test]
    fn enter_flushes_debounce_but_new_input_cancels_pending_open(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-search-enter-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/latest.md"), "latest").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(root.join("vault"), root.join("recovery")).unwrap());
                Arc::make_mut(&mut w.index).update("latest.md".into(), "latest".into());
                w.fulltext = true;
                w.search
                    .update(cx, |s, cx| s.set_value("latest", window, cx));
                w.schedule_search(cx);
                w.submit_search_result(window, cx);
                assert!(w.search_jobs.wait.is_none());
                assert!(w.search_jobs.open_after.is_some());
                w.search
                    .update(cx, |s, cx| s.set_value("missing", window, cx));
                w.schedule_search(cx);
                assert!(w.search_jobs.open_after.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.tabs.is_empty());
                w.search
                    .update(cx, |s, cx| s.set_value("latest", window, cx));
                w.schedule_search(cx);
                w.submit_search_result(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(w.tabs[0].path, PathBuf::from("latest.md"));
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
