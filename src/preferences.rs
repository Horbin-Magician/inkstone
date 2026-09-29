//! Local workspace preferences. Note contents always remain in the vault.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadingPosition {
    pub block: usize,
    pub offset: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub path: PathBuf,
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
    pub daily: crate::daily::Settings,
    pub hotkeys: std::collections::BTreeMap<usize, Vec<String>>,
    pub graph: crate::graph::Options,
    pub local_graph: crate::graph::Options,
    pub locations: crate::locations::Locations,
    pub always_update_links: bool,
    pub link_format: crate::locations::LinkFormat,
    pub use_markdown_links: bool,
    pub auto_reveal_file: bool,
    pub light: bool,
    pub font_size: f32,
    pub line_numbers: bool,
    pub readable_width: bool,
    pub show_inline_title: bool,
    pub show_view_header: bool,
    pub expanded_folders: Vec<PathBuf>,
    pub left_open: bool,
    pub right_open: bool,
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
    pub split_vertical: bool,
    pub split_focused: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            daily: Default::default(),
            hotkeys: Default::default(),
            graph: Default::default(),
            local_graph: Default::default(),
            locations: Default::default(),
            always_update_links: false,
            link_format: Default::default(),
            use_markdown_links: false,
            auto_reveal_file: false,
            light: false,
            font_size: 16.,
            line_numbers: false,
            readable_width: true,
            show_inline_title: true,
            show_view_header: true,
            expanded_folders: vec![],
            left_open: true,
            right_open: true,
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
            split_vertical: false,
            split_focused: false,
        }
    }
}
impl Preferences {
    pub fn load(path: &Path) -> Self {
        let mut value: Self = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        value.font_size = value.font_size.clamp(12., 30.);
        value.graph.normalize();
        value.local_graph.normalize();
        value.left_width = value.left_width.clamp(180., 500.);
        value.right_width = value.right_width.clamp(180., 500.);
        value
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
            std::fs::copy(path, path.with_extension("backup"))?;
        }
        std::fs::rename(pending, path)
    }
}

#[derive(Default, Debug)]
pub struct Navigation {
    pub entries: Vec<PathBuf>,
    pub cursor: usize,
}
impl Navigation {
    pub fn visit(&mut self, path: PathBuf) {
        if self.entries.get(self.cursor) == Some(&path) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(path);
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
        self.entries.get(self.cursor).cloned()
    }
    pub fn forward(&mut self) -> Option<PathBuf> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn old_preferences_accept_new_defaults() {
        let p: Preferences = serde_json::from_str(r#"{"font_size":20}"#).unwrap();
        assert!(p.left_open);
        assert!(!p.always_update_links);
        assert!(!p.use_markdown_links);
        assert!(!p.auto_reveal_file);
        assert!(p.show_inline_title);
        assert!(p.show_view_header);
        assert_eq!(p.link_format, crate::locations::LinkFormat::Shortest);
        assert_eq!(p.font_size, 20.);
    }
    #[test]
    fn preferences_replace_existing_file_and_retain_previous_backup() {
        let root = std::env::temp_dir().join(format!("inkstone-prefs-{}", std::process::id()));
        let path = root.join("workspace.json");
        let mut prefs = Preferences::default();
        prefs.save(&path).unwrap();
        prefs.font_size = 23.;
        prefs.daily.folder = "日记".into();
        prefs.daily.format = "YYYY/MM/DD".into();
        prefs.daily.template = "模板/日记.md".into();
        prefs.always_update_links = true;
        prefs.use_markdown_links = true;
        prefs.auto_reveal_file = true;
        prefs.show_inline_title = false;
        prefs.show_view_header = false;
        prefs.link_format = crate::locations::LinkFormat::Relative;
        prefs.sort_by = crate::file_order::SortBy::Created;
        prefs.sort_descending = true;
        prefs.graph.node_size = 2.;
        prefs.graph.arrows = true;
        prefs.graph.link_distance = 2.2;
        prefs.locations.notes = crate::locations::Location::Folder;
        prefs.locations.note_folder = "收件箱".into();
        prefs.locations.attachments = crate::locations::Location::Subfolder;
        prefs.locations.attachment_folder = "media".into();
        prefs.local_graph.repel_force = 4.;
        prefs.graph.groups = vec![crate::graph::ColorGroup {
            query: "tag:work".into(),
            color: 0x112233,
        }];
        prefs.local_graph.depth = 3;
        prefs.local_graph.query = "#work".into();
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
        assert_eq!(Preferences::load(&path).font_size, 23.);
        assert_eq!(Preferences::load(&path).daily.folder, "日记");
        assert_eq!(Preferences::load(&path).daily.format, "YYYY/MM/DD");
        assert_eq!(Preferences::load(&path).daily.template, "模板/日记.md");
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
        assert_eq!(Preferences::load(&path).graph.node_size, 2.);
        assert!(Preferences::load(&path).graph.arrows);
        assert_eq!(Preferences::load(&path).graph.link_distance, 2.2);
        assert_eq!(Preferences::load(&path).locations.note_folder, "收件箱");
        assert_eq!(
            Preferences::load(&path).locations.attachments,
            crate::locations::Location::Subfolder
        );
        assert_eq!(
            Preferences::load(&path).locations.attachment_folder,
            "media"
        );
        assert_eq!(Preferences::load(&path).local_graph.repel_force, 4.);
        assert_eq!(Preferences::load(&path).graph.groups[0].color, 0x112233);
        assert_eq!(Preferences::load(&path).graph.groups[0].query, "tag:work");
        assert_eq!(Preferences::load(&path).local_graph.depth, 3);
        assert_eq!(Preferences::load(&path).local_graph.query, "#work");
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
