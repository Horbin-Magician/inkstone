mod daily;
mod extras;
mod file_settings;
mod hotkeys;
mod inline_title;
mod link_updates;
mod templates;
mod ui;
mod views;
use crate::editor::{EditorEvent, EditorPane};
use gpui::{prelude::*, *};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    tree::{TreeItem, TreeState},
};
use inkstone::index::{Index, Resolution, SearchHit};
use inkstone::vault::{RecoveryEntry, Vault, VaultError};
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
    path: PathBuf,
    pane: Entity<EditorPane>,
    baseline: Option<String>,
    dirty: bool,
    saving: bool,
    conflict: bool,
    error: Option<String>,
    recovery_text: String,
    _subscription: Subscription,
    _links: Subscription,
    _focus: Subscription,
}
pub struct Workspace {
    graph: Option<Entity<crate::graph_view::GraphView>>,
    graph_open: bool,
    graph_subscription: Option<Subscription>,
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
    backlinks: Vec<PathBuf>,
    backlink_scroll: UniformListScrollHandle,
    tree: Entity<TreeState>,
    tree_files: Vec<PathBuf>,
    command_open: bool,
    changed_paths: std::collections::BTreeSet<PathBuf>,
    rescan: bool,
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
        #[cfg(feature = "metrics")]
        crate::metrics::sample(
            window,
            self.files.len(),
            self.active
                .and_then(|i| self.tabs.get(i))
                .map_or(0, |tab| tab.pane.read(cx).editor.read(cx).value().len()),
            cx,
        );
        if let Some(receiver) = &self.watch_events {
            for event in receiver.try_iter() {
                match event {
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
                            } else {
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
        self.finish_pending_closes(window, cx);
        self.persist_workspace(cx);
        self.finish_window_close(window, cx);
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.refresh_requested = false;
        self.refreshing = true;
        let generation = self.generation;
        let changed = std::mem::take(&mut self.changed_paths);
        let rescan = std::mem::take(&mut self.rescan) || changed.is_empty();
        let previous = self.index.clone();
        let requests: Vec<_> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.path.clone(), t.baseline.clone()))
            .collect();
        let task = cx.background_executor().spawn(async move {
            let index = if rescan {
                Index::build(&vault)?
            } else {
                let mut index = (*previous).clone();
                index.refresh_paths(&vault, changed)?;
                index
            };
            let files = index.notes.keys().cloned().collect();
            let mut documents = vec![];
            for (id, path, baseline) in requests {
                let disk = vault.read(&path)?;
                documents.push((id, path, baseline, disk));
            }
            let folders = vault.folders()?;
            Ok::<_, VaultError>((files, documents, index, folders))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation { return; }
                this.refreshing = false;
                if this.ui.file_operation {
                    this.refresh_requested = true;
                    this.rescan = true;
                    return;
                }
                match result {
                    Ok((files, documents, index, folders)) => {
                        this.ui.folders = folders;
                        this.files = files;
                        this.index = Arc::new(index);
                        this.sync_index_ui(cx);
                        for (id, path, baseline, disk) in documents {
                            let split_pending=this.has_pending_input(id,window,cx);
                            let Some(tab) = this.tabs.iter_mut().find(|t|t.id == id) else { continue; };
                            if tab.saving || tab.path!=path || tab.baseline != baseline { this.refresh_requested = true; continue; }
                            if disk == tab.baseline { continue; }
                            if tab.dirty || split_pending || disk.is_none() {
                                tab.conflict = true; tab.dirty = true;
                                this.status = format!("{} 在外部发生变化。编辑内容已保留；可用“另存为副本”保存当前版本。", tab.path.display());
                            } else if let Some(text) = disk {
                                let editor = tab.pane.read(cx).editor.clone();
                                let selection = editor.read(cx).selected_range();
                                tab.baseline = Some(text.clone());
                                editor.update(cx, |state, cx| { state.set_value(text, window, cx); state.set_selected_range(selection, cx); });
                                this.status = format!("已重新加载外部修改：{}", tab.path.display());
                            }
                        }
                    }
                    Err(error) => this.status = error.to_string(),
                }
                this.run_search(cx);
                cx.notify();
            });
        }).detach();
    }
    fn restore_draft(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.recoveries.get(index).cloned() else {
            return;
        };
        let name = entry
            .record
            .relative
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = PathBuf::from(format!("{name}-恢复-{unique}.md"));
        self.add_tab(path, None, true, window, cx);
        let editor = self.tabs.last().unwrap().pane.read(cx).editor.clone();
        editor.update(cx, |state, cx| {
            state.set_value(entry.record.draft, window, cx)
        });
        self.status = "恢复内容已打开为新笔记，原文件与恢复记录均保留。".into();
        self.save_all(window, cx);
        cx.notify();
    }
    fn manage_note(&mut self, trash: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active.and_then(|i| self.tabs.get(i)).map(|tab| tab.id) else {
            return;
        };
        let name = self.name.read(cx).value().trim().to_string();
        self.manage_named_note(id, trash, name, window, cx);
    }
    fn manage_named_note(
        &mut self,
        id: usize,
        trash: bool,
        requested_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.pending_file_writes > 0 || self.ui.link_update.is_some() {
            return;
        }
        if trash
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path.as_os_str().is_empty())
        {
            self.close_tab(window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if !trash && self.tabs.iter().any(|t| t.dirty || t.saving || t.conflict) {
            self.status = "请先保存打开的笔记，再重命名并更新内部链接。".into();
            self.save_all(window, cx);
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        if tab.dirty || tab.saving || tab.conflict {
            self.status = "请先保存并处理冲突，再重命名或移入回收区。".into();
            cx.notify();
            return;
        }
        let Some(baseline) = tab.baseline.clone() else {
            return;
        };
        let mut name = requested_name;
        if !trash && name.is_empty() {
            self.status = "请在名称框输入新文件名。".into();
            cx.notify();
            return;
        }
        if !name.to_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        let dest = PathBuf::from(name);
        let path = tab.path.clone();
        let id = tab.id;
        let generation = self.generation;
        tab.saving = true;
        self.ui.file_operation = true;
        self.ui.pending_file_writes += 1;
        let task = cx.background_executor().spawn(async move {
            if trash {
                vault
                    .trash_note(&path, &baseline)
                    .map(|p| (true, p, LinkEdits::new()))
            } else {
                let index = Index::build(&vault)?;
                let edits = index.relocation_edits(&path, &dest, false, Some(&vault.root));
                vault.rename_note(&path, &dest, &baseline)?;
                Ok((false, dest, edits))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation {
                    return;
                }
                this.ui.file_operation = false;
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                let Some(index) = this.tabs.iter().position(|t| t.id == id) else {
                    return;
                };
                this.tabs[index].saving = false;
                match result {
                    Ok((true, path, _)) => {
                        this.status = format!("已移入可恢复回收区：{}", path.display());
                        if this.tabs[index].dirty {
                            this.tabs[index].conflict = true;
                            this.status
                                .push_str("；操作期间的新编辑已保留，可另存副本。");
                        } else {
                            this.tabs.remove(index);
                            this.active = if this.tabs.is_empty() {
                                None
                            } else {
                                Some(index.min(this.tabs.len() - 1))
                            };
                        }
                    }
                    Ok((false, path, edits)) => {
                        let focus_after = this
                            .ui
                            .inline_title
                            .as_ref()
                            .is_some_and(|edit| edit.id == id && edit.focus_after);
                        if this
                            .ui
                            .inline_title
                            .as_ref()
                            .is_some_and(|edit| edit.id == id)
                        {
                            this.ui.inline_title = None;
                        }
                        let old = this.tabs[index].path.clone();
                        for item in this
                            .ui
                            .prefs
                            .bookmarks
                            .iter_mut()
                            .chain(this.ui.prefs.pinned_paths.iter_mut())
                            .chain(this.ui.history.entries.iter_mut())
                        {
                            if *item == old {
                                *item = path.clone();
                            }
                        }
                        this.tabs[index].path = path;
                        this.sync_reference_contexts(cx);
                        if focus_after && let Some(pane) = this.current_pane() {
                            pane.update(cx, |p, cx| p.focus_view(window, cx));
                        }
                        this.status = "已重命名。".into();
                        this.offer_link_updates(edits, window, cx);
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        this.ui.window_close_requested = false;
                    }
                }
                this.refresh_requested = true;
                this.tick(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("新笔记名称.md"));
        let tree = cx.new(|cx| TreeState::new(cx));
        let tree_subscription = cx.subscribe(
            &tree,
            |this, _, event: &gpui_component::tree::TreeEvent, _| {
                use gpui_component::tree::TreeEvent;
                match event {
                    TreeEvent::Expanded(id) => {
                        let path = PathBuf::from(id.as_ref());
                        if !this.ui.prefs.expanded_folders.contains(&path) {
                            this.ui.prefs.expanded_folders.push(path);
                        }
                    }
                    TreeEvent::Collapsed(id) => this
                        .ui
                        .prefs
                        .expanded_folders
                        .retain(|p| p != std::path::Path::new(id.as_ref())),
                }
            },
        );
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名 / 全文"));
        let name_subscription = cx.subscribe_in(&name, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_name(window, cx);
            }
        });
        let search_subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.ui.selected = 0;
                this.ui.modal_scroll.set_offset(Point::default());
                this.run_search(cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.open_selected_result(window, cx);
            }
        });
        let timer = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.request_window_close(window, cx))
                .unwrap_or(true)
        });
        let task = cx.background_executor().spawn(async move {
            if cfg!(test) {
                return None;
            }
            std::env::args_os().nth(1).map(PathBuf::from).or_else(|| {
                std::fs::read_to_string(app_dir().join("recent.txt"))
                    .ok()
                    .map(PathBuf::from)
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Some(root) = task.await {
                let _ = this.update_in(cx, |this, window, cx| this.load_vault(root, window, cx));
            }
        })
        .detach();
        let ui = ui::UiState::new(window, cx);
        Self {
            graph: None,
            graph_open: false,
            graph_subscription: None,
            ui,
            views: Default::default(),
            vault: None,
            files: vec![],
            tabs: vec![],
            active: None,
            next_id: 0,
            generation: 0,
            navigation_generation: 0,
            name,
            status: "Ctrl+O 打开库，Ctrl+S 保存，Ctrl+F 文档查找。".into(),
            loading: false,
            _timer: timer,
            watcher: None,
            watch_events: None,
            refresh_requested: false,
            refreshing: false,
            recoveries: vec![],
            index: Arc::new(Index::default()),
            search,
            search_results: vec![],
            search_revision: 0,
            fulltext: false,
            _search_subscription: search_subscription,
            _name_subscription: name_subscription,
            _tree_subscription: tree_subscription,
            pending_jump: None,
            backlinks: vec![],
            backlink_scroll: UniformListScrollHandle::new(),
            tree,
            tree_files: vec![],
            command_open: false,
            changed_paths: Default::default(),
            rescan: false,
        }
    }
    fn choose_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.pending_file_writes > 0 || self.tabs.iter().any(|t| t.dirty || t.saving) {
            self.status = "请先保存当前笔记，再切换笔记库。".into();
            cx.notify();
            return;
        }
        let dialog = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开 Markdown 笔记库".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = dialog.await
                && let Some(root) = paths.first()
            {
                let root = root.clone();
                let _ = this.update_in(cx, |this, window, cx| this.load_vault(root, window, cx));
            }
        })
        .detach();
    }
    fn load_vault(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.pending_file_writes > 0 || self.tabs.iter().any(|t| t.dirty || t.saving) {
            self.status = "当前仍有未保存内容，已取消切换笔记库。".into();
            cx.notify();
            return;
        }
        self.loading = true;
        self.generation += 1;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let vault = Vault::open(root, app_dir().join("recovery"))?;
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut watcher = notify::recommended_watcher(move |event| {
                let _ = sender.send(event);
            })
            .map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
            watcher
                .watch(&vault.root, RecursiveMode::Recursive)
                .map_err(|e| VaultError::Io(std::io::Error::other(e)))?;
            let index = Index::build(&vault)?;
            let files = index.notes.keys().cloned().collect();
            let recoveries = vault.recoveries()?;
            let _ = std::fs::write(
                app_dir().join("recent.txt"),
                vault.root.to_string_lossy().as_bytes(),
            );
            let folders = vault.folders()?;
            let prefs = inkstone::preferences::Preferences::load(
                &vault.root.join(".inkstone-workspace.json"),
            );
            let restored: Vec<_> = prefs
                .open_paths
                .iter()
                .enumerate()
                .filter_map(|(saved_index, path)| {
                    if path.as_os_str().is_empty() {
                        return Some((saved_index, path.clone(), String::new()));
                    }
                    vault
                        .read(path)
                        .ok()
                        .flatten()
                        .map(|text| (saved_index, path.clone(), text))
                })
                .collect();
            Ok::<_, VaultError>((
                vault, files, watcher, receiver, recoveries, index, folders, prefs, restored,
            ))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                if this.ui.pending_file_writes > 0 || this.tabs.iter().any(|t| t.dirty || t.saving)
                {
                    this.status = "读取期间产生了新编辑，已保留当前笔记库。".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((
                        vault,
                        files,
                        watcher,
                        receiver,
                        recoveries,
                        index,
                        folders,
                        prefs,
                        restored,
                    )) => {
                        this.ui.folders = folders;
                        let restore_active = prefs.active_path.clone();
                        let restore_active_index = prefs.active_tab_index;
                        this.ui.prefs = prefs;
                        this.ui.link_update = None;
                        this.graph_open = false;
                        this.graph = None;
                        this.graph_subscription = None;
                        this.ui.last_persisted.clear();
                        ui::apply_theme(this.ui.prefs.light, cx);
                        this.ui.history = Default::default();
                        this.ui.closed.clear();
                        this.ui.inline_title = None;
                        this.ui.close_pending.clear();
                        this.ui.window_close_requested = false;
                        this.vault = Some(vault);
                        this.watcher = Some(watcher);
                        this.watch_events = Some(receiver);
                        this.recoveries = recoveries;
                        this.index = Arc::new(index);
                        this.sync_index_ui(cx);
                        this.run_search(cx);
                        this.refreshing = false;
                        this.refresh_requested = false;
                        this.changed_paths.clear();
                        this.rescan = false;
                        this.files = files;
                        this.views = Default::default();
                        this.tabs.clear();
                        this.active = None;
                        this.status =
                            "笔记库已打开。每 2 秒自动保存，恢复副本在应用恢复区。".into();
                        this.loading = true;
                        let mut restored_active = None;
                        for (saved_index, path, text) in restored {
                            this.add_tab(path, Some(text), false, window, cx);
                            if Some(saved_index) == restore_active_index {
                                restored_active = this.active;
                            }
                        }
                        this.loading = false;
                        if let Some(i) = restored_active.or_else(|| {
                            restore_active.and_then(|p| this.tabs.iter().position(|t| t.path == p))
                        }) {
                            this.activate_tab(i, window, cx);
                        } else if this.tabs.is_empty()
                            && let Some(path) = this.files.first().cloned()
                        {
                            this.open_note(path, window, cx);
                        }
                        this.restore_split(window, cx);
                    }
                    Err(error) => this.status = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn open_note(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_with_view(path, None, window, cx);
    }
    fn apply_reopened_view(
        &mut self,
        state: Option<&inkstone::preferences::ViewState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = state
            && let Some(pane) = self.current_pane()
        {
            Self::restore_view_state(&pane, state, cx);
            pane.update(cx, |p, cx| p.focus_view(window, cx));
            self.persist_workspace(cx);
        }
    }
    fn open_note_with_view(
        &mut self,
        path: PathBuf,
        view: Option<inkstone::preferences::ViewState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.graph_open = false;
        if self
            .ui
            .pending_command
            .as_ref()
            .is_some_and(|(p, _)| p != &path)
        {
            self.ui.pending_command = None;
        }
        if self
            .pending_jump
            .as_ref()
            .is_some_and(|(target, _)| target != &path)
        {
            self.pending_jump = None;
        }
        self.navigation_generation += 1;
        let navigation_generation = self.navigation_generation;
        if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
            self.activate_tab(i, window, cx);
            self.apply_reopened_view(view.as_ref(), window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        let requested = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { vault.read(&requested) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation
                    || this.navigation_generation != navigation_generation
                {
                    return;
                }
                if let Some(i) = this.tabs.iter().position(|t| t.path == path) {
                    this.activate_tab(i, window, cx);
                    this.apply_reopened_view(view.as_ref(), window, cx);
                    return;
                }
                match result {
                    Ok(Some(text)) => {
                        this.add_tab(path, Some(text), false, window, cx);
                        this.apply_reopened_view(view.as_ref(), window, cx);
                    }
                    Ok(None) => {
                        this.status = "文件已不存在，请刷新目录。".into();
                        cx.notify();
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn add_tab(
        &mut self,
        path: PathBuf,
        baseline: Option<String>,
        new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.inline_title.is_some() {
            self.commit_inline_title(false, window, cx);
        }
        self.graph_open = false;
        if !self.loading
            && !path.as_os_str().is_empty()
            && !self.views.secondary_focused
            && let Some(i) = self.active.filter(|i| {
                self.tabs
                    .get(*i)
                    .is_some_and(|t| t.path.as_os_str().is_empty())
            })
        {
            self.tabs.remove(i);
            self.active = None;
            self.views.main = None;
        }
        let target_split = self.views.secondary_focused && self.views.split.is_some();
        let pane = cx.new(|cx| EditorPane::new(baseline.as_deref().unwrap_or(""), window, cx));
        pane.update(cx, |pane, _| pane.set_paths(self.link_paths_for(&path)));
        if let Some(vault) = &self.vault {
            pane.update(cx, |pane, _| {
                pane.image_dir = vault
                    .root
                    .join(path.parent().unwrap_or(std::path::Path::new("")))
            });
        }
        let editor = pane.read(cx).editor.clone();
        let id = self.next_id;
        self.next_id += 1;
        let links = cx.subscribe_in(&pane, window, move |this, _, event, window, cx| {
            if let Some(tab) = this.tabs.iter().find(|t| t.id == id) {
                let from = tab.path.clone();
                match event {
                    EditorEvent::FollowLink(target) => {
                        this.follow_link(from, target.clone(), window, cx)
                    }
                    EditorEvent::FollowMarkdownLink(target) => {
                        this.follow_markdown_link(from, target, window, cx)
                    }
                    EditorEvent::FollowReference(reference) => {
                        this.follow_reference(reference.clone(), window, cx)
                    }
                    EditorEvent::ToggleTask(target, checked) => {
                        this.toggle_referenced_task(target.clone(), *checked, window, cx)
                    }
                    EditorEvent::PasteFiles(paths) => this.import_files(paths.clone(), window, cx),
                    EditorEvent::PasteImage(name, bytes) => {
                        this.paste_image(name.clone(), bytes.clone(), window, cx)
                    }
                }
            }
        });
        let subscription =
            cx.subscribe_in(&editor, window, move |this, editor, event, window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id)
                {
                    tab.dirty = tab.baseline.as_deref() != Some(editor.read(cx).value().as_ref());
                    this.sync_to_split(id, window, cx);
                    cx.notify();
                }
            });
        let focus = cx.observe_in(&editor, window, move |this, editor, w, cx| {
            if editor.read(cx).focus_handle(cx).is_focused(w)
                && this.views.split.is_some()
                && (this.views.secondary_focused || this.views.main != Some(id))
            {
                this.views.secondary_focused = false;
                this.views.main = Some(id);
                this.active = this.tabs.iter().position(|t| t.id == id);
                this.run_search(cx);
                cx.notify();
            }
        });
        self.tabs.push(Tab {
            id,
            path,
            pane,
            baseline,
            dirty: new,
            saving: false,
            conflict: false,
            error: None,
            recovery_text: String::new(),
            _subscription: subscription,
            _links: links,
            _focus: focus,
        });
        self.sync_reference_contexts(cx);
        self.active = Some(self.tabs.len() - 1);
        if target_split {
            self.bind_split(self.tabs.len() - 1, window, cx);
        } else {
            self.views.main = Some(id);
        }
        self.ui.quick_open = false;
        self.ui.name_mode = None;
        if !self.tabs.last().unwrap().path.as_os_str().is_empty() {
            self.ui
                .history
                .visit(self.tabs.last().unwrap().path.clone());
        }
        self.apply_editor_preferences(window, cx);
        if let Some((path, command)) = self.ui.pending_command.take()
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path == path)
        {
            self.execute_command(command, window, cx);
        }
        self.apply_jump(window, cx);
        self.run_search(cx);
        cx.notify();
    }
    fn create_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.as_ref() else {
            self.status = "请先打开笔记库。".into();
            cx.notify();
            return;
        };
        let mut name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            name = "未命名.md".into();
        }
        if !name.to_lowercase().ends_with(".md") {
            name.push_str(".md");
        }
        let path = PathBuf::from(name);
        if let Err(error) = vault.path(&path) {
            self.status = error.to_string();
            cx.notify();
            return;
        }
        if self.files.contains(&path) || self.tabs.iter().any(|t| t.path == path) {
            self.status = "同名笔记已存在。".into();
            cx.notify();
            return;
        }
        self.add_tab(path, None, true, window, cx);
        self.save_all(window, cx);
    }
    fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        for tab in &mut self.tabs {
            if !tab.conflict {
                tab.error = None;
            }
        }
        self.save_pending(window, cx);
    }
    fn save_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.file_operation {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        for tab in &mut self.tabs {
            let editor = tab.pane.read(cx).editor.clone();
            if editor.update(cx, |state, cx| {
                state.marked_text_range(window, cx).is_some()
            }) {
                continue;
            }
            if tab.dirty && (tab.conflict || tab.error.is_some()) {
                let text = tab.pane.read(cx).editor.read(cx).value().to_string();
                if text != tab.recovery_text {
                    tab.recovery_text = text.clone();
                    let vault = vault.clone();
                    let path = tab.path.clone();
                    let baseline = tab.baseline.clone();
                    let id = tab.id;
                    let task = cx
                        .background_executor()
                        .spawn(async move { vault.journal(&path, baseline.as_deref(), &text) });
                    cx.spawn(async move |this, cx| {
                        if let Err(error) = task.await {
                            let _ = this.update(cx, |this, cx| {
                                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id) {
                                    tab.recovery_text.clear();
                                }
                                this.status = format!("恢复副本写入失败：{error}");
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
            }
            if !tab.dirty || tab.saving || tab.conflict || tab.error.is_some() {
                continue;
            }
            tab.saving = true;
            let (id, generation) = (tab.id, self.generation);
            let path = tab.path.clone();
            let baseline = tab.baseline.clone();
            let text = tab.pane.read(cx).editor.read(cx).value().to_string();
            let vault = vault.clone();
            let task = cx.background_executor().spawn(async move {
                if baseline.is_none() {
                    vault.create(&path, &text)
                } else {
                    vault.save(&path, baseline.as_deref(), &text)
                }
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.generation != generation {
                        return;
                    }
                    let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id) else {
                        return;
                    };
                    tab.saving = false;
                    match result {
                        Ok(receipt) => {
                            tab.error = None;
                            tab.baseline = Some(receipt.text);
                            tab.dirty = tab.baseline.as_deref()
                                != Some(tab.pane.read(cx).editor.read(cx).value().as_ref());
                            if !this.files.contains(&tab.path) {
                                this.files.push(tab.path.clone());
                                this.files.sort();
                            }
                            this.status = format!("已保存 {}", tab.path.display());
                        }
                        Err(error) => {
                            tab.conflict = matches!(
                                error,
                                VaultError::Conflict { .. } | VaultError::RaceConflict { .. }
                            );
                            this.status = error.to_string();
                            tab.error = Some(this.status.clone());
                        }
                    }
                    this.finish_pending_closes(window, cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }
}
impl Workspace {
    fn focus_search(&mut self, fulltext: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.fulltext = fulltext;
        if fulltext {
            self.ui.left_mode = 1;
            self.ui.prefs.left_open = true;
        }
        self.ui.quick_open = !fulltext;
        self.ui.template_mode = false;
        self.ui.selected = 0;
        self.command_open = false;
        self.ui.modal_scroll.set_offset(Point::default());
        self.search.update(cx, |state, cx| state.focus(window, cx));
        self.run_search(cx);
        cx.notify();
    }
    pub(super) fn ensure_active_note(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .active
            .and_then(|i| self.tabs.get(i))
            .is_none_or(|t| t.path.as_os_str().is_empty())
        {
            self.focus_new(window, cx);
        }
        self.active
            .and_then(|i| self.tabs.get(i))
            .is_some_and(|t| !t.path.as_os_str().is_empty())
    }
    fn new_blank(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(PathBuf::new(), Some(String::new()), false, window, cx);
        window.focus(&self.ui.workspace_focus, cx);
    }
    fn focus_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() {
            self.choose_vault(window, cx);
            return;
        }
        self.command_open = false;
        let current = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.as_path());
        let folder = match self.ui.prefs.locations.directory(current, false) {
            Ok(folder) => folder,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        let mut suffix = 0;
        let path = loop {
            let path = if suffix == 0 {
                "未命名.md".to_string()
            } else {
                format!("未命名 {suffix}.md")
            };
            let path = folder.join(path).to_string_lossy().to_string();
            if !self.files.contains(&PathBuf::from(&path))
                && !self
                    .tabs
                    .iter()
                    .any(|t| t.path == std::path::Path::new(&path))
            {
                break path;
            }
            suffix += 1;
        };
        self.name.update(cx, |s, cx| s.set_value(path, window, cx));
        self.create_note(window, cx);
    }
    fn activate_tab(&mut self, mut index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.graph_open = false;
        let id = self.tabs[index].id;
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| edit.id != id)
        {
            self.commit_inline_title(false, window, cx);
        }
        if !self.views.secondary_focused
            && !self.tabs[index].path.as_os_str().is_empty()
            && let Some(i) = self.active.filter(|i| {
                self.tabs
                    .get(*i)
                    .is_some_and(|t| t.path.as_os_str().is_empty())
            })
        {
            self.tabs.remove(i);
            index = self.tabs.iter().position(|t| t.id == id).unwrap();
        }
        self.active = Some(index);
        self.ui.quick_open = false;
        if !self.tabs[index].path.as_os_str().is_empty() {
            self.ui.history.visit(self.tabs[index].path.clone());
        }
        if self.views.secondary_focused && self.views.split.is_some() {
            if self
                .views
                .split
                .as_ref()
                .is_some_and(|s| s.source == self.tabs[index].id)
            {
                self.views
                    .split
                    .as_ref()
                    .unwrap()
                    .pane
                    .clone()
                    .update(cx, |p, cx| p.focus_view(window, cx));
            } else {
                self.bind_split(index, window, cx);
            }
        } else {
            self.views.main = Some(self.tabs[index].id);
            self.tabs[index]
                .pane
                .update(cx, |p, cx| p.focus_view(window, cx));
        }
        self.run_search(cx);
        if let Some((path, command)) = self.ui.pending_command.take()
            && self
                .active
                .and_then(|i| self.tabs.get(i))
                .is_some_and(|t| t.path == path)
        {
            self.execute_command(command, window, cx);
        }
        cx.notify();
    }
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.graph_open {
            self.graph_open = false;
            if let Some(p) = self.current_pane() {
                p.update(cx, |p, cx| p.focus_view(window, cx));
            }
            cx.notify();
            return;
        }
        if self.views.secondary_focused && self.views.split.is_some() {
            self.close_split(window, cx);
            return;
        }
        if let Some(index) = self.active {
            self.close_tab_at(index, window, cx);
        }
    }
    fn close_tab_group(
        &mut self,
        anchor: Option<usize>,
        mode: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor_index = anchor.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if mode != 2 && anchor_index.is_none() {
            return;
        }
        let ids: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                !self.ui.prefs.pinned_paths.contains(&t.path)
                    && match mode {
                        0 => Some(t.id) != anchor,
                        1 => Some(*i) > anchor_index,
                        _ => true,
                    }
            })
            .map(|(_, t)| t.id)
            .collect();
        if mode == 2 {
            self.graph_open = false;
        }
        for id in ids.into_iter().rev() {
            if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                self.close_tab_at(i, window, cx);
            }
        }
        if mode == 0
            && let Some(index) = anchor.and_then(|id| self.tabs.iter().position(|t| t.id == id))
        {
            self.focus_primary(index, window, cx);
        }
        self.persist_workspace(cx);
        cx.notify();
    }
    fn close_tab_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| self.tabs.get(index).is_some_and(|tab| tab.id == edit.id))
        {
            self.commit_inline_title(false, window, cx);
            if self.ui.inline_title.is_some() && !self.ui.file_operation {
                self.status = "请完成或取消标题修改后关闭标签。".into();
                cx.notify();
                return;
            }
        }
        if let Some(id) = self.tabs.get(index).map(|t| t.id)
            && self.has_pending_input(id, window, cx)
        {
            self.status = "请完成输入法组词后关闭标签。".into();
            cx.notify();
            return;
        }
        self.sync_from_split(window, cx);
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if self.ui.prefs.pinned_paths.contains(&tab.path) {
            self.status = "请先取消固定，再关闭标签页。".into();
            cx.notify();
            return;
        }
        tab.dirty =
            tab.baseline.as_deref() != Some(tab.pane.read(cx).editor.read(cx).value().as_ref());
        if tab.conflict || tab.error.is_some() {
            self.status = "请先处理保存错误或另存副本，再关闭标签。".into();
            cx.notify();
            return;
        }
        if tab.dirty || tab.saving {
            self.ui.close_pending.insert(tab.id);
            self.status = "正在保存，成功后关闭标签…".into();
            self.save_all(window, cx);
            cx.notify();
            return;
        }
        self.remove_saved_tab(index, window, cx);
    }
    fn remove_saved_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let main_before = self.views.main;
        let active_id = self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id);
        let removed = self.tabs.remove(index);
        if self
            .ui
            .inline_title
            .as_ref()
            .is_some_and(|edit| edit.id == removed.id)
        {
            self.ui.inline_title = None;
        }
        let closed_pane = self
            .views
            .split
            .as_ref()
            .filter(|s| s.source == removed.id && self.views.secondary_focused)
            .map(|s| &s.pane)
            .unwrap_or(&removed.pane);
        let closed_view = Self::snapshot_view(removed.path.clone(), closed_pane, cx);
        if self
            .views
            .split
            .as_ref()
            .is_some_and(|s| s.source == removed.id)
        {
            self.views.split = None;
            self.views.secondary_focused = false;
        }
        self.ui.close_pending.remove(&removed.id);
        if !removed.path.as_os_str().is_empty() {
            self.ui.closed.push(ui::ClosedTab { view: closed_view });
        }
        self.active = active_id
            .and_then(|id| self.tabs.iter().position(|t| t.id == id))
            .or_else(|| {
                (!self.tabs.is_empty()).then_some(index.min(self.tabs.len().saturating_sub(1)))
            });
        self.views.main = main_before
            .filter(|id| self.tabs.iter().any(|t| t.id == *id))
            .or_else(|| self.active.and_then(|i| self.tabs.get(i)).map(|t| t.id));
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.focus_view(window, cx));
        } else {
            window.focus(&self.ui.workspace_focus, cx);
        }
        self.run_search(cx);
        self.persist_workspace(cx);
        cx.notify();
    }
    fn finish_pending_closes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            if self.ui.prefs.pinned_paths.contains(&tab.path) {
                self.ui.close_pending.remove(&tab.id);
            }
        }
        let ready: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                self.ui.close_pending.contains(&t.id)
                    && !t.dirty
                    && !t.saving
                    && !t.conflict
                    && t.error.is_none()
                    && !self.ui.prefs.pinned_paths.contains(&t.path)
                    && !self.has_pending_input(t.id, window, cx)
            })
            .map(|(i, _)| i)
            .collect();
        for i in ready.into_iter().rev() {
            self.remove_saved_tab(i, window, cx);
        }
    }
    fn sync_index_ui(&mut self, cx: &mut Context<Self>) {
        if self.graph_open
            && let Some(graph) = &self.graph
        {
            graph.update(cx, |g, cx| {
                g.set_index(self.index.clone(), self.ui.prefs.light, cx)
            });
        }
        for tab in &self.tabs {
            tab.pane.update(cx, |pane, _| {
                pane.set_paths(self.link_paths_for(&tab.path));
                if let Some(vault) = &self.vault {
                    pane.image_dir = vault
                        .root
                        .join(tab.path.parent().unwrap_or(std::path::Path::new("")));
                }
            });
        }
        if let Some(split) = &self.views.split {
            if let Some(tab) = self.tabs.iter().find(|t| t.id == split.source) {
                let image_dir = tab.pane.read(cx).image_dir.clone();
                let paths = self.link_paths_for(&tab.path);
                split.pane.update(cx, |p, _| {
                    p.image_dir = image_dir;
                    p.set_paths(paths);
                });
            } else {
                self.views.split = None;
                self.views.secondary_focused = false;
            }
        }
        let files: Vec<_> = self.index.notes.keys().cloned().collect();
        if self.tree_files != files
            || self.ui.tree_folders != self.ui.folders
            || self.ui.prefs.sort_by != inkstone::file_order::SortBy::Name
        {
            self.ui.tree_folders = self.ui.folders.clone();
            self.tree_files = files.clone();
            self.rebuild_sorted_tree(cx);
        }
        self.sync_reference_contexts(cx);
    }
    fn run_search(&mut self, cx: &mut Context<Self>) {
        self.search_revision += 1;
        let revision = self.search_revision;
        let generation = self.generation;
        let query = self.search.read(cx).value().to_string();
        let index = self.index.clone();
        let fulltext = self.fulltext;
        let template_folder = self
            .ui
            .template_mode
            .then(|| self.ui.prefs.templates.directory().ok());
        if fulltext && let Err(error) = inkstone::search::Query::parse(&query) {
            self.search_results.clear();
            self.status = error;
            cx.notify();
            return;
        }
        let active = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        let task = cx.background_executor().spawn(async move {
            let backlinks = active.map(|p| index.backlinks(&p)).unwrap_or_default();
            let hits = if let Some(folder) = template_folder {
                folder
                    .map(|folder| inkstone::templates::search(&index, &folder, &query))
                    .unwrap_or_default()
            } else if query.trim().is_empty() {
                vec![]
            } else if fulltext {
                index.search(&query)
            } else {
                index.filenames(&query)
            };
            (hits, backlinks)
        });
        cx.spawn(async move |this, cx| {
            let (hits, backlinks) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.search_revision == revision && this.generation == generation {
                    this.search_results = hits;
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
    fn apply_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, offset)) = self.pending_jump.clone() else {
            return;
        };
        let Some(tab) = self
            .active
            .and_then(|i| self.tabs.get(i))
            .filter(|t| t.path == path)
        else {
            return;
        };
        let _ = tab;
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| p.jump(offset, window, cx));
        }
        self.pending_jump = None;
    }
    fn follow_markdown_link(
        &mut self,
        from: PathBuf,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if href.starts_with("https://")
            || href.starts_with("http://")
            || href.starts_with("mailto:")
        {
            cx.open_url(href);
            return;
        }
        if let Some(vault) = &self.vault {
            let raw = href.split('#').next().unwrap_or("");
            if let Ok(decoded) = percent_encoding::percent_decode_str(raw).decode_utf8()
                && !decoded.contains(':')
                && !decoded.is_empty()
                && std::path::Path::new(decoded.as_ref())
                    .extension()
                    .is_some_and(|e| !e.eq_ignore_ascii_case("md"))
            {
                let reference = inkstone::rendering::Reference {
                    from: from.clone(),
                    target: href.into(),
                    wiki: false,
                };
                match inkstone::rendering::asset_path(&vault.root, &reference, &self.index.files) {
                    Some(path) => {
                        cx.open_with_system(&path);
                        return;
                    }
                    _ => {
                        self.status = "附件不存在或超出笔记库。".into();
                        cx.notify();
                        return;
                    }
                }
            }
        }
        let (resolution, heading) = self.index.resolve_markdown(&from, href);
        match resolution {
            Resolution::Found(path) => {
                if let Some(range) =
                    heading.and_then(|heading| self.index.anchor_range(&path, &heading))
                {
                    self.pending_jump = Some((path.clone(), range.start));
                }
                self.open_note(path, window, cx);
                self.apply_jump(window, cx);
            }
            Resolution::Missing(path) => {
                self.status = format!(
                    "本地 Markdown 链接目标不存在：{}。如需创建笔记，请使用双链。",
                    path.display()
                )
            }
            _ => self.status = "此本地链接不是有效的库内 Markdown 目标。".into(),
        }
        cx.notify();
    }
    fn follow_link(
        &mut self,
        from: PathBuf,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.index.resolve(&from, &target) {
            Resolution::Found(path) => {
                if let Some((_, heading)) = target.split_once('#')
                    && let Some(range) = self.index.anchor_range(&path, heading)
                {
                    self.pending_jump = Some((path.clone(), range.start));
                }
                self.open_note(path, window, cx);
                self.apply_jump(window, cx);
            }
            Resolution::Missing(path) => {
                if let Some(i) = self.tabs.iter().position(|t| t.path == path) {
                    self.activate_tab(i, window, cx);
                } else {
                    self.add_tab(path, None, true, window, cx);
                    self.save_all(window, cx);
                }
            }
            Resolution::Ambiguous(paths) => {
                self.status = format!(
                    "同名笔记有 {} 个，请使用目录/笔记名，或在搜索结果中选择。",
                    paths.len()
                );
                self.search.update(cx, |state, cx| {
                    state.set_value(target.split('#').next().unwrap_or(""), window, cx)
                });
                self.fulltext = false;
                self.run_search(cx);
            }
            Resolution::Invalid => self.status = "双链路径无效或超出笔记库。".into(),
        }
        cx.notify();
    }
    fn save_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return;
        };
        if tab.saving {
            return;
        }
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
        tab.path
            .set_file_name(format!("{stem}-副本-{timestamp}.md"));
        tab.baseline = None;
        tab.conflict = false;
        tab.error = None;
        tab.dirty = true;
        self.sync_reference_contexts(cx);
        self.save_all(window, cx);
        cx.notify();
    }
}

