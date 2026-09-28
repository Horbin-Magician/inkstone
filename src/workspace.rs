use crate::editor::{EditorEvent, EditorPane};
use gpui::{prelude::*, *};
use gpui_component::{
    button::*,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    scroll::ScrollableElement,
    tree::{Tree, TreeItem, TreeState},
};
use inkstone::index::{Index, Resolution, SearchHit};
use inkstone::vault::{RecoveryEntry, Vault, VaultError};
use notify::{RecursiveMode, Watcher};
use std::sync::Arc;
use std::{path::PathBuf, time::Duration};

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
        ClosePalette
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
}
pub struct Workspace {
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
    pending_jump: Option<(PathBuf, usize)>,
    backlinks: Vec<PathBuf>,
    backlink_scroll: UniformListScrollHandle,
    completion_paths: Arc<Vec<String>>,
    tree: Entity<TreeState>,
    tree_files: Vec<PathBuf>,
    command_open: bool,
    changed_paths: std::collections::BTreeSet<PathBuf>,
    rescan: bool,
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
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            Ok::<_, VaultError>((files, documents, index))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation { return; }
                this.refreshing = false;
                match result {
                    Ok((files, documents, index)) => {
                        this.files = files;
                        this.index = Arc::new(index);
                        this.sync_index_ui(cx);
                        for (id, path, baseline, disk) in documents {
                            let Some(tab) = this.tabs.iter_mut().find(|t|t.id == id) else { continue; };
                            if tab.saving || tab.path!=path || tab.baseline != baseline { this.refresh_requested = true; continue; }
                            if disk == tab.baseline { continue; }
                            if tab.dirty || disk.is_none() {
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
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
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
        let mut name = self.name.read(cx).value().trim().to_string();
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
        let task = cx.background_executor().spawn(async move {
            if trash {
                vault.trash_note(&path, &baseline).map(|p| (true, p))
            } else {
                vault
                    .rename_note(&path, &dest, &baseline)
                    .map(|_| (false, dest))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if generation != this.generation {
                    return;
                }
                let Some(index) = this.tabs.iter().position(|t| t.id == id) else {
                    return;
                };
                this.tabs[index].saving = false;
                match result {
                    Ok((true, path)) => {
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
                    Ok((false, path)) => {
                        this.tabs[index].path = path;
                        this.status = "笔记已重命名。现有双链原文保留。".into();
                    }
                    Err(error) => this.status = error.to_string(),
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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名 / 全文"));
        let name_subscription = cx.subscribe_in(&name, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create_note(window, cx);
            }
        });
        let search_subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.run_search(cx);
            }
            if matches!(event, InputEvent::PressEnter { .. })
                && let Some(hit) = this.search_results.first()
            {
                let path = hit.path.clone();
                this.pending_jump = Some((path.clone(), hit.offset));
                this.open_note(path, window, cx);
                this.apply_jump(window, cx);
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
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                if this.tabs.iter().any(|t| t.dirty || t.saving) {
                    this.status = "仍有未保存内容，关闭已阻止。请保存，或在冲突处理后重试。".into();
                    cx.notify();
                    false
                } else {
                    true
                }
            })
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
        Self {
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
            pending_jump: None,
            backlinks: vec![],
            backlink_scroll: UniformListScrollHandle::new(),
            completion_paths: Arc::new(vec![]),
            tree,
            tree_files: vec![],
            command_open: false,
            changed_paths: Default::default(),
            rescan: false,
        }
    }
    fn choose_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.iter().any(|t| t.dirty || t.saving) {
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
        if self.tabs.iter().any(|t| t.dirty || t.saving) {
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
            Ok::<_, VaultError>((vault, files, watcher, receiver, recoveries, index))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                if this.tabs.iter().any(|t| t.dirty || t.saving) {
                    this.status = "读取期间产生了新编辑，已保留当前笔记库。".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((vault, files, watcher, receiver, recoveries, index)) => {
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
                        this.tabs.clear();
                        this.active = None;
                        this.status =
                            "笔记库已打开。每 2 秒自动保存，恢复副本在应用恢复区。".into();
                        if let Some(path) = this.files.first().cloned() {
                            this.open_note(path, window, cx);
                        }
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
                    return;
                }
                match result {
                    Ok(Some(text)) => this.add_tab(path, Some(text), false, window, cx),
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
        let pane = cx.new(|cx| EditorPane::new(baseline.as_deref().unwrap_or(""), window, cx));
        pane.update(cx, |pane, _| pane.set_paths(self.completion_paths.clone()));
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
                }
            }
        });
        let subscription = cx.subscribe(&editor, move |this, editor, event, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id)
            {
                tab.dirty = tab.baseline.as_deref() != Some(editor.read(cx).value().as_ref());
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
        });
        self.active = Some(self.tabs.len() - 1);
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
        for tab in &mut self.tabs {
            if !tab.conflict {
                tab.error = None;
            }
        }
        self.save_pending(window, cx);
    }
    fn save_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                let _ = this.update_in(cx, |this, _, cx| {
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
        self.command_open = false;
        self.search.update(cx, |state, cx| state.focus(window, cx));
        self.run_search(cx);
        cx.notify();
    }
    fn focus_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_open = false;
        self.name.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    fn activate_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(index);
        self.tabs[index]
            .pane
            .update(cx, |pane, cx| pane.focus_view(window, cx));
        self.run_search(cx);
        cx.notify();
    }
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active else {
            return;
        };
        if self.tabs[index].dirty || self.tabs[index].saving {
            self.status = "请保存或另存副本后关闭标签。".into();
            cx.notify();
            return;
        }
        self.tabs.remove(index);
        self.active = if self.tabs.is_empty() {
            None
        } else {
            Some(index.min(self.tabs.len() - 1))
        };
        if let Some(index) = self.active {
            self.tabs[index]
                .pane
                .update(cx, |pane, cx| pane.focus_view(window, cx));
        }
        self.run_search(cx);
        cx.notify();
    }
    fn sync_index_ui(&mut self, cx: &mut Context<Self>) {
        self.completion_paths = Arc::new(
            self.index
                .notes
                .keys()
                .map(|p| {
                    format!(
                        "/{}",
                        p.with_extension("").to_string_lossy().replace('\\', "/")
                    )
                })
                .collect(),
        );
        for tab in &self.tabs {
            tab.pane.update(cx, |pane, _| {
                pane.set_paths(self.completion_paths.clone());
                if let Some(vault) = &self.vault {
                    pane.image_dir = vault
                        .root
                        .join(tab.path.parent().unwrap_or(std::path::Path::new("")));
                }
            });
        }
        let files: Vec<_> = self.index.notes.keys().cloned().collect();
        if self.tree_files != files {
            self.tree_files = files.clone();
            let items = make_tree(&files);
            self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        }
    }
    fn run_search(&mut self, cx: &mut Context<Self>) {
        self.search_revision += 1;
        let revision = self.search_revision;
        let generation = self.generation;
        let query = self.search.read(cx).value().to_string();
        let index = self.index.clone();
        let fulltext = self.fulltext;
        let active = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.path.clone());
        let task = cx.background_executor().spawn(async move {
            let backlinks = active.map(|p| index.backlinks(&p)).unwrap_or_default();
            let hits = if query.trim().is_empty() {
                vec![]
            } else if fulltext {
                index.search(&query)
            } else {
                index
                    .filenames(&query)
                    .into_iter()
                    .map(|path| SearchHit {
                        path,
                        offset: 0,
                        line: 1,
                        excerpt: String::new(),
                    })
                    .collect()
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
        tab.pane
            .update(cx, |pane, cx| pane.jump(offset, window, cx));
        self.pending_jump = None;
    }
    fn follow_markdown_link(
        &mut self,
        from: PathBuf,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (resolution, heading) = self.index.resolve_markdown(&from, href);
        match resolution {
            Resolution::Found(path) => {
                if let Some(h) = heading.and_then(|heading| {
                    self.index
                        .notes
                        .get(&path)
                        .and_then(|n| n.parsed.headings.iter().find(|h| h.title == heading))
                }) {
                    self.pending_jump = Some((path.clone(), h.offset));
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
                    && let Some(h) = self
                        .index
                        .notes
                        .get(&path)
                        .and_then(|n| n.parsed.headings.iter().find(|h| h.title == heading))
                {
                    self.pending_jump = Some((path.clone(), h.offset));
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
        self.save_all(window, cx);
        cx.notify();
    }
}
impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_id = self
            .active
            .and_then(|i| self.tabs.get(i))
            .map_or(0, |tab| tab.id);
        let backlink_list = uniform_list(
            ("backlinks", active_id),
            self.backlinks.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|i| {
                        this.backlinks.get(i).cloned().map(|path| {
                            ListItem::new(("backlink", i))
                                .h(px(28.))
                                .accessibility_label(path.display().to_string())
                                .child(div().truncate().child(path.display().to_string()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_note(path.clone(), window, cx)
                                }))
                        })
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.backlink_scroll)
        .h(px(self.backlinks.len().min(4) as f32 * 28.))
        .w_full();
        let weak = cx.entity().downgrade();
        let file_tree = Tree::new(&self.tree, move |i, entry, _, _, _| {
            let path = PathBuf::from(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let prefix = if folder {
                if entry.is_expanded() { "▾ " } else { "▸ " }
            } else {
                "  "
            };
            let weak = weak.clone();
            ListItem::new(i)
                .accessibility_label(entry.item().label.clone())
                .child(
                    div()
                        .pl(px(entry.depth() as f32 * 12.))
                        .child(format!("{prefix}{}", entry.item().label)),
                )
                .on_click(move |_, window, cx| {
                    if !folder {
                        let _ =
                            weak.update(cx, |this, cx| this.open_note(path.clone(), window, cx));
                    }
                })
        });
        let title = self
            .vault
            .as_ref()
            .map(|v| v.root.display().to_string())
            .unwrap_or_else(|| "尚未打开笔记库".into());
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x151c28))
            .text_color(rgb(0xe6edf6))
            .on_action(cx.listener(|this, _: &Save, window, cx| this.save_all(window, cx)))
            .on_action(cx.listener(|this, _: &OpenVault, window, cx| this.choose_vault(window, cx)))
            .on_action(
                cx.listener(|this, _: &QuickOpen, window, cx| this.focus_search(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FullSearch, window, cx| this.focus_search(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.focus_new(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_tab(window, cx)))
            .on_action(cx.listener(|this, _: &CommandPalette, _, cx| {
                this.command_open = !this.command_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ClosePalette, _, cx| {
                this.command_open = false;
                cx.notify();
            }))
            .when(self.command_open, |s| {
                s.child(
                    div()
                        .p_3()
                        .bg(rgb(0x283446))
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child("命令面板 · Esc 关闭")
                        .child(Button::new("cmd-open").label("打开库 Ctrl+O").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.command_open = false;
                                this.choose_vault(window, cx);
                            }),
                        ))
                        .child(Button::new("cmd-new").label("新笔记 Ctrl+N").on_click(
                            cx.listener(|this, _, window, cx| this.focus_new(window, cx)),
                        ))
                        .child(Button::new("cmd-find").label("快速打开 Ctrl+P").on_click(
                            cx.listener(|this, _, window, cx| this.focus_search(false, window, cx)),
                        ))
                        .child(
                            Button::new("cmd-search")
                                .label("全文搜索 Ctrl+Shift+F")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.focus_search(true, window, cx)
                                })),
                        )
                        .child(
                            Button::new("cmd-save")
                                .label("保存 Ctrl+S")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.command_open = false;
                                    this.save_all(window, cx);
                                })),
                        )
                        .child(Button::new("cmd-close").label("关闭标签 Ctrl+W").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.command_open = false;
                                this.close_tab(window, cx);
                            }),
                        )),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .child(
                        Button::new("open-vault").label("打开笔记库").on_click(
                            cx.listener(|this, _, window, cx| this.choose_vault(window, cx)),
                        ),
                    )
                    .child(
                        Button::new("save")
                            .label("保存 Ctrl+S")
                            .on_click(cx.listener(|this, _, window, cx| this.save_all(window, cx))),
                    )
                    .child(
                        Button::new("save-copy").label("另存为副本").on_click(
                            cx.listener(|this, _, window, cx| this.save_copy(window, cx)),
                        ),
                    )
                    .child(Button::new("commands").label("命令 Ctrl+Shift+P").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.command_open = !this.command_open;
                            cx.notify();
                        }),
                    ))
                    .child(div().text_sm().child(title)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(240.))
                            .p_3()
                            .gap_2()
                            .border_r_1()
                            .border_color(rgb(0x303b4d))
                            .child(Input::new(&self.search))
                            .child(
                                Button::new("search-mode")
                                    .label(if self.fulltext {
                                        "全文搜索"
                                    } else {
                                        "文件名搜索"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.fulltext = !this.fulltext;
                                        this.run_search(cx);
                                        cx.notify();
                                    })),
                            )
                            .when(!self.search.read(cx).value().is_empty(), |s| {
                                s.child(
                                    div()
                                        .id("search-results")
                                        .max_h(px(240.))
                                        .overflow_y_scroll()
                                        .children(self.search_results.iter().enumerate().map(
                                            |(i, hit)| {
                                                let path = hit.path.clone();
                                                let offset = hit.offset;
                                                div()
                                                    .id(("hit", i))
                                                    .p_2()
                                                    .cursor_pointer()
                                                    .child(format!(
                                                        "{}:{} {}",
                                                        path.display(),
                                                        hit.line,
                                                        hit.excerpt
                                                    ))
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.pending_jump =
                                                                Some((path.clone(), offset));
                                                            this.open_note(
                                                                path.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                            this.apply_jump(window, cx);
                                                        },
                                                    ))
                                            },
                                        )),
                                )
                            })
                            .child(Input::new(&self.name))
                            .child(Button::new("new-note").label("创建笔记").on_click(
                                cx.listener(|this, _, window, cx| this.create_note(window, cx)),
                            ))
                            .child(
                                Button::new("rename-note")
                                    .label("重命名为上述名称")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.manage_note(false, window, cx)
                                    })),
                            )
                            .child(Button::new("trash-note").label("移入回收区").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.manage_note(true, window, cx)
                                }),
                            ))
                            .child(
                                div()
                                    .id("recoveries")
                                    .flex()
                                    .flex_col()
                                    .max_h(px(150.))
                                    .overflow_y_scroll()
                                    .children(self.recoveries.iter().enumerate().map(
                                        |(i, entry)| {
                                            Button::new(("recover", i))
                                                .label(format!(
                                                    "恢复：{}",
                                                    entry.record.relative.display()
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.restore_draft(i, window, cx)
                                                    },
                                                ))
                                        },
                                    )),
                            )
                            .child(div().flex_1().min_h_0().child(file_tree)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .p_2()
                                    .gap_2()
                                    .child(format!("反向链接（{}）", self.backlinks.len()))
                                    .child(
                                        div()
                                            .relative()
                                            .child(backlink_list)
                                            .vertical_scrollbar(&self.backlink_scroll),
                                    ),
                            )
                            .child(div().flex().flex_wrap().gap_1().p_2().children(
                                self.tabs.iter().enumerate().map(|(i, tab)| {
                                    let suffix = if tab.conflict || tab.error.is_some() {
                                        " ⚠"
                                    } else if tab.dirty {
                                        " ●"
                                    } else {
                                        ""
                                    };
                                    Button::new(("tab", tab.id))
                                        .label(format!("{}{suffix}", tab.path.display()))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.activate_tab(i, window, cx)
                                        }))
                                }),
                            ))
                            .child(
                                div().flex_1().min_h_0().children(
                                    self.active
                                        .and_then(|i| self.tabs.get(i))
                                        .map(|t| t.pane.clone()),
                                ),
                            )
                            .when(self.tabs.is_empty(), |s| {
                                s.child(div().p_6().child(if self.loading {
                                    "正在读取笔记库…"
                                } else {
                                    "打开文件夹，或输入名称创建第一篇 Markdown 笔记。"
                                }))
                            }),
                    ),
            )
            .child(
                div().p_2().text_sm().text_color(rgb(0xffc08a)).children(
                    self.active
                        .and_then(|i| self.tabs.get(i))
                        .and_then(|t| t.error.clone()),
                ),
            )
            .child(div().p_2().text_sm().child(self.status.clone()))
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
