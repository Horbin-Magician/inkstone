//! Local workspace preferences. Note contents always remain in the vault.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub query: String,
    pub case_sensitive: bool,
    pub sort_by: crate::file_order::SortBy,
    pub descending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}
impl ThemeMode {
    pub fn is_light(self, system_light: bool) -> bool {
        match self {
            Self::System => system_light,
            Self::Light => true,
            Self::Dark => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub block: usize,
    pub offset: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub path: PathBuf,
    pub pinned: Option<bool>,
    pub reading: bool,
    pub live: bool,
    pub selection: std::ops::Range<usize>,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub folded_lines: Vec<usize>,
    pub reading_position: Option<ReadingPosition>,
    pub callout_states: std::collections::BTreeMap<u64, bool>,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            pinned: None,
            reading: false,
            live: true,
            selection: 0..0,
            scroll_x: 0.,
            scroll_y: 0.,
            folded_lines: vec![],
            reading_position: None,
            callout_states: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub saved_searches: Vec<SavedSearch>,
    pub backup: crate::vault::backup::Preferences,
    pub tags: crate::tags::Options,
    pub property_types: std::collections::BTreeMap<String, crate::properties::Kind>,
    pub hotkeys: std::collections::BTreeMap<usize, Vec<String>>,
    pub locations: crate::locations::Locations,
    pub always_update_links: bool,
    pub link_format: crate::locations::LinkFormat,
    pub use_markdown_links: bool,
    pub auto_reveal_file: bool,
    pub light: bool,
    pub theme: ThemeMode,
    pub font_size: f32,
    pub quick_font_size: bool,
    pub interface_font: String,
    pub text_font: String,
    pub monospace_font: String,
    pub line_numbers: bool,
    pub readable_width: bool,
    pub strict_line_breaks: bool,
    pub default_live_preview: bool,
    pub default_reading: bool,
    pub use_tabs: bool,
    pub show_indent_guides: bool,
    pub auto_pair_brackets: bool,
    pub auto_pair_markdown: bool,
    pub smart_lists: bool,
    pub fold_headings: bool,
    pub fold_indentation: bool,
    pub tab_size: usize,
    pub show_inline_title: bool,
    pub show_view_header: bool,
    pub expanded_folders: Vec<PathBuf>,
    pub left_open: bool,
    pub right_open: bool,
    pub left_panel: usize,
    pub right_panel: usize,
    pub search_query: String,
    pub search_case_sensitive: bool,
    pub search_sort_by: crate::file_order::SortBy,
    pub search_descending: bool,
    pub left_width: f32,
    pub right_width: f32,
    pub sort_descending: bool,
    pub sort_by: crate::file_order::SortBy,
    pub bookmarks: Vec<PathBuf>,
    pub pinned_paths: Vec<PathBuf>,
    pub open_paths: Vec<PathBuf>,
    pub active_path: Option<PathBuf>,
    pub active_tab_index: Option<usize>,
    pub views: Vec<ViewState>,
    pub split_view: Option<ViewState>,
    pub main_path: Option<PathBuf>,
    pub main_tab_index: Option<usize>,
    pub split_source_tab_index: Option<usize>,
    pub split_vertical: bool,
    pub split_focused: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            saved_searches: vec![],
            tags: Default::default(),
            backup: Default::default(),
            property_types: Default::default(),
            hotkeys: Default::default(),
            locations: Default::default(),
            always_update_links: false,
            link_format: Default::default(),
            use_markdown_links: false,
            auto_reveal_file: false,
            light: false,
            theme: ThemeMode::System,
            font_size: 16.,
            quick_font_size: false,
            interface_font: String::new(),
            text_font: String::new(),
            monospace_font: String::new(),
            line_numbers: false,
            readable_width: true,
            strict_line_breaks: false,
            default_live_preview: true,
            default_reading: false,
            use_tabs: true,
            show_indent_guides: true,
            auto_pair_brackets: true,
            auto_pair_markdown: true,
            smart_lists: true,
            fold_headings: true,
            fold_indentation: true,
            tab_size: 4,
            show_inline_title: true,
            show_view_header: true,
            expanded_folders: vec![],
            left_open: true,
            right_open: true,
            left_panel: 0,
            right_panel: 0,
            search_query: String::new(),
            search_case_sensitive: false,
            search_sort_by: Default::default(),
            search_descending: false,
            left_width: 250.,
            right_width: 260.,
            sort_descending: false,
            sort_by: Default::default(),
            bookmarks: vec![],
            pinned_paths: vec![],
            open_paths: vec![],
            active_path: None,
            active_tab_index: None,
            views: vec![],
            split_view: None,
            main_path: None,
            main_tab_index: None,
            split_source_tab_index: None,
            split_vertical: false,
            split_focused: false,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> Self {
        Self::load_with_warning(path).0
    }
    pub fn load_with_warning(path: &Path) -> (Self, Option<String>) {
        let valid_json = |path: &Path| -> Option<serde_json::Value> {
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            serde_json::from_value::<Self>(json.clone()).ok()?;
            Some(json)
        };
        let main = valid_json(path);
        let backup = if main.is_none() {
            valid_json(&path.with_extension("backup"))
        } else {
            None
        };
        let warning = if backup.is_some() {
            Some("工作区配置无法读取，已从备份恢复；原配置会保留供检查。".into())
        } else if main.is_none() && (path.exists() || path.with_extension("backup").exists()) {
            Some("工作区配置和备份无法读取，已使用默认设置；原文件会保留供检查。".into())
        } else {
            None
        };
        let saved = main.or(backup);
        let mut value: Self = saved
            .as_ref()
            .and_then(|json| serde_json::from_value(json.clone()).ok())
            .unwrap_or_default();
        if saved
            .as_ref()
            .is_some_and(|json| json.get("theme").is_none() && json.get("light").is_some())
        {
            value.theme = if value.light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
        }
        value.font_size = value.font_size.clamp(10., 30.);
        value.tab_size = value.tab_size.clamp(2, 8);
        value.left_width = value.left_width.clamp(180., 500.);
        value.right_width = value.right_width.clamp(180., 500.);
        if value.left_panel > 2 {
            value.left_panel = 0;
        }
        if value.right_panel > 4 {
            value.right_panel = 0;
        }
        (value, warning)
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // A retained backup keeps the previous session available if replacement fails.
        let pending = path.with_extension("pending");
        std::fs::write(&pending, serde_json::to_vec_pretty(self)?)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&pending)?
            .sync_all()?;
        if path.exists() {
            let bytes = std::fs::read(path)?;
            if serde_json::from_slice::<Self>(&bytes).is_ok() {
                std::fs::copy(path, path.with_extension("backup"))?;
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(path.with_extension("backup"))?
                    .sync_all()?;
            } else {
                // Never replace the last valid backup with a corrupt main file.
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let corrupt =
                    path.with_extension(format!("corrupt-{}-{stamp}", std::process::id()));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(corrupt)?;
                std::io::Write::write_all(&mut file, &bytes)?;
                file.sync_all()?;
            }
        }
        std::fs::rename(pending, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

#[derive(Clone, Default, Debug)]
pub struct Navigation {
    pub entries: Vec<NavigationEntry>,
    pub cursor: usize,
}
#[derive(Clone, Debug)]
pub struct NavigationEntry {
    pub path: PathBuf,
    pub state: Option<ViewState>,
}
impl Navigation {
    pub fn record_at(&mut self, index: usize, state: ViewState) {
        if let Some(entry) = self.entries.get_mut(index)
            && entry.path == state.path
        {
            entry.state = Some(state);
        }
    }
    pub fn current_state(&self) -> Option<&ViewState> {
        self.entries.get(self.cursor)?.state.as_ref()
    }
    pub fn visit(&mut self, path: PathBuf) {
        if self
            .entries
            .get(self.cursor)
            .is_some_and(|entry| entry.path == path)
        {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(NavigationEntry { path, state: None });
        if self.entries.len() > 200 {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }
    pub fn back(&mut self) -> Option<PathBuf> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.entries
            .get(self.cursor)
            .map(|entry| entry.path.clone())
    }
    pub fn forward(&mut self) -> Option<PathBuf> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.entries
            .get(self.cursor)
            .map(|entry| entry.path.clone())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn corrupt_config_recovers_backup_and_preserves_damaged_bytes() {
        let root =
            std::env::temp_dir().join(format!("inkstone-config-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("workspace.json");
        let prefs = Preferences {
            font_size: 23.,
            bookmarks: vec!["important.md".into()],
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        prefs.save(&path).unwrap();
        std::fs::write(&path, b"broken-json").unwrap();
        let (recovered, warning) = Preferences::load_with_warning(&path);
        assert_eq!(recovered.font_size, 23.);
        assert_eq!(recovered.bookmarks, prefs.bookmarks);
        assert!(warning.is_some());
        recovered.save(&path).unwrap();
        assert_eq!(
            Preferences::load(&path.with_extension("backup")).font_size,
            23.
        );
        assert!(std::fs::read_dir(&root).unwrap().any(|e| {
            let path = e.unwrap().path();
            path.extension()
                .is_some_and(|e| e.to_string_lossy().starts_with("corrupt-"))
                && std::fs::read(path).unwrap() == b"broken-json"
        }));
        assert!(Preferences::load_with_warning(&path).1.is_none());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(Preferences::load(&path).font_size, 23.);
        std::fs::write(&path, r#"{"font_size":"wrong type"}"#).unwrap();
        assert_eq!(Preferences::load(&path).font_size, 23.);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removed_feature_settings_do_not_reset_existing_workspace() {
        let path = std::env::temp_dir().join(format!(
            "inkstone-legacy-features-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{
                "daily": {"folder": "日记", "format": "YYYY-MM-DD"},
                "templates": {"folder": "模板"},
                "graph": {"node_size": 2},
                "local_graph": {"depth": 3},
                "font_size": 21,
                "open_paths": ["笔记.md"],
                "bookmarks": ["收藏.md"],
                "show_ribbon": true,
                "ribbon_commands": [43, 39, 2]
            }"#,
        )
        .unwrap();
        let prefs = Preferences::load(&path);
        assert_eq!(prefs.font_size, 21.);
        assert_eq!(prefs.open_paths, [PathBuf::from("笔记.md")]);
        assert_eq!(prefs.bookmarks, [PathBuf::from("收藏.md")]);
        prefs.save(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for removed in [
            "daily",
            "templates",
            "graph",
            "local_graph",
            "show_ribbon",
            "ribbon_commands",
        ] {
            assert!(saved.get(removed).is_none());
        }
        assert_eq!(Preferences::load(&path).open_paths, prefs.open_paths);
        std::fs::remove_file(path.with_extension("backup")).unwrap();
        std::fs::remove_file(path).unwrap();
    }
    use super::*;
    #[test]
    fn theme_migration_preserves_manual_choices_and_persists_system_mode() {
        let path = std::env::temp_dir().join(format!("inkstone-theme-{}.json", std::process::id()));
        for (json, expected) in [
            (r#"{"light":true}"#, ThemeMode::Light),
            (r#"{"light":false}"#, ThemeMode::Dark),
            (r#"{}"#, ThemeMode::System),
        ] {
            std::fs::write(&path, json).unwrap();
            assert_eq!(Preferences::load(&path).theme, expected);
        }
        let prefs = Preferences {
            light: true,
            theme: ThemeMode::System,
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).theme, ThemeMode::System);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("backup")).unwrap();
    }
    #[test]
    fn navigation_discards_forward_branch_and_deduplicates() {
        let mut h = Navigation::default();
        h.visit("a.md".into());
        h.visit("b.md".into());
        h.visit("b.md".into());
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.back(), Some("a.md".into()));
        h.visit("c.md".into());
        assert_eq!(h.forward(), None);
        assert_eq!(h.back(), Some("a.md".into()));
        assert_eq!(h.forward(), Some("c.md".into()));
    }
    #[test]
    fn navigation_states_belong_to_each_visit_and_survive_branching() {
        let mut history = Navigation::default();
        history.visit("a.md".into());
        history.record_at(
            0,
            ViewState {
                path: "a.md".into(),
                selection: 2..4,
                ..Default::default()
            },
        );
        history.visit("b.md".into());
        history.visit("a.md".into());
        history.record_at(
            2,
            ViewState {
                path: "a.md".into(),
                selection: 9..9,
                ..Default::default()
            },
        );
        history.back();
        history.back();
        assert_eq!(history.current_state().unwrap().selection, 2..4);
        let mut independent = history.clone();
        independent.record_at(
            0,
            ViewState {
                path: "a.md".into(),
                selection: 6..6,
                ..Default::default()
            },
        );
        assert_eq!(history.current_state().unwrap().selection, 2..4);
        history.visit("c.md".into());
        assert!(history.current_state().is_none());
        assert!(history.forward().is_none());
        history.back();
        assert_eq!(history.current_state().unwrap().selection, 2..4);
        history.record_at(
            0,
            ViewState {
                path: "wrong.md".into(),
                ..Default::default()
            },
        );
        assert_eq!(history.current_state().unwrap().selection, 2..4);
        for i in 0..205 {
            history.visit(format!("{i}.md").into());
        }
        assert_eq!(history.entries.len(), 200);
        assert_eq!(history.entries[0].path, PathBuf::from("5.md"));
    }
    #[test]
    fn old_preferences_accept_new_defaults() {
        let p: Preferences = serde_json::from_str(r#"{"font_size":20}"#).unwrap();
        assert!(p.left_open);
        assert!(!p.always_update_links);
        assert!(!p.use_markdown_links);
        assert!(!p.auto_reveal_file);
        assert!(p.show_inline_title);
        assert!(p.show_view_header);
        assert!(p.show_indent_guides);
        assert!(p.auto_pair_brackets && p.auto_pair_markdown);
        assert!(p.smart_lists);
        assert!(!p.strict_line_breaks);
        assert!(p.fold_headings && p.fold_indentation);
        assert_eq!(p.link_format, crate::locations::LinkFormat::Shortest);
        assert_eq!(p.font_size, 20.);
        assert!(!p.quick_font_size);
    }
    #[test]
    fn preferences_replace_existing_file_and_retain_previous_backup() {
        let root = std::env::temp_dir().join(format!("inkstone-prefs-{}", std::process::id()));
        let path = root.join("workspace.json");
        let mut prefs = Preferences::default();
        prefs.save(&path).unwrap();
        prefs.font_size = 10.;
        prefs.quick_font_size = true;
        prefs.interface_font = "Segoe UI".into();
        prefs.text_font = "Microsoft YaHei UI".into();
        prefs.monospace_font = "Consolas".into();
        prefs.default_live_preview = false;
        prefs.default_reading = true;
        prefs.use_tabs = false;
        prefs.show_indent_guides = false;
        prefs.auto_pair_brackets = false;
        prefs.auto_pair_markdown = false;
        prefs.smart_lists = false;
        prefs.strict_line_breaks = true;
        prefs.fold_headings = false;
        prefs.fold_indentation = false;
        prefs.tab_size = 6;
        prefs.search_case_sensitive = true;
        prefs.search_sort_by = crate::file_order::SortBy::Modified;
        prefs.search_descending = true;
        prefs.tags.hierarchy = false;
        prefs.tags.sort = crate::tags::Sort::NameDescending;
        prefs.tags.collapsed.insert("work".into());
        prefs.tags.show_filter = true;
        prefs.tags.query = "项目".into();
        prefs.left_panel = 2;
        prefs.right_panel = 4;
        prefs
            .property_types
            .insert("code".into(), crate::properties::Kind::Text);
        prefs.always_update_links = true;
        prefs.use_markdown_links = true;
        prefs.auto_reveal_file = true;
        prefs.show_inline_title = false;
        prefs.show_view_header = false;
        prefs.link_format = crate::locations::LinkFormat::Relative;
        prefs.sort_by = crate::file_order::SortBy::Created;
        prefs.sort_descending = true;
        prefs.locations.notes = crate::locations::Location::Folder;
        prefs.locations.note_folder = "收件箱".into();
        prefs.locations.attachments = crate::locations::Location::Subfolder;
        prefs.locations.attachment_folder = "media".into();
        prefs.open_paths = vec!["中文.md".into()];
        prefs.views = vec![ViewState {
            path: "中文.md".into(),
            reading_position: Some(ReadingPosition {
                block: 20,
                offset: 7.,
            }),
            ..Default::default()
        }];
        prefs.views[0].callout_states.insert(42, true);
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).font_size, 10.);
        assert!(Preferences::load(&path).quick_font_size);
        assert_eq!(Preferences::load(&path).interface_font, "Segoe UI");
        assert_eq!(Preferences::load(&path).text_font, "Microsoft YaHei UI");
        assert_eq!(Preferences::load(&path).monospace_font, "Consolas");
        assert!(!Preferences::load(&path).default_live_preview);
        assert!(Preferences::load(&path).default_reading);
        assert!(!Preferences::load(&path).use_tabs);
        assert!(!Preferences::load(&path).show_indent_guides);
        assert!(!Preferences::load(&path).auto_pair_brackets);
        assert!(!Preferences::load(&path).auto_pair_markdown);
        assert!(!Preferences::load(&path).smart_lists);
        assert!(Preferences::load(&path).strict_line_breaks);
        assert!(!Preferences::load(&path).fold_headings);
        assert!(!Preferences::load(&path).fold_indentation);
        assert_eq!(Preferences::load(&path).tab_size, 6);
        assert!(Preferences::load(&path).search_case_sensitive);
        assert_eq!(
            Preferences::load(&path).search_sort_by,
            crate::file_order::SortBy::Modified
        );
        assert!(Preferences::load(&path).search_descending);
        assert!(!Preferences::load(&path).tags.hierarchy);
        assert_eq!(
            Preferences::load(&path).tags.sort,
            crate::tags::Sort::NameDescending
        );
        assert!(Preferences::load(&path).tags.collapsed.contains("work"));
        assert!(Preferences::load(&path).tags.show_filter);
        assert_eq!(Preferences::load(&path).tags.query, "项目");
        assert_eq!(Preferences::load(&path).left_panel, 2);
        assert_eq!(Preferences::load(&path).right_panel, 4);
        assert_eq!(
            Preferences::load(&path).property_types["code"],
            crate::properties::Kind::Text
        );
        assert_eq!(
            Preferences::load(&path).views[0].callout_states.get(&42),
            Some(&true)
        );
        assert_eq!(
            Preferences::load(&path).views[0].reading_position,
            Some(ReadingPosition {
                block: 20,
                offset: 7.
            })
        );
        assert!(Preferences::load(&path).always_update_links);
        assert!(Preferences::load(&path).use_markdown_links);
        assert!(Preferences::load(&path).auto_reveal_file);
        assert!(!Preferences::load(&path).show_inline_title);
        assert!(!Preferences::load(&path).show_view_header);
        assert_eq!(
            Preferences::load(&path).link_format,
            crate::locations::LinkFormat::Relative
        );
        assert_eq!(
            Preferences::load(&path).sort_by,
            crate::file_order::SortBy::Created
        );
        assert!(Preferences::load(&path).sort_descending);
        assert_eq!(Preferences::load(&path).locations.note_folder, "收件箱");
        assert_eq!(
            Preferences::load(&path).locations.attachments,
            crate::locations::Location::Subfolder
        );
        assert_eq!(
            Preferences::load(&path).locations.attachment_folder,
            "media"
        );
        assert_eq!(
            Preferences::load(&path).open_paths,
            vec![PathBuf::from("中文.md")]
        );
        assert_eq!(
            Preferences::load(&path.with_extension("backup")).font_size,
            16.
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
