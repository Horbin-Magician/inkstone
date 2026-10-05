mod file_sync;
mod left_sidebar;
mod links;
mod modal_ui;
mod note_files;
mod note_open;
mod notifications;
mod right_sidebar;
mod save_coordinator;
mod save_state;
mod saving;
mod search;
mod session;
mod settings_save;
mod settings_ui;
mod tabs;
#[cfg(test)]
mod tests;

use file_sync::make_tree;
#[cfg(test)]
use file_sync::refresh_created_index;
mod appearance;
mod attachments;
mod backups;
mod bookmarks;
mod bulk_edit;
mod cloud_sync;
mod commands;
mod conflicts;
mod document;
mod drafts;
mod exports;
mod extras;
mod file_settings;
mod hotkeys;
mod inline_title;
mod link_health;
mod link_updates;
mod navigation;
mod recovery;
mod remote_check;
mod search_tools;
mod sync_recovery;
mod table_editor;
mod ui;
mod views;
mod webdav_secret;
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
    settings_save: settings_save::State,
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
    notifications: notifications::Notifications,
    loading: bool,
    startup_pending: bool,
    _timer: Task<()>,
    watcher: Option<notify::RecommendedWatcher>,
    watch_events: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    refresh_requested: bool,
    refreshing: bool,
    recoveries: Vec<RecoveryEntry>,
    index: Arc<Index>,
    link_paths: std::collections::HashMap<PathBuf, Arc<Vec<crate::editor_links::CompletionPath>>>,
    link_paths_key: Option<(Arc<Index>, inkstone_core::locations::LinkFormat, bool)>,
    search: Entity<InputState>,
    search_results: Vec<SearchHit>,
    search_revision: u64,
    search_jobs: search::Jobs,
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
    known_writes: std::collections::BTreeMap<PathBuf, String>,
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
        let mut sync_paths = Vec::new();
        if let Some(receiver) = &self.watch_events {
            for event in receiver.try_iter() {
                match event {
                    Ok(event) if event.need_rescan() => {
                        sync_paths.push(PathBuf::new());
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
                            if let Some(relative) = self
                                .vault
                                .as_ref()
                                .and_then(|v| path.strip_prefix(&v.root).ok())
                            {
                                sync_paths.push(relative.to_path_buf());
                            }
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
                                // A renamed or removed directory is not a content change.
                                // Reconcile the tree from a file listing; do not reread every note.
                                self.structure_changed = true;
                            }
                        }
                    }
                    Err(error) => {
                        self.notifications.publish(format!("文件监听错误：{error}"));
                        cx.notify();
                    }
                    _ => (),
                }
            }
        }
        self.note_sync_watch_paths(sync_paths);
        self.tick_sync_watch(cx);
        if self.refresh_requested && !self.refreshing {
            self.refresh(window, cx);
        }
        // Only continue saves required by an explicit close or navigation request.
        if self.ui.window_close_requested
            || !self.ui.close_pending.is_empty()
            || self.pending_navigation.is_some()
        {
            self.save_pending(window, cx);
        }
        self.tick_drafts(cx);
        if self.ui.search_drafts_changed {
            self.run_search(cx);
        }
        self.finish_pending_navigation(window, cx);
        self.finish_pending_closes(window, cx);
        self.persist_workspace(cx);
        self.pump_auto_sync(window, cx);
        self.tick_cloud_sync(window, cx);
        self.tick_backups(window, cx);
        self.finish_window_close(window, cx);
    }
}
