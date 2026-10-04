mod file_sync;
mod links;
mod note_files;
mod note_open;
mod saving;
mod search;
mod session;
mod tabs;
#[cfg(test)]
mod tests;

use file_sync::make_tree;
#[cfg(test)]
use file_sync::refresh_created_index;
mod appearance;
mod attachments;
mod backups;
mod bulk_edit;
mod cloud_sync;
mod commands;
mod conflicts;
mod document;
mod exports;
mod extras;
mod file_settings;
mod hotkeys;
mod inline_title;
mod link_health;
mod link_updates;
mod navigation;
mod recovery;
mod search_tools;
mod table_editor;
mod ui;
mod views;
mod welcome;
use crate::editor::{EditorEvent, EditorPane};
use gpui::{prelude::*, *};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    tree::{TreeItem, TreeState},
};
use inkstone_core::index::{Index, Resolution, SearchHit};
use inkstone_core::vault::{RecoveryEntry, Vault, VaultError};
use notify::{RecursiveMode, Watcher};
use std::sync::Arc;
use std::{path::PathBuf, time::Duration};
type LinkEdits = Vec<(PathBuf, String, String)>;

actions!(
    inkstone,
    [
        Save,
        OpenVault,
        QuickOpen,
        FullSearch,
        CommandPalette,
        NewNote,
        CloseTab,
        ClosePalette,
        ToggleLeft,
        ToggleRight,
        Settings,
        NavigateBack,
        NavigateForward,
        NextTab,
        PreviousTab,
        ReopenTab,
        ToggleReading,
        RenameNote,
        Bold,
        Italic,
        InsertLink,
        NewTab,
        SplitRight,
        SplitDown,
        ToggleTaskLine
    ]
);

struct Tab {
    id: usize,
    pinned: bool,
    path: PathBuf,
    pane: Entity<EditorPane>,
    save: std::rc::Rc<document::DocumentState>,
    synced_text: SharedString,
    _subscription: Option<Subscription>,
    _links: Subscription,
    _focus: Subscription,
}
pub struct Workspace {
    #[cfg(target_os = "macos")]
    quit_requested: bool,
    vault: Option<Vault>,
    files: Vec<PathBuf>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    next_id: usize,
    generation: u64,
    navigation_generation: u64,
    name: Entity<InputState>,
    status: String,
    loading: bool,
    startup_pending: bool,
    _timer: Task<()>,
    watcher: Option<notify::RecommendedWatcher>,
    watch_events: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    refresh_requested: bool,
    refreshing: bool,
    recoveries: Vec<RecoveryEntry>,
    index: Arc<Index>,
    search: Entity<InputState>,
    search_results: Vec<SearchHit>,
    search_revision: u64,
    fulltext: bool,
    _search_subscription: Subscription,
    _name_subscription: Subscription,
    _tree_subscription: Subscription,
    pending_jump: Option<(PathBuf, usize)>,
    pending_navigation: Option<navigation::PendingNavigation>,
    backlinks: Vec<PathBuf>,
    backlink_scroll: UniformListScrollHandle,
    tree: Entity<TreeState>,
    tree_files: Vec<PathBuf>,
    command_open: bool,
    changed_paths: std::collections::BTreeSet<PathBuf>,
    rescan: bool,
    structure_changed: bool,
    folder_revision: u64,
    ui: ui::UiState,
    views: views::Views,
}
#[cfg(not(test))]
fn app_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.unwrap_or_else(std::env::temp_dir).join("Inkstone")
}
#[cfg(test)]
fn app_dir() -> PathBuf {
    std::env::temp_dir().join(format!("inkstone-ui-test-{}", std::process::id()))
}
impl Workspace {
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(receiver) = &self.watch_events {
            for event in receiver.try_iter() {
                match event {
                    Ok(event) if event.need_rescan() => {
                        self.refresh_requested = true;
                        self.rescan = true;
                    }
                    Ok(event)
                        if !matches!(event.kind, notify::EventKind::Access(_))
                            && event.paths.iter().any(|p| {
                                !p.components().any(|c| {
                                    c.as_os_str().to_string_lossy().starts_with(".inkstone-")
                                })
                            }) =>
                    {
                        self.refresh_requested = true;
                        for path in event.paths {
                            if path
                                .components()
                                .any(|c| c.as_os_str().to_string_lossy().starts_with(".inkstone-"))
                            {
                                continue;
                            }
                            if matches!(event.kind, notify::EventKind::Create(_)) {
                                self.structure_changed = true;
                            }
                            if path
                                .extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                            {
                                if let Some(relative) = self
                                    .vault
                                    .as_ref()
                                    .and_then(|v| path.strip_prefix(&v.root).ok())
                                {
                                    self.changed_paths.insert(relative.to_path_buf());
                                } else {
                                    self.rescan = true;
                                }
                            } else if !matches!(event.kind, notify::EventKind::Create(_)) {
                                self.rescan = true;
                            }
                        }
                    }
                    Err(error) => {
                        self.status = format!("文件监听错误：{error}");
                        cx.notify();
                    }
                    _ => (),
                }
            }
        }
        if self.refresh_requested && !self.refreshing {
            self.refresh(window, cx);
        }
        self.save_pending(window, cx);
        if self.ui.search_drafts_changed {
            self.run_search(cx);
        }
        self.finish_pending_navigation(window, cx);
        self.finish_pending_closes(window, cx);
        self.persist_workspace(cx);
        self.tick_cloud_sync(window, cx);
        self.tick_backups(window, cx);
        self.finish_window_close(window, cx);
    }
}
