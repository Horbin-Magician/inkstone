use super::*;
use gpui_component::{
    Disableable,
    button::Button,
    menu::{DropdownMenu, PopupMenuItem},
};
use inkstone::preferences::SavedSearch;

impl Workspace {
    fn current_saved_search(&self, cx: &App) -> SavedSearch {
        SavedSearch {
            query: self.search.read(cx).value().to_string(),
            case_sensitive: self.ui.prefs.search_case_sensitive,
            sort_by: self.ui.prefs.search_sort_by,
            descending: self.ui.prefs.search_descending,
        }
    }
    fn toggle_saved_search(&mut self, cx: &mut Context<Self>) {
        let entry = self.current_saved_search(cx);
        if entry.query.trim().is_empty()
            || inkstone::search::Query::parse_with_case(&entry.query, entry.case_sensitive).is_err()
        {
            return;
        }
        if let Some(i) = self
            .ui
            .prefs
            .saved_searches
            .iter()
            .position(|s| s == &entry)
        {
            self.ui.prefs.saved_searches.remove(i);
        } else if self.ui.prefs.saved_searches.len() < 100 {
            self.ui.prefs.saved_searches.push(entry);
        } else {
            self.status = "已保存 100 个搜索，请先取消不再使用的搜索。".into();
        }
        self.persist_workspace(cx);
        cx.notify();
    }
    fn apply_saved_search(
        &mut self,
        entry: SavedSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.prefs.search_case_sensitive = entry.case_sensitive;
        self.ui.prefs.search_sort_by = entry.sort_by;
        self.ui.prefs.search_descending = entry.descending;
        self.focus_search(true, window, cx);
        self.search
            .update(cx, |s, cx| s.set_value(entry.query, window, cx));
        self.run_search(cx);
        self.persist_workspace(cx);
    }
    pub(super) fn saved_search_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let saved = self.ui.prefs.saved_searches.clone();
        let selected = self.current_saved_search(cx);
        let exists = saved.contains(&selected);
        let weak = cx.entity().downgrade();
        div()
            .px_3()
            .pt_2()
            .flex()
            .gap_1()
            .child(
                Button::new("save-search")
                    .compact()
                    .label(if exists {
                        "取消保存"
                    } else {
                        "保存搜索"
                    })
                    .disabled(
                        self.ui.quick_open
                            || selected.query.trim().is_empty()
                            || !self.ui.search_error.is_empty(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_saved_search(cx))),
            )
            .child(
                Button::new("saved-searches")
                    .compact()
                    .label(format!("已保存 ({})", saved.len()))
                    .disabled(saved.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        for entry in &saved {
                            let weak = weak.clone();
                            let entry = entry.clone();
                            menu = menu.item(
                                PopupMenuItem::new(entry.query.clone())
                                    .checked(entry == selected)
                                    .on_click(move |_, w, cx| {
                                        let _ = weak.update(cx, |this, cx| {
                                            this.apply_saved_search(entry.clone(), w, cx)
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn saved_search_roundtrip_includes_options_and_search_sees_unsaved_text(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-saved-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "old disk").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note("a.md".into(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("中文 UNIQUE 草稿", window, cx));
                w.fulltext = true;
                w.ui.left_mode = 1;
                w.search
                    .update(cx, |s, cx| s.set_value("UNIQUE", window, cx));
                w.ui.prefs.search_case_sensitive = true;
                w.ui.prefs.search_sort_by = inkstone::file_order::SortBy::Modified;
                w.ui.prefs.search_descending = true;
                w.run_search(cx);
                w.toggle_saved_search(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.search_results.len(), 1);
                assert!(w.search_results[0].excerpt.contains("草稿"));
                assert_eq!(w.index.notes[&PathBuf::from("a.md")].text, "old disk");
                let prefs = inkstone::preferences::Preferences::load(
                    &root.join(".inkstone-workspace.json"),
                );
                assert_eq!(prefs.saved_searches.len(), 1);
                let saved = prefs.saved_searches[0].clone();
                assert!(saved.case_sensitive && saved.descending);
                w.search
                    .update(cx, |s, cx| s.set_value("old disk", window, cx));
                w.run_search(cx);
                w.apply_saved_search(saved, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.search_results.len(), 1);
                assert_eq!(w.search.read(cx).value().as_ref(), "UNIQUE");
                w.toggle_saved_search(cx);
                assert!(w.ui.prefs.saved_searches.is_empty());
                w.watcher = None;
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("a.md")).unwrap(),
            "old disk"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