fn make_tree(files: &[PathBuf]) -> Vec<TreeItem> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut children: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for file in files {
        let mut path = PathBuf::new();
        for component in file.components() {
            let parent = path.clone();
            path.push(component);
            children.entry(parent).or_default().insert(path.clone());
        }
    }
    fn build(
        parent: &std::path::Path,
        children: &BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    ) -> Vec<TreeItem> {
        children
            .get(parent)
            .into_iter()
            .flat_map(|items| items.iter())
            .map(|path| {
                TreeItem::new(
                    path.to_string_lossy().to_string(),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                )
                .children(build(path, children))
            })
            .collect()
    }
    build(std::path::Path::new(""), &children)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn template_properties_and_body_are_one_undoable_edit(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let source = "---\ntags: [old]\n---\n中文😀结束";
                w.add_tab("note.md".into(), Some(source.into()), false, window, cx);
                Arc::make_mut(&mut w.index)
                    .update("template.md".into(), "---\ntags: [new]\n---\n插入".into());
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                let at = source.find("😀").unwrap();
                editor.update(cx, |s, cx| s.set_selected_range(at..at + 4, cx));
                w.insert_template(std::path::Path::new("template.md"), window, cx);
                let merged = editor.read(cx).value();
                assert!(merged.ends_with("中文插入结束"));
                assert!(merged.contains("old") && merged.contains("new"));
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), source);
                assert_eq!(editor.read(cx).selected_range(), at..at + 4);
                Arc::make_mut(&mut w.index)
                    .update("bad.md".into(), "---\ntags: [\n---\n破损".into());
                w.insert_template(std::path::Path::new("bad.md"), window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), source);
                assert!(w.status.contains("YAML"));
            })
            .unwrap();
    }
    #[gpui::test]
    fn daily_navigation_opens_existing_neighbors_without_creating_notes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-daily-navigation-{}", std::process::id()));
        std::fs::create_dir_all(root.join("日记")).unwrap();
        for name in ["2026-08-31", "2026-09-02", "2026-09-04"] {
            std::fs::write(root.join(format!("日记/{name}.md")), name).unwrap();
        }
        handle
            .update(cx, |w, window, cx| {
                let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.vault = Some(vault);
                w.ui.prefs.daily.folder = "日记".into();
                w.add_tab(
                    "日记/2026-09-02.md".into(),
                    Some("2026-09-02".into()),
                    false,
                    window,
                    cx,
                );
                w.execute_command(77, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("日记/2026-08-31.md")
                );
                w.execute_command(77, window, cx);
                assert_eq!(w.status, "没有上一篇日记。");
                w.execute_command(78, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("日记/2026-09-02.md")
                );
                w.execute_command(78, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("日记/2026-09-04.md")
                );
            })
            .unwrap();
        assert_eq!(std::fs::read_dir(root.join("日记")).unwrap().count(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn date_time_commands_insert_at_selection_head_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.execute_command(75, window, cx);
                assert!(w.tabs.is_empty());
                w.add_tab("note.md".into(), Some("中文😀".into()), false, window, cx);
                w.ui.prefs.templates.date_format = "[日期]".into();
                w.ui.prefs.templates.time_format = "[时间]".into();
                let pane = w.current_pane().unwrap();
                let editor = pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(3..10, cx));
                w.execute_command(75, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀日期");
                assert_eq!(editor.read(cx).selected_range(), 16..16);
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
                assert_eq!(editor.read(cx).selected_range(), 3..10);
                editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
                w.execute_command(76, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中时间文😀");
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                w.ui.prefs.templates.date_format = "[未闭合".into();
                w.execute_command(75, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
                assert!(w.status.contains("未闭合"));
                pane.update(cx, |p, _| p.reading = true);
                w.execute_command(76, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
            })
            .unwrap();
    }
    #[gpui::test]
    fn template_picker_filters_empty_query_and_inserts_selected_template(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-templates-{}", std::process::id()));
        std::fs::create_dir_all(root.join("模板/子目录")).unwrap();
        std::fs::write(root.join("a.md"), "").unwrap();
        std::fs::write(
            root.join("模板/子目录/会议.md"),
            "# {{title}} {{date:YYYY}} {{time}}",
        )
        .unwrap();
        handle
            .update(cx, |w, window, cx| {
                let vault = Vault::open(&root, root.with_extension("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.files = w.index.notes.keys().cloned().collect();
                w.vault = Some(vault);
                w.add_tab("a.md".into(), Some(String::new()), false, window, cx);
                w.execute_command(29, window, cx);
                assert!(!w.ui.quick_open);
                assert!(w.status.contains("指定模板文件夹"));
                w.ui.prefs.templates.folder = "模板".into();
                w.ui.prefs.templates.time_format = "[测试时间]".into();
                w.execute_command(29, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.search_results.len(), 1);
                assert_eq!(
                    w.search_results[0].display_name.as_deref(),
                    Some("子目录/会议")
                );
                w.open_selected_result(window, cx);
                assert!(!w.ui.template_mode);
                let text = w.current_pane().unwrap().read(cx).editor.read(cx).value();
                assert!(text.starts_with("# a "));
                assert!(text.ends_with(" 测试时间"));
                w.ui.prefs.templates.folder = "缺失文件夹".into();
                w.execute_command(29, window, cx);
                assert!(!w.ui.quick_open);
                assert!(w.status.contains("不存在"));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("模板/子目录/会议.md")).unwrap(),
            "# {{title}} {{date:YYYY}} {{time}}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn daily_creation_uses_template_and_never_rewrites_existing_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-daily-{}", std::process::id()));
        std::fs::create_dir_all(root.join("日记")).unwrap();
        std::fs::create_dir_all(root.join("模板")).unwrap();
        std::fs::write(
            root.join("模板/日记.md"),
            "# {{title}}\n{{date:YYYY年MM月DD日}}\n",
        )
        .unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, root.with_extension("recovery")).unwrap());
                w.ui.prefs.daily.folder = "日记".into();
                w.ui.prefs.daily.format = "[验收]/[今天]".into();
                w.ui.prefs.daily.template = "模板/日记".into();
                w.open_daily(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let path = root.join("日记/验收/今天.md");
        let original = std::fs::read_to_string(&path).unwrap();
        assert!(original.starts_with("# 今天\n"));
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    std::path::Path::new("日记/验收/今天.md")
                );
                w.ui.prefs.daily.template = "模板/不存在".into();
                w.open_daily(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.daily.format = "[缺模板]".into();
                w.open_daily(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!root.join("日记/缺模板.md").exists());
        handle
            .update(cx, |w, window, cx| {
                assert!(w.status.contains("找不到日记模板"));
                w.ui.prefs.daily.template.clear();
                w.ui.prefs.daily.format = "[空白]".into();
                w.open_daily(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("日记/空白.md")).unwrap(),
            ""
        );
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.daily.folder = "尚未创建".into();
                w.open_daily(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!root.join("尚未创建").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn reading_quote_link_opens_its_source_relative_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-reading-link-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        let source = "> [前往](../other.md)\n";
        std::fs::write(root.join("folder/root.md"), source).unwrap();
        std::fs::write(root.join("other.md"), "# 目的地").unwrap();
        handle
            .update(cx, |w, window, cx| {
                let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.vault = Some(vault);
                w.add_tab(
                    "folder/root.md".into(),
                    Some(source.into()),
                    false,
                    window,
                    cx,
                );
                w.tabs[0].pane.update(cx, |p, cx| {
                    p.reading = true;
                    p.focus_view(window, cx);
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let bounds = handle
            .update(&mut visual, |w, _, cx| {
                w.tabs[0].pane.read(cx).reading_bounds(cx)
            })
            .unwrap();
        visual.simulate_click(
            bounds.origin + point(px(28.), px(10.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("other.md"),
                    "bounds={bounds:?}; status={}",
                    w.status
                )
            })
            .unwrap();
        visual.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn file_location_settings_control_note_and_attachment_writes(cx: &mut TestAppContext) {
        use inkstone::locations::Location;
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-file-locations-{}", std::process::id()));
        std::fs::create_dir_all(root.join("project")).unwrap();
        std::fs::write(root.join("project/start.md"), "").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab(
                    "project/start.md".into(),
                    Some("".into()),
                    false,
                    window,
                    cx,
                );
                w.ui.prefs.locations.notes = Location::Current;
                w.focus_new(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("project/未命名.md").is_file());
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.locations.attachments = Location::Subfolder;
                w.ui.prefs.locations.attachment_folder = "media".into();
                w.graph_open = true;
                w.paste_image("skip.png".into(), b"skip".to_vec(), window, cx);
                assert_eq!(w.ui.pending_file_writes, 0);
                w.graph_open = false;
                w.paste_image("image.png".into(), b"fixture".to_vec(), window, cx);
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| {
                        s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
                    });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.ui.pending_file_writes, 1);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                assert_eq!(editor.read(cx).value(), "ni");
                editor.update(cx, |s, cx| s.replace_text_in_range(None, "你", window, cx));
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
        assert!(!root.join("project/media/skip.png").exists());
        handle
            .update(cx, |w, window, cx| {
                let text = w.tabs[w.active.unwrap()]
                    .pane
                    .read(cx)
                    .editor
                    .read(cx)
                    .value();
                assert!(text.contains("![[image.png]]"));
                w.save_all(window, cx);
                w.ui.prefs.locations.notes = Location::Folder;
                w.ui.prefs.locations.note_folder = "inbox".into();
                w.focus_new(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("inbox/未命名.md").is_file());
        assert_eq!(
            std::fs::read(root.join("project/media/image.png")).unwrap(),
            b"fixture"
        );
        handle
            .update(cx, |w, window, cx| {
                let count = w.tabs.len();
                w.ui.prefs.locations.note_folder = "../outside".into();
                w.focus_new(window, cx);
                assert_eq!(w.tabs.len(), count);
                assert!(w.status.contains("库内文件夹"));
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn relation_graph_opens_notes_and_creates_missing_targets(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-graph-navigation-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "[[b]] [[missing]]").unwrap();
        std::fs::write(root.join("b.md"), "# B").unwrap();
        handle
            .update(cx, |w, window, cx| {
                let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.vault = Some(vault);
                w.add_tab(
                    "a.md".into(),
                    Some("[[b]] [[missing]]".into()),
                    false,
                    window,
                    cx,
                );
                w.open_graph(false, window, cx);
                assert!(w.graph_open);
                w.close_tab(window, cx);
                assert!(!w.graph_open);
                assert_eq!(w.tabs.len(), 1);
                w.open_graph(false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                w.graph.as_ref().unwrap().update(cx, |_, cx| {
                    cx.emit(crate::graph_view::GraphEvent::Open("b.md".into(), false))
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(!w.graph_open);
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("b.md"));
                w.open_graph(true, window, cx);
                w.graph.as_ref().unwrap().update(cx, |_, cx| {
                    cx.emit(crate::graph_view::GraphEvent::Open(
                        "missing.md".into(),
                        true,
                    ))
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(!w.graph_open);
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("missing.md"));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("missing.md")).unwrap(),
            ""
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.md")).unwrap(),
            "[[b]] [[missing]]"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn duplicate_opens_snapshot_without_retargeting_original_tab(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-duplicate-ui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        std::fs::write(root.join("folder/source.md"), "original").unwrap();
        let original = handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab(
                    "folder/source.md".into(),
                    Some("original".into()),
                    false,
                    window,
                    cx,
                );
                let id = w.tabs[0].id;
                w.tabs[0].pane.read(cx).editor.clone().update(cx, |s, cx| {
                    s.set_selected_range(8..8, cx);
                    s.replace(" 新编辑😀", window, cx);
                });
                w.duplicate_current(window, cx);
                assert_eq!(w.ui.pending_file_writes, 1);
                id
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.ui.pending_file_writes, 0);
                assert_eq!(w.tabs.len(), 2);
                let old = w.tabs.iter().find(|t| t.id == original).unwrap();
                assert_eq!(old.path, PathBuf::from("folder/source.md"));
                assert_eq!(old.baseline.as_deref(), Some("original"));
                assert_eq!(
                    old.pane.read(cx).editor.read(cx).value(),
                    "original 新编辑😀"
                );
                let copy = &w.tabs[w.active.unwrap()];
                assert_eq!(copy.path, PathBuf::from("folder/source 副本.md"));
                assert_eq!(copy.baseline.as_deref(), Some("original 新编辑😀"));
                assert_eq!(
                    std::fs::read_to_string(root.join(&copy.path)).unwrap(),
                    "original 新编辑😀"
                );
                assert_eq!(
                    std::fs::read_to_string(root.join("folder/source.md")).unwrap(),
                    "original"
                );
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn explorer_sort_preserves_folders_selection_and_reacts_to_dates(cx: &mut TestAppContext) {
        use inkstone::file_order::SortBy;
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, cx| {
                let mut index = Index::default();
                index.update("note10.md".into(), "ten".into());
                index.update("note2.md".into(), "two".into());
                let old = std::time::SystemTime::UNIX_EPOCH;
                index
                    .notes
                    .get_mut(std::path::Path::new("note10.md"))
                    .unwrap()
                    .times
                    .modified = Some(old);
                index
                    .notes
                    .get_mut(std::path::Path::new("note2.md"))
                    .unwrap()
                    .times
                    .modified = Some(old + Duration::from_secs(10));
                w.index = Arc::new(index);
                w.ui.folders = vec!["Folder".into()];
                w.sync_index_ui(cx);
                assert!(w.tree.read(cx).entry(0).unwrap().item().is_folder());
                assert_eq!(
                    w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                    "note2.md"
                );
                w.tree
                    .update(cx, |tree, cx| tree.set_selected_index(Some(2), cx));
                w.ui.prefs.sort_descending = true;
                w.rebuild_sorted_tree(cx);
                assert!(w.tree.read(cx).entry(0).unwrap().item().is_folder());
                assert_eq!(
                    w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                    "note10.md"
                );
                assert_eq!(
                    w.tree.read(cx).selected_item().unwrap().label.as_ref(),
                    "note10.md"
                );
                w.ui.prefs.sort_by = SortBy::Modified;
                w.sync_index_ui(cx);
                assert_eq!(
                    w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                    "note2.md"
                );
                Arc::make_mut(&mut w.index)
                    .notes
                    .get_mut(std::path::Path::new("note10.md"))
                    .unwrap()
                    .times
                    .modified = Some(old + Duration::from_secs(20));
                w.sync_index_ui(cx);
                assert_eq!(
                    w.tree.read(cx).entry(1).unwrap().item().label.as_ref(),
                    "note10.md"
                );
                assert_eq!(
                    w.tree.read(cx).selected_item().unwrap().label.as_ref(),
                    "note10.md"
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn inline_title_renames_original_tab_and_preserves_failed_input(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-inline-title-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        std::fs::write(root.join("folder/old.md"), "body").unwrap();
        std::fs::write(root.join("folder/other.md"), "[[old]]").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.ui.prefs.always_update_links = true;
                w.add_tab(
                    "folder/old.md".into(),
                    Some("body".into()),
                    false,
                    window,
                    cx,
                );
                w.add_tab(
                    "folder/other.md".into(),
                    Some("[[old]]".into()),
                    false,
                    window,
                    cx,
                );
                w.begin_inline_title(0, false, window, cx);
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(handle.into(), cx);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        handle
            .update(cx, |w, window, cx| {
                let input = w.ui.inline_title.as_ref().unwrap().input.clone();
                input.update(cx, |s, cx| s.set_value("新名😀", window, cx));
                w.focus_primary(1, window, cx);
            })
            .unwrap();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(
                    root.join("folder/新名😀.md").is_file(),
                    "status={}, title={:?}, pending={}",
                    w.status,
                    w.ui.inline_title
                        .as_ref()
                        .map(|edit| edit.input.read(cx).value()),
                    w.ui.pending_file_writes
                );
                assert!(!root.join("folder/old.md").exists());
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("folder/other.md")
                );
                assert!(
                    std::fs::read_to_string(root.join("folder/other.md"))
                        .unwrap()
                        .contains("新名😀")
                );
                assert!(w.ui.inline_title.is_none());
                w.begin_inline_title(0, false, window, cx);
                let input = w.ui.inline_title.as_ref().unwrap().input.clone();
                input.update(cx, |s, cx| s.set_value("../bad", window, cx));
                w.commit_inline_title(true, window, cx);
                assert!(!w.ui.file_operation);
                assert!(w.ui.inline_title.is_some());
                input.update(cx, |s, cx| s.set_value("other", window, cx));
                w.commit_inline_title(true, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.ui.inline_title
                        .as_ref()
                        .unwrap()
                        .input
                        .read(cx)
                        .value()
                        .as_ref(),
                    "other"
                );
                assert_eq!(
                    std::fs::read_to_string(root.join("folder/新名😀.md")).unwrap(),
                    "body"
                );
                w.cancel_inline_title(window, cx);
                assert!(w.ui.inline_title.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn rename_link_prompt_supports_skip_once_always_and_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-link-prompt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "A").unwrap();
        std::fs::write(root.join("source.md"), "[[a]]").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                w.name.update(cx, |s, cx| s.set_value("b.md", window, cx));
                w.manage_note(false, window, cx);
                w.ui.property_open = true;
                w.ui.property_value
                    .update(cx, |s, cx| s.set_value("未提交值", window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("b.md").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            "[[a]]"
        );
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.ui.link_update.as_ref().unwrap().len(), 1);
                w.close_overlays(window, cx);
                assert!(w.ui.property_open);
                assert_eq!(w.ui.property_value.read(cx).value(), "未提交值");
                w.close_overlays(window, cx);
            })
            .unwrap();
        std::fs::write(root.join("source.md"), "[[b]]").unwrap();
        for (name, always) in [("c.md", false), ("d.md", true)] {
            handle
                .update(cx, |w, window, cx| {
                    w.name.update(cx, |s, cx| s.set_value(name, window, cx));
                    w.manage_note(false, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            handle
                .update(cx, |w, window, cx| {
                    assert!(w.ui.link_update.is_some());
                    w.confirm_link_updates(always, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(
                std::fs::read_to_string(root.join("source.md")).unwrap(),
                format!("[[/{}]]", name.trim_end_matches(".md"))
            );
        }
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.prefs.always_update_links);
                w.name.update(cx, |s, cx| s.set_value("e.md", window, cx));
                w.manage_note(false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            "[[/e]]"
        );
        handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.link_update.is_none());
                w.ui.prefs.always_update_links = false;
                w.name.update(cx, |s, cx| s.set_value("f.md", window, cx));
                w.manage_note(false, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::write(root.join("source.md"), "外部变更").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.confirm_link_updates(false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            "外部变更"
        );
        handle
            .update(cx, |w, _, _| {
                assert!(w.status.contains("未更新"));
                assert!(!w.ui.file_operation);
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn folder_move_updates_disk_open_tabs_and_session_paths(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-folder-move-ui-{}", std::process::id()));
        std::fs::create_dir_all(root.join("old")).unwrap();
        let source = "[[target]] [outside](../outside.md)";
        std::fs::write(root.join("old/note.md"), source).unwrap();
        std::fs::write(root.join("old/target.md"), "target").unwrap();
        std::fs::write(root.join("outside.md"), "[[old/note|alias]]").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.ui.prefs.always_update_links = true;
                let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.vault = Some(vault);
                w.add_tab("old/note.md".into(), Some(source.into()), false, window, cx);
                w.ui.prefs.bookmarks.push("old/note.md".into());
                w.ui.closed.push("old/target.md".into());
                w.manage_folder("old".into(), Some("archive/new".into()), window, cx);
                assert!(w.ui.file_operation);
                assert_eq!(w.ui.pending_file_writes, 1);
                w.save_all(window, cx);
                assert!(!w.tabs[0].saving);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.ui.file_operation);
                assert_eq!(w.ui.pending_file_writes, 0);
                assert_eq!(w.tabs[0].path, PathBuf::from("archive/new/note.md"));
                assert_eq!(
                    w.ui.prefs.bookmarks[0],
                    PathBuf::from("archive/new/note.md")
                );
                assert_eq!(w.ui.closed[0], PathBuf::from("archive/new/target.md"));
                let disk = std::fs::read_to_string(root.join("archive/new/note.md")).unwrap();
                assert_eq!(disk, "[[/archive/new/target]] [outside](../../outside.md)");
                assert_eq!(w.tabs[0].baseline.as_ref(), Some(&disk));
                assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), disk);
                assert!(!w.tabs[0].conflict);
                assert_eq!(
                    std::fs::read_to_string(root.join("outside.md")).unwrap(),
                    "[[/archive/new/note|alias]]"
                );
            })
            .unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.manage_folder("archive/new".into(), Some("final".into()), window, cx);
                let editor = w.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| {
                    let end = s.value().len();
                    s.set_selected_range(end..end, cx);
                    s.replace(" 新编辑😀", window, cx);
                });
                w.tabs[0].dirty = true;
                w.save_pending(window, cx);
                assert!(!w.tabs[0].saving);
                w.refresh_requested = true;
                w.refresh(window, cx);
                assert!(w.refresh_requested);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.tabs[0].path, PathBuf::from("final/note.md"));
                assert!(w.tabs[0].conflict);
                assert!(
                    w.tabs[0]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .ends_with(" 新编辑😀")
                );
                assert!(!root.join("archive/new").exists());
                let disk = std::fs::read_to_string(root.join("final/note.md")).unwrap();
                assert_eq!(disk, "[[/final/target]] [outside](../outside.md)");
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn embedded_task_write_checks_the_source_snapshot(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-embedded-task-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = "- [ ] 源任务😀\r\n";
        std::fs::write(root.join("source.md"), source).unwrap();
        let marker = inkstone::index::parse(source).tasks[0].marker.clone();
        let target = inkstone::rendering::TaskTarget {
            rendered_start: 0,
            path: "source.md".into(),
            marker,
            baseline: std::sync::Arc::from(source),
        };
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.toggle_referenced_task(target.clone(), true, window, cx);
                assert_eq!(w.ui.pending_file_writes, 1);
                assert!(!w.request_window_close(window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            "- [x] 源任务😀\r\n"
        );
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.ui.pending_file_writes, 0);
                assert!(
                    w.index
                        .notes
                        .get(std::path::Path::new("source.md"))
                        .unwrap()
                        .parsed
                        .tasks[0]
                        .checked
                );
            })
            .unwrap();
        std::fs::write(root.join("source.md"), "外部已更新").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.toggle_referenced_task(target, false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("source.md")).unwrap(),
            "外部已更新"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn window_close_waits_for_note_and_session_writes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-shutdown-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("关闭保存.md".into(), None, true, window, cx);
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.replace("关闭时的内容😀", window, cx));
                assert!(!w.request_window_close(window, cx));
                assert!(w.ui.window_close_requested);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.request_window_close(window, cx));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("关闭保存.md")).unwrap(),
            "关闭时的内容😀"
        );
        let prefs =
            inkstone::preferences::Preferences::load(&root.join(".inkstone-workspace.json"));
        assert_eq!(prefs.active_path, Some("关闭保存.md".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn split_views_share_edits_and_undo_but_keep_independent_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("a.md".into(), Some("中文😀".into()), false, window, cx);
                w.split_active(false, window, cx);
                let editor = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| {
                    s.set_selected_range("中文😀".len().."中文😀".len(), cx);
                    s.replace(" 新文本", window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.views.secondary_focused);
                assert!(w.tabs[0].dirty);
                let canonical = w.tabs[0].pane.read(cx).editor.clone();
                assert_eq!(canonical.read(cx).value().as_ref(), "中文😀 新文本");
                assert_eq!(canonical.read(cx).selected_range(), 0..0);
                canonical.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let mirror = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
                assert_eq!(mirror.read(cx).value().as_ref(), "中文😀");
                assert!(!w.tabs[0].dirty);
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.redo(&gpui_component::input::Redo, window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.views
                        .split
                        .as_ref()
                        .unwrap()
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .as_ref(),
                    "中文😀 新文本"
                );
                w.add_tab("b.md".into(), Some("B".into()), false, window, cx);
                assert_eq!(w.tabs[w.main_tab().unwrap()].path, PathBuf::from("a.md"));
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("b.md"));
                assert_ne!(w.tabs[1].pane, w.current_pane().unwrap());
                w.close_split(window, cx);
                assert!(w.views.split.is_none());
                assert_eq!(w.active, Some(0));
                assert_eq!(w.tabs.len(), 2);
            })
            .unwrap();
    }

    #[gpui::test]
    fn quick_switcher_results_stay_inside_short_window_and_scroll_to_selection(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.files = (0..30)
                    .map(|i| PathBuf::from(format!("folder/中文笔记-{i:02}.md")))
                    .collect();
                w.focus_search(false, window, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(500.)));
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, _, _| {
                let bounds = w.ui.modal_scroll.bounds();
                assert!(
                    bounds.size.height > px(0.) && bounds.size.height <= px(360.),
                    "{bounds:?}"
                );
                assert!(bounds.bottom() <= px(484.), "{bounds:?}");
            })
            .unwrap();
        visual.simulate_keystrokes("down down down down down down down down down down down down down down down down down down down down");
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.ui.selected, 20);
                let bounds = w.ui.modal_scroll.bounds();
                let item = w.ui.modal_scroll.bounds_for_item(20).unwrap();
                let offset = w.ui.modal_scroll.offset();
                assert!(offset.y < px(0.));
                assert!(item.top() + offset.y >= bounds.top() - px(1.));
                assert!(item.bottom() + offset.y <= bounds.bottom() + px(1.));
            })
            .unwrap();
    }

    #[gpui::test]
    fn reopen_closed_tab_restores_mode_selection_and_folds(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-reopen-view-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let text = "# Heading\nbody\nmore\n\n# Second\ntail";
        std::fs::write(root.join("note.md"), text).unwrap();
        let handle = cx.add_window(Workspace::new);
        let expected = handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("note.md".into(), Some(text.into()), false, window, cx);
                let pane = w.current_pane().unwrap();
                pane.update(cx, |p, cx| {
                    p.fold_sections(Some(true), window, cx);
                    p.reading = true;
                    p.live = false;
                    p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
                });
                let state = Workspace::snapshot_view("note.md".into(), &pane, cx);
                assert!(!state.folded_lines.is_empty());
                w.close_tab_at(0, window, cx);
                assert!(w.tabs.is_empty());
                w.execute_command(17, window, cx);
                state
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs.len(), 1);
                let pane = w.current_pane().unwrap();
                let actual = Workspace::snapshot_view("note.md".into(), &pane, cx);
                assert!(actual.reading);
                assert!(!actual.live);
                assert_eq!(actual.selection, expected.selection);
                assert_eq!(actual.folded_lines, expected.folded_lines);
                pane.update(cx, |p, cx| p.focus_view(window, cx));
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn middle_click_closes_only_the_pressed_tab_and_preserves_conflicts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                for name in ["a", "b", "c"] {
                    w.add_tab(
                        format!("{name}.md").into(),
                        Some(name.into()),
                        false,
                        window,
                        cx,
                    );
                }
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let position = handle
            .update(&mut visual, |w, _, _| {
                w.ui.tab_scroll.bounds_for_item(0).unwrap().center() + w.ui.tab_scroll.offset()
            })
            .unwrap();
        visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 3))
            .unwrap();
        visual.simulate_mouse_down(position, MouseButton::Middle, Modifiers::default());
        visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("c.md"));
                w.tabs[0].conflict = true;
            })
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let position = handle
            .update(&mut visual, |w, _, _| {
                w.ui.tab_scroll.bounds_for_item(0).unwrap().center() + w.ui.tab_scroll.offset()
            })
            .unwrap();
        visual.simulate_mouse_down(position, MouseButton::Middle, Modifiers::default());
        visual.simulate_mouse_up(position, MouseButton::Middle, Modifiers::default());
        handle
            .update(&mut visual, |w, _, _| assert_eq!(w.tabs.len(), 2))
            .unwrap();
    }

    #[gpui::test]
    fn closing_background_tab_preserves_current_editor(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                for name in ["a", "b", "c"] {
                    w.add_tab(
                        format!("{name}.md").into(),
                        Some(name.into()),
                        false,
                        window,
                        cx,
                    );
                }
                let active = w.tabs[2].pane.clone();
                w.close_tab_at(0, window, cx);
                assert_eq!(w.tabs[w.active.unwrap()].pane, active);
                assert!(
                    active
                        .read(cx)
                        .editor
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                assert_eq!(w.ui.closed, vec![PathBuf::from("a.md")]);
            })
            .unwrap();
    }
    #[gpui::test]
    fn session_restores_interleaved_blank_tabs_and_active_blank(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-blank-session-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "A").unwrap();
        std::fs::write(root.join("b.md"), "B").unwrap();
        inkstone::preferences::Preferences {
            open_paths: ["missing.md", "", "a.md", "", "b.md", ""]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            active_path: Some(PathBuf::new()),
            active_tab_index: Some(3),
            ..Default::default()
        }
        .save(&root.join(".inkstone-workspace.json"))
        .unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs.iter().map(|t| t.path.clone()).collect::<Vec<_>>(),
                    ["", "a.md", "", "b.md", ""]
                        .into_iter()
                        .map(PathBuf::from)
                        .collect::<Vec<_>>()
                );
                assert_eq!(w.active, Some(2));
                w.add_tab("c.md".into(), Some("C".into()), false, window, cx);
                assert_eq!(w.tabs.len(), 5);
                assert_eq!(
                    w.tabs
                        .iter()
                        .filter(|t| t.path.as_os_str().is_empty())
                        .count(),
                    2
                );
                w.watcher = None;
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn empty_tabs_reuse_their_slot_and_new_notes_receive_unique_names(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-blank-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.new_blank(window, cx);
                assert!(w.tabs[0].path.as_os_str().is_empty());
                assert!(!w.tabs[0].dirty);
                w.focus_new(window, cx);
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(w.tabs[0].path, PathBuf::from("未命名.md"));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.new_blank(window, cx);
                assert_eq!(w.tabs.len(), 2);
                w.open_note("未命名.md".into(), window, cx);
                assert_eq!(w.tabs.len(), 1);
                w.focus_new(window, cx);
                assert_eq!(w.tabs[1].path, PathBuf::from("未命名 1.md"));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("未命名.md").is_file());
        assert!(root.join("未命名 1.md").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn restores_split_orientation_document_and_independent_modes(cx: &mut TestAppContext) {
        use inkstone::preferences::{Preferences, ViewState};
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root =
            std::env::temp_dir().join(format!("inkstone-split-session-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "# A\nbody").unwrap();
        std::fs::write(root.join("b.md"), "# B\nbody").unwrap();
        Preferences {
            open_paths: vec!["a.md".into(), "b.md".into()],
            active_path: Some("b.md".into()),
            main_path: Some("a.md".into()),
            split_view: Some(ViewState {
                path: "b.md".into(),
                reading: true,
                selection: 2..2,
                folded_lines: vec![0],
                ..Default::default()
            }),
            split_vertical: true,
            split_focused: true,
            views: vec![ViewState {
                path: "a.md".into(),
                live: false,
                selection: 1..1,
                folded_lines: vec![0],
                ..Default::default()
            }],
            ..Default::default()
        }
        .save(&root.join(".inkstone-workspace.json"))
        .unwrap();
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert!(
                    w.views.vertical,
                    "status={} prefs={:?}",
                    w.status, w.ui.prefs
                );
                assert!(w.views.secondary_focused);
                assert_eq!(w.tabs[w.main_tab().unwrap()].path, PathBuf::from("a.md"));
                assert!(!w.tabs[0].pane.read(cx).live);
                assert_eq!(
                    w.tabs[0]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .folded_ranges()
                        .len(),
                    1
                );
                let pane = w.current_pane().unwrap();
                assert!(pane.read(cx).reading);
                assert_eq!(pane.read(cx).editor.read(cx).selected_range(), 2..2);
                assert_eq!(pane.read(cx).editor.read(cx).folded_ranges().len(), 1);
                assert!(!w.tabs[1].pane.read(cx).reading);
                assert!(
                    w.tabs[1]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .folded_ranges()
                        .is_empty()
                );
                w.snapshot_views(cx);
                assert_eq!(w.ui.prefs.views[0].folded_lines, vec![0]);
                assert_eq!(
                    w.ui.prefs.split_view.as_ref().unwrap().folded_lines,
                    vec![0]
                );
                w.watcher = None;
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn dirty_tab_closes_after_save_and_stays_open_on_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-close-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab("draft.md".into(), None, true, window, cx);
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.replace("不能丢失😀", window, cx));
                w.close_tab(window, cx);
                assert_eq!(w.tabs.len(), 1);
                assert!(w.tabs[0].saving);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("draft.md")).unwrap(),
            "不能丢失😀"
        );
        handle
            .update(cx, |w, window, cx| {
                assert!(w.tabs.is_empty());
                w.add_tab(
                    "draft.md".into(),
                    Some("不能丢失😀".into()),
                    false,
                    window,
                    cx,
                );
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.replace("本地", window, cx));
                std::fs::write(root.join("draft.md"), "外部").unwrap();
                w.close_tab(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert_eq!(w.tabs.len(), 1);
                assert!(w.tabs[0].conflict);
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("draft.md")).unwrap(),
            "外部"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn split_composition_is_not_saved_and_external_changes_require_resolution(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let root = std::env::temp_dir().join(format!("inkstone-split-ime-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.md"), "原文").unwrap();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("a.md".into(), Some("原文".into()), false, window, cx);
                w.split_active(false, window, cx);
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| {
                        s.set_selected_range(6..6, cx);
                        s.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                    });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                    "原文"
                );
                assert!(!w.tabs[0].dirty);
                w.close_split(window, cx);
                assert!(w.views.split.is_some());
                std::fs::write(root.join("a.md"), "外部更新").unwrap();
                w.refresh(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.tabs[0].conflict);
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.replace_text_in_range(None, "你", window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[0].pane.read(cx).editor.read(cx).value().as_ref(),
                    "原文你"
                );
                w.save_all(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("a.md")).unwrap(),
            "外部更新"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn bulk_tab_closing_preserves_pins_anchor_and_conflicts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                for name in ["a.md", "b.md", "c.md", "d.md"] {
                    w.add_tab(name.into(), Some(String::new()), false, window, cx);
                }
                w.ui.prefs.pinned_paths.push("b.md".into());
                let a = w.tabs[0].id;
                let c = w.tabs[2].id;
                w.close_tab_group(Some(c), 1, window, cx);
                assert_eq!(w.tabs.len(), 3);
                assert_eq!(w.tabs[w.active.unwrap()].id, c);
                w.close_tab_group(Some(a), 0, window, cx);
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(w.tabs[w.active.unwrap()].id, a);
                w.tabs[0].conflict = true;
                w.close_tab_group(None, 2, window, cx);
                assert_eq!(w.tabs.len(), 2);
                w.tabs[0].conflict = false;
                w.close_tab_group(None, 2, window, cx);
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(w.tabs[0].path, PathBuf::from("b.md"));
                w.ui.close_pending.insert(w.tabs[0].id);
                w.finish_pending_closes(window, cx);
                assert!(w.ui.close_pending.is_empty());
                w.ui.prefs.pinned_paths.clear();
                w.finish_pending_closes(window, cx);
                assert_eq!(w.tabs.len(), 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn automatic_file_reveal_expands_parents_and_can_be_disabled(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let mut index = Index::default();
                index.update("one/deep/a.md".into(), "A".into());
                index.update("two/b.md".into(), "B".into());
                w.index = Arc::new(index);
                w.sync_index_ui(cx);
                w.add_tab("one/deep/a.md".into(), Some("A".into()), false, window, cx);
                assert!(w.tree.read(cx).index_of(&"one/deep/a.md".into()).is_none());
                w.ui.prefs.auto_reveal_file = true;
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, window, cx| {
                assert_eq!(
                    w.tree
                        .read(cx)
                        .selected_item()
                        .unwrap()
                        .id
                        .replace('\\', "/"),
                    "one/deep/a.md"
                );
                w.ui.prefs.auto_reveal_file = false;
                w.add_tab("two/b.md".into(), Some("B".into()), false, window, cx);
            })
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, window, cx| {
                assert_eq!(
                    w.tree
                        .read(cx)
                        .selected_item()
                        .unwrap()
                        .id
                        .replace('\\', "/"),
                    "one/deep/a.md"
                );
                w.ui.prefs.left_open = false;
                w.ui.left_mode = 1;
                w.execute_command(74, window, cx);
                assert!(w.ui.prefs.left_open);
                assert_eq!(w.ui.left_mode, 0);
                assert_eq!(
                    w.tree
                        .read(cx)
                        .selected_item()
                        .unwrap()
                        .id
                        .replace('\\', "/"),
                    "two/b.md"
                );
                assert!(!w.ui.prefs.auto_reveal_file);
            })
            .unwrap();
    }

    #[gpui::test]
    fn many_tabs_keep_header_bounded_and_reveal_active_tab(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                for i in 0..12 {
                    w.add_tab(
                        format!("很长的标签名称-{i}.md").into(),
                        Some(String::new()),
                        false,
                        window,
                        cx,
                    );
                }
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, _, _| {
                let bounds = w.ui.tab_scroll.bounds();
                assert!(
                    bounds.size.width > px(0.) && bounds.right() < px(800.),
                    "{bounds:?}"
                );
                assert!(
                    w.ui.tab_scroll.offset().x < px(0.),
                    "tabs={}, selected={:?}, bounds={:?}, offset={:?}, last={:?}",
                    w.tabs.len(),
                    w.main_tab(),
                    bounds,
                    w.ui.tab_scroll.offset(),
                    w.ui.tab_scroll.bounds_for_item(11)
                );
            })
            .unwrap();
        handle
            .update(&mut visual, |w, window, cx| w.focus_primary(0, window, cx))
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |w, _, _| {
                assert!(
                    w.ui.tab_scroll.offset().x >= px(-5.),
                    "{:?}",
                    w.ui.tab_scroll.offset()
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn completion_link_preferences_keep_full_labels_and_resolve(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, _, _| {
                let mut index = Index::default();
                for name in [
                    "a/same.md",
                    "same.md",
                    "a/sub/中文.2026.md",
                    "other/unique.md",
                ] {
                    index.update(name.into(), String::new());
                }
                w.index = Arc::new(index);
                let from = std::path::Path::new("a/source.md");
                use inkstone::locations::LinkFormat;
                for format in [
                    LinkFormat::Shortest,
                    LinkFormat::Relative,
                    LinkFormat::Absolute,
                ] {
                    for markdown in [false, true] {
                        w.ui.prefs.link_format = format;
                        w.ui.prefs.use_markdown_links = markdown;
                        let entries = w.link_paths_for(from);
                        for (path, entry) in w.index.notes.keys().zip(entries.iter()) {
                            assert_eq!(
                                entry.label,
                                path.with_extension("").to_string_lossy().replace('\\', "/")
                            );
                            assert_eq!(entry.markdown, markdown);
                            let resolved = if markdown {
                                w.index.resolve_markdown(from, &entry.target).0
                            } else {
                                w.index.resolve(from, &entry.target)
                            };
                            assert_eq!(
                                resolved,
                                inkstone::index::Resolution::Found(path.clone()),
                                "{format:?}: {}",
                                entry.target
                            );
                        }
                    }
                }
                Arc::make_mut(&mut w.index).files = [
                    "media/photo.png",
                    "media/record.mp3",
                    "media/中文#1.pdf",
                    "data/private.bin",
                ]
                .into_iter()
                .map(PathBuf::from)
                .collect();
                w.ui.prefs.link_format = LinkFormat::Shortest;
                w.ui.prefs.use_markdown_links = false;
                let entries = w.link_paths_for(from);
                let photo = entries
                    .iter()
                    .find(|e| e.label == "media/photo.png")
                    .unwrap();
                assert_eq!(photo.target, "photo.png");
                assert!(!photo.markdown);
                assert!(entries.iter().any(|e| e.label == "media/record.mp3"));
                assert!(
                    entries
                        .iter()
                        .find(|e| e.label == "media/中文#1.pdf")
                        .unwrap()
                        .markdown
                );
                assert!(!entries.iter().any(|e| e.label.ends_with("private.bin")));
                Arc::make_mut(&mut w.index).update(
                    "other/unique.md".into(),
                    "---\naliases: [项目别名, 项目别名, a|b]\n---\n".into(),
                );
                let entries = w.link_paths_for(from);
                assert_eq!(
                    entries
                        .iter()
                        .filter(|e| e.alias.as_deref() == Some("项目别名"))
                        .count(),
                    1
                );
                let alias = entries
                    .iter()
                    .find(|e| e.alias.as_deref() == Some("项目别名"))
                    .unwrap();
                assert_eq!(alias.target, "unique");
                let unsafe_alias = entries
                    .iter()
                    .find(|e| e.alias.as_deref() == Some("a|b"))
                    .unwrap();
                assert!(unsafe_alias.markdown);
                assert_eq!(unsafe_alias.target, "unique.md");
            })
            .unwrap();
    }

    #[gpui::test]
    fn insert_footnote_command_preserves_text_and_undo_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.add_tab(
                    "footnote.md".into(),
                    Some("中文😀".into()),
                    false,
                    window,
                    cx,
                );
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(3..10, cx));
                w.execute_command(72, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中[^1]文😀\n\n[^1]: \n");
                assert_eq!(editor.read(cx).selected_range(), 7..7);
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
                assert_eq!(editor.read(cx).selected_range(), 3..10);
            })
            .unwrap();
    }

    #[gpui::test]
    fn table_commands_insert_align_remove_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.execute_command(61, window, cx);
                assert!(w.tabs.is_empty());
                w.add_tab("table.md".into(), Some(String::new()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                w.execute_command(60, window, cx);
                assert!(editor.read(cx).value().contains("| --- | --- |"));
                w.execute_command(63, window, cx);
                assert!(editor.read(cx).value().contains("| --- | --- | --- |"));
                w.execute_command(66, window, cx);
                let aligned = editor.read(cx).value();
                assert!(aligned.contains("| --- | :---: | --- |"));
                w.execute_command(64, window, cx);
                assert!(editor.read(cx).value().contains("| --- | --- |"));
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value(), aligned);
            })
            .unwrap();
    }

    #[gpui::test]
    fn preferences_formatting_and_pinned_tabs_work(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.add_tab("中文.md".into(), Some("中文😀".into()), false, window, cx);
                let editor = workspace.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(0.."中文😀".len(), cx));
                workspace.execute_command(24, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
                workspace.execute_command(24, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "中文😀");
                assert_eq!(editor.read(cx).selected_range(), 0.."中文😀".len());
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
                assert_eq!(editor.read(cx).selected_range(), 2..2 + "中文😀".len());
                workspace.execute_command(46, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "**==中文😀==**");
                workspace.execute_command(46, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
                workspace.execute_command(53, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "# **中文😀**");
                workspace.execute_command(52, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
                workspace.execute_command(48, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "> **中文😀**");
                workspace.execute_command(48, window, cx);
                workspace.execute_command(51, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "```\n**中文😀**\n```");
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), "**中文😀**");
                assert_eq!(editor.read(cx).selected_range(), 2..2 + "中文😀".len());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                assert!(workspace.tabs[0].dirty);
                workspace.tabs[0].baseline = Some(
                    workspace.tabs[0]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .to_string(),
                );
                workspace.tabs[0].dirty = false;
                workspace.execute_command(27, window, cx);
                workspace.close_tab(window, cx);
                assert_eq!(workspace.tabs.len(), 1);
                workspace.execute_command(27, window, cx);
                workspace.close_tab(window, cx);
                assert!(workspace.tabs.is_empty());
                assert_eq!(workspace.ui.closed, vec![PathBuf::from("中文.md")]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn session_restores_open_tabs_active_note_and_empty_folders(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-session-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("空文件夹")).unwrap();
        std::fs::write(root.join("a.md"), "# A").unwrap();
        std::fs::write(root.join("b.md"), "# B").unwrap();
        let prefs = inkstone::preferences::Preferences {
            open_paths: vec!["b.md".into(), "a.md".into()],
            active_path: Some("b.md".into()),
            light: true,
            font_size: 20.,
            ..Default::default()
        };
        prefs.save(&root.join(".inkstone-workspace.json")).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("b.md"));
                assert_eq!(w.tabs[0].pane.read(cx).font_size, 20.);
                assert!(w.ui.prefs.light);
                let tree = w.tree.read(cx);
                let id: SharedString = "空文件夹".into();
                assert!(tree.entry(tree.index_of(&id).unwrap()).unwrap().is_folder());
                w.watcher = None;
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn reopening_existing_tab_moves_keyboard_focus(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                workspace.add_tab("b.md".into(), Some("B".into()), false, window, cx);
                assert!(
                    workspace.tabs[1]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                workspace.open_note("a.md".into(), window, cx);
                assert!(
                    workspace.tabs[0]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn ten_thousand_backlinks_leave_the_editor_visible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "中文正文 😀 e\u{301}\n".repeat(400);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.add_tab("目标.md".into(), Some(source.clone()), false, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let editor = handle
            .update(cx, |workspace, _, cx| {
                workspace.tabs[0].pane.read(cx).editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-end shift-left");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let before = editor.read_with(&visual, |state, _| (state.selected_range(), state.cursor()));
        handle
            .update(&mut visual, |workspace, _, cx| {
                workspace.backlinks = (0..10000)
                    .map(|i| PathBuf::from(format!("{i:05}.md")))
                    .collect();
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
        editor.read_with(&visual, |state, _| {
            assert!(state.input_bounds().size.height > px(200.));
            assert_eq!(state.value().as_ref(), source);
            assert_eq!((state.selected_range(), state.cursor()), before);
            let (mut caret, _) = state.cursor_layout().unwrap();
            caret.origin += state.scroll_offset();
            assert!(state.input_bounds().intersects(&caret));
        });
        handle
            .update(&mut visual, |workspace, _, _| {
                assert_eq!(workspace.backlinks.len(), 10000)
            })
            .unwrap();
    }

    #[gpui::test]
    fn missing_link_creation_backlinks_and_search(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-links-test-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("来源.md"), "# 来源\n\n线索 [[目标]]").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.load_vault(root.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                workspace.follow_link("来源.md".into(), "目标".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("目标.md").exists());
        handle
            .update(cx, |workspace, window, cx| workspace.refresh(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                assert_eq!(workspace.backlinks, vec![PathBuf::from("来源.md")]);
                workspace.fulltext = true;
                workspace.search.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "线索", window, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, _, _| {
                assert_eq!(workspace.search_results.len(), 1);
                assert_eq!(workspace.search_results[0].path, PathBuf::from("来源.md"));
            })
            .unwrap();
        handle
            .update(cx, |workspace, _, cx| {
                let pane = workspace.tabs[workspace.active.unwrap()].pane.clone();
                pane.update(cx, |_, cx| {
                    cx.emit(EditorEvent::FollowMarkdownLink("不会创建.md".into()))
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!root.join("不会创建.md").exists());
        handle
            .update(cx, |workspace, _, cx| {
                assert!(workspace.status.contains("目标不存在"));
                let pane = workspace.tabs[workspace.active.unwrap()].pane.clone();
                pane.update(cx, |_, cx| {
                    cx.emit(EditorEvent::FollowMarkdownLink(
                        "%E6%9D%A5%E6%BA%90.md".into(),
                    ))
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, _, _| {
                assert_eq!(
                    workspace.tabs[workspace.active.unwrap()].path,
                    PathBuf::from("来源.md")
                );
                workspace.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn recovery_opens_exact_draft_as_new_note_without_overwriting_original(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-recovery-ui-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        let original = "# 原件\r\n磁盘版本 😀\r\n";
        let draft = "# 原件\r\n本地草稿 👩‍💻 e\u{301} **保留格式**\r\n";
        std::fs::write(root.join("原件.md"), original).unwrap();
        let vault = Vault::open(root.clone(), app_dir().join("recovery")).unwrap();
        let journal = vault
            .journal(std::path::Path::new("原件.md"), Some(original), draft)
            .unwrap();
        drop(vault);

        // A fresh workspace reads the durable journal, as it would after restart.
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.load_vault(root.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                assert_eq!(workspace.recoveries.len(), 1);
                workspace.restore_draft(0, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let restored = handle
            .update(cx, |workspace, _, cx| {
                let tab = &workspace.tabs[workspace.active.unwrap()];
                assert_ne!(tab.path, std::path::Path::new("原件.md"));
                assert!(!tab.dirty && !tab.saving && tab.error.is_none());
                assert_eq!(tab.pane.read(cx).editor.read(cx).value().as_ref(), draft);
                let path = root.join(&tab.path);
                workspace.watcher = None;
                path
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(restored).unwrap(), draft);
        assert_eq!(
            std::fs::read_to_string(root.join("原件.md")).unwrap(),
            original
        );
        assert!(
            journal.exists(),
            "restoring must preserve the original recovery journal"
        );
        std::fs::remove_file(journal).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn gpui_create_edit_save_reopen_and_external_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-ui-vault-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |workspace, window, cx| {
                workspace.load_vault(root.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                assert!(workspace.vault.is_some());
                workspace
                    .name
                    .update(cx, |state, cx| state.set_value("测试.md", window, cx));
                workspace.create_note(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(root.join("测试.md")).unwrap(), "");
        handle
            .update(cx, |workspace, window, cx| {
                let editor = workspace.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "# 中文😀\r\n**原文**\r\n", window, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                assert!(workspace.tabs[0].dirty);
                workspace.save_all(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let persisted = std::fs::read_to_string(root.join("测试.md")).unwrap();
        assert_eq!(persisted, "# 中文😀\r\n**原文**\r\n");
        handle
            .update(cx, |workspace, window, cx| {
                assert!(!workspace.tabs[0].dirty);
                workspace.tabs.clear();
                workspace.active = None;
                workspace.open_note(PathBuf::from("测试.md"), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, window, cx| {
                let editor = workspace.tabs[0].pane.read(cx).editor.clone();
                assert_eq!(editor.read(cx).value().as_ref(), persisted);
                editor.update(cx, |state, cx| {
                    state.replace_text_in_range(None, "本地修改", window, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        std::fs::write(root.join("测试.md"), "外部修改").unwrap();
        handle
            .update(cx, |workspace, window, cx| workspace.save_all(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |workspace, _, cx| {
                assert!(workspace.tabs[0].conflict);
                assert!(workspace.tabs[0].dirty);
                assert!(
                    workspace.tabs[0]
                        .pane
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .contains("本地修改")
                );
                assert!(workspace.status.contains("外部版本"));
                workspace.watcher = None;
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("测试.md")).unwrap(),
            "外部修改"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
