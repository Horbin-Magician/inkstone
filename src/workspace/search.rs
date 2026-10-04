//! Search focus, asynchronous query results, and pagination.

use super::*;

impl Workspace {
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
        self.ui.search_drafts_changed = false;
        let previous_error = std::mem::take(&mut self.ui.search_error);
        if !previous_error.is_empty() && self.status == previous_error {
            self.status.clear();
        }
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
            && let Err(error) = inkstone::search::Query::parse_with_case(&query, case_sensitive)
        {
            self.search_results.clear();
            self.ui.search_loading = false;
            self.ui.search_has_more = false;
            self.ui.search_error = error.clone();
            self.status = error;
            cx.notify();
            return;
        }
        let active = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        let task = cx.background_executor().spawn(async move {
            let mut index = index;
            if !drafts.is_empty() {
                let index = Arc::make_mut(&mut index);
                for (path, text) in drafts {
                    index.update(path, text.to_string());
                }
            }
            let backlinks = active.map(|p| index.backlinks(&p)).unwrap_or_default();
            let mut hits = if query.trim().is_empty() {
                vec![]
            } else if fulltext {
                index
                    .search_limited(
                        &query,
                        case_sensitive,
                        sort_by,
                        descending,
                        limit.saturating_add(1),
                    )
                    .unwrap_or_default()
            } else {
                index.filenames(&query)
            };
            let more = fulltext && hits.len() > limit;
            if more {
                hits.truncate(limit);
            }
            (hits, backlinks, more)
        });
        cx.spawn(async move |this, cx| {
            let (hits, backlinks, more) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.search_revision == revision && this.generation == generation {
                    this.search_results = hits;
                    this.ui.search_loading = false;
                    this.ui.search_has_more = more;
                    if this.backlinks != backlinks {
                        this.backlink_scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    this.backlinks = backlinks;
                    cx.notify();
                }
            });
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
