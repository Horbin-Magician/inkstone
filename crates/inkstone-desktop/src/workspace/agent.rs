//! Workspace adapter: the agent can stage proposals, only this layer applies them.
use super::ui::{icon, tool};
use super::*;
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Enter, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    spinner::Spinner,
    text::TextView,
};
use inkstone_agent::{
    AgentEvent, AgentRun, RunState, Scope,
    provider::ModelConfig,
    runtime::{Cancel, RequestMode, RunRequest},
    storage::{self, Message, Proposal, Session, Settings},
    tools::{Source, hash},
};
use inkstone_core::changes::{ApplyReport, ApplyStatus};
use std::collections::BTreeMap;

type AgentSnapshot = (PathBuf, String, std::ops::Range<usize>);

const POLISH: &str = "请润色当前选区或笔记，保留原意，提出修改供我审阅。";
const SUMMARY: &str = "请总结当前选区或笔记，列出要点并引用来源。";
const PROBE_OK: &str = "流式连接与工具参数测试通过";
const SCOPES: [(&str, &str); 4] = [
    ("file-text", "当前笔记"),
    ("folder", "指定文件夹"),
    ("list", "指定笔记"),
    ("library", "整个笔记库"),
];
const SUGGESTIONS: [(&str, &str, &str); 4] = [
    ("file-text", "总结要点", SUMMARY),
    ("pencil", "润色文字", POLISH),
    (
        "list-checks",
        "提取待办",
        "请从当前选区或笔记中提取待办事项，整理为任务列表。",
    ),
    (
        "quote",
        "解释概念",
        "请解释当前选区或笔记中的关键概念，并引用相关来源。",
    ),
];

pub(super) struct State {
    settings: Settings,
    secrets: BTreeMap<String, String>,
    fields: Vec<Entity<InputState>>,
    prompt: Entity<TextareaState>,
    scope_input: Entity<InputState>,
    scope: usize,
    mode: RequestMode,
    scope_key: String,
    sessions: Vec<Session>,
    selected: usize,
    root: Option<PathBuf>,
    generation: u64,
    run: Option<AgentRun>,
    stream_task: Option<Task<()>>,
    sequence: u64,
    applying: Option<Cancel>,
    show_tools: bool,
    error: String,
    persistence_error: bool,
    writer: storage::SessionWriter,
    last_checkpoint: std::time::Instant,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let settings = storage::read::<Settings>(&app_dir().join("agent-settings.json"));
        let secrets = storage::load_secrets(&app_dir());
        let error = settings
            .as_ref()
            .err()
            .or(secrets.as_ref().err())
            .map(ToString::to_string)
            .unwrap_or_default();
        let mut settings = settings.unwrap_or_default();
        if settings.models.is_empty() {
            settings.models.push(ModelConfig::default());
        }
        settings.selected = settings.selected.min(settings.models.len() - 1);
        let fields: Vec<_> = [
            "配置名称",
            "服务地址（含 /v1）",
            "模型名称",
            "API Key（本地明文保存）",
            "上下文容量",
            "输出上限",
            "排除目录，逗号分隔",
        ]
        .into_iter()
        .map(|label| cx.new(|cx| InputState::new(window, cx).placeholder(label)))
        .collect();
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 10)
                .submit_on_enter(true)
                .placeholder("向 Agent 提问或描述任务")
        });
        let mut state = Self {
            settings,
            secrets: secrets.unwrap_or_default(),
            fields,
            prompt,
            scope_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("库内目录 / 笔记路径，逗号分隔")),
            scope: 0,
            mode: RequestMode::Chat,
            scope_key: String::new(),
            sessions: vec![Session::default()],
            selected: 0,
            root: None,
            generation: 0,
            run: None,
            stream_task: None,
            sequence: 0,
            applying: None,
            show_tools: false,
            writer: Default::default(),
            last_checkpoint: std::time::Instant::now(),
            persistence_error: false,
            error,
            scroll: ScrollHandle::new(),
        };
        state.fields[3].update(cx, |s, cx| s.set_masked(true, window, cx));
        state.fill_fields(window, cx);
        state
    }
    fn fill_fields(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        let model = &self.settings.models[self.settings.selected];
        let values = [
            model.name.clone(),
            model.base_url.clone(),
            model.model.clone(),
            self.secrets.get(&model.id).cloned().unwrap_or_default(),
            model.context_tokens.to_string(),
            model.output_tokens.to_string(),
            self.settings
                .excluded
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join(","),
        ];
        for (field, value) in self.fields.iter().zip(values) {
            field.update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }
    fn session(&self) -> &Session {
        &self.sessions[self.selected]
    }
    fn session_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.selected]
    }
    fn busy(&self) -> bool {
        self.run.is_some() || self.applying.is_some()
    }
    fn persist(&mut self) {
        if self.persistence_error {
            return;
        }
        if let Some(root) = &self.root {
            self.writer.queue(&app_dir(), root, self.sessions.clone());
        }
        self.last_checkpoint = std::time::Instant::now();
    }
}
impl Workspace {
    pub(super) fn open_agent(
        &mut self,
        command: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ui.prefs.right_open = true;
        self.ui.focus_mode = false;
        self.ui.right_mode = 5;
        let prompt = match command {
            113 => POLISH,
            114 => SUMMARY,
            _ => "",
        };
        if !prompt.is_empty() {
            self.agent
                .prompt
                .update(cx, |s, cx| s.set_value(prompt, window, cx));
        }
        window.focus(&self.agent.prompt.focus_handle(cx), cx);
        cx.notify();
    }
    pub(super) fn poll_agent(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let root = self.vault.as_ref().map(|v| v.root.clone());
        if self.agent.root != root || self.agent.generation != self.generation {
            self.agent.run.take();
            if let Some(cancel) = self.agent.applying.take() {
                cancel.cancel();
            }
            if matches!(
                self.agent.session().state,
                RunState::Preparing | RunState::Requesting | RunState::Reading | RunState::Applying
            ) {
                self.agent.session_mut().state = RunState::Interrupted;
            }
            self.agent.persist();
            if let Err(error) = self.agent.writer.flush() {
                self.agent.error = error.to_string();
            }
            self.agent.root = root.clone();
            self.agent.generation = self.generation;
            self.agent.selected = 0;
            self.agent.scope = 0;
            self.agent.scope_key.clear();
            match root
                .as_ref()
                .map(|root| storage::load_sessions(&app_dir(), root))
                .transpose()
            {
                Ok(sessions) => {
                    self.agent.sessions = sessions.unwrap_or_default();
                    self.agent.persistence_error = false;
                }
                Err(error) => {
                    self.agent.sessions.clear();
                    self.agent.error = error.to_string();
                    self.agent.persistence_error = true;
                }
            }
            if self.agent.sessions.is_empty() {
                self.agent.sessions.push(Session::default());
            }
            cx.notify();
        }
        for error in self.agent.writer.errors.try_iter() {
            self.agent.error = error;
        }
        let mut events = vec![];
        let mut finished = false;
        if let Some(run) = &self.agent.run {
            if run.generation != self.generation || Some(&run.vault) != self.agent.root.as_ref() {
                run.cancel.cancel();
                finished = true;
            } else if run.cancel.cancelled() {
                events.push(AgentEvent::State(RunState::Cancelled));
                finished = true;
            } else {
                for _ in 0..128 {
                    match run.events.try_recv() {
                        Ok(AgentEvent::Finished) => {
                            finished = true;
                            break;
                        }
                        Ok(event) => events.push(event),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            finished = true;
                            break;
                        }
                    }
                }
            }
        }
        let changed = !events.is_empty() || finished;
        for event in events {
            match event {
                AgentEvent::State(state) => {
                    if state == RunState::Requesting {
                        self.agent.session_mut().usage = None;
                    }
                    self.agent.session_mut().state = state;
                }
                AgentEvent::Text(text) => {
                    if let Some(message) = self.agent.session_mut().messages.last_mut() {
                        message.text.push_str(&text);
                    }
                }
                AgentEvent::Tool { name, result } => {
                    self.agent.session_mut().tools.push((name, result))
                }
                AgentEvent::Sources(sources) => {
                    let session = self.agent.session_mut();
                    for source in sources {
                        if !session.sources.iter().any(|s| s.id == source.id) {
                            session.sources.push(source);
                        }
                    }
                }
                AgentEvent::Proposal(changes, previews) => {
                    self.agent.session_mut().proposals.push(Proposal {
                        previews,
                        changes,
                        status: "等待审阅".into(),
                        report: None,
                    })
                }
                AgentEvent::Usage(usage) => self.agent.session_mut().usage = Some(usage),
                AgentEvent::Error(error) => self.agent.error = error,
                AgentEvent::Probe => self.agent.error = PROBE_OK.into(),
                AgentEvent::Finished => (),
            }
        }
        if finished {
            if matches!(
                self.agent.session().state,
                RunState::Preparing | RunState::Requesting | RunState::Reading
            ) {
                self.agent.session_mut().state = RunState::Interrupted;
                self.agent.error = "执行已中断；保留现有输出，不自动重发请求".into();
            }
            self.agent.run.take();
            self.agent.persist();
        }
        if changed {
            if self.agent.last_checkpoint.elapsed() >= Duration::from_secs(1) {
                self.agent.persist();
            }
            // Follow streamed output unless the reader has scrolled up.
            let scroll = &self.agent.scroll;
            if scroll.offset().y + scroll.max_offset().y <= px(48.) {
                scroll.scroll_to_bottom();
            }
            cx.notify();
        }
    }
    fn agent_save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let values: Vec<_> = self
            .agent
            .fields
            .iter()
            .map(|f| f.read(cx).value().to_string())
            .collect();
        let old = &self.agent.settings.models[self.agent.settings.selected];
        let config = ModelConfig {
            id: old.id.clone(),
            name: values[0].trim().into(),
            base_url: values[1].trim().into(),
            model: values[2].trim().into(),
            context_tokens: values[4].parse().unwrap_or(0),
            output_tokens: values[5].parse().unwrap_or(0),
        };
        let excluded: Vec<PathBuf> = values[6]
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .collect();
        if let Err(error) = config
            .validate()
            .and_then(|_| storage::check_exclusions(&excluded))
        {
            self.agent.error = error.to_string();
            cx.notify();
            return false;
        }
        self.agent.settings.models[self.agent.settings.selected] = config.clone();
        self.agent.settings.excluded = excluded;
        self.agent.secrets.insert(config.id, values[3].clone());
        let result = storage::save_secrets(&app_dir(), &self.agent.secrets).and_then(|_| {
            storage::write(&app_dir().join("agent-settings.json"), &self.agent.settings)
        });
        match result {
            Ok(()) => {
                self.agent.error.clear();
                self.agent.fill_fields(window, cx);
                cx.notify();
                true
            }
            Err(error) => {
                self.agent.error = error.to_string();
                cx.notify();
                false
            }
        }
    }
    fn agent_snapshot(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<AgentSnapshot>, String> {
        let Some(pane) = self.current_pane() else {
            return Ok(None);
        };
        let (path, input) = {
            let pane = pane.read(cx);
            (pane.current_path.clone(), pane.editor.clone())
        };
        if input.read(cx).is_composing()
            || self
                .tabs
                .iter()
                .filter(|t| t.path == path)
                .any(|t| self.has_pending_input(t.id, window, cx))
        {
            return Err("请先完成中文组词再发送".into());
        }
        let editor = input.read(cx);
        let text = editor.value().to_string();
        let selection = editor.selected_range();
        let range = if self.agent.scope == 0 && !selection.is_empty() {
            selection
        } else {
            0..text.len()
        };
        Ok(Some((path, text, range)))
    }
    fn agent_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_agent(window, cx);
        if self.agent.busy() {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let prompt = self.agent.prompt.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            return;
        }
        let config = self.agent.settings.models[self.agent.settings.selected].clone();
        if let Err(error) = config.validate() {
            self.agent.error = error.to_string();
            self.open_agent_settings(window, cx);
            cx.notify();
            return;
        }
        self.flush_document_views(window, cx);
        let snapshot = if self.agent.mode == RequestMode::Probe {
            None
        } else {
            match self.agent_snapshot(window, cx) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.agent.error = error;
                    cx.notify();
                    return;
                }
            }
        };
        let input = self.agent.scope_input.read(cx).value();
        let scope = match if self.agent.mode == RequestMode::Probe {
            3
        } else {
            self.agent.scope
        } {
            0 => match &snapshot {
                Some((path, _, _)) => Scope::Notes(vec![path.clone()]),
                None => {
                    self.agent.error = "请打开 Markdown 笔记，或选择文件夹 / 全库范围".into();
                    cx.notify();
                    return;
                }
            },
            1 => Scope::Folder(PathBuf::from(input.trim())),
            2 => Scope::Notes(
                input
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect(),
            ),
            _ => Scope::Vault,
        };
        let scope_key = format!(
            "{scope:?}:{:?}:{:?}",
            self.agent.settings.excluded,
            snapshot.as_ref().map(|(p, _, range)| (p, range))
        );
        // Never resend earlier broad-scope content after narrowing the task scope.
        let history = if self.agent.scope_key == scope_key {
            self.agent
                .session()
                .messages
                .iter()
                .map(|m| serde_json::json!({"role":m.role,"content":m.text}))
                .collect()
        } else {
            vec![]
        };
        self.agent.scope_key = scope_key;
        let session = self.agent.session_mut();
        if session.messages.is_empty() {
            session.title = prompt.chars().take(24).collect();
        }
        session.messages.push(Message {
            role: "user".into(),
            text: prompt.clone(),
        });
        session.messages.push(Message {
            role: "assistant".into(),
            text: String::new(),
        });
        session.state = RunState::Preparing;
        session.usage = None;
        self.agent.error.clear();
        self.agent.sequence = self.agent.sequence.wrapping_add(1);
        // Use a timestamp for source IDs so reopened sessions cannot reuse citations.
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        let key = self
            .agent
            .secrets
            .get(&config.id)
            .cloned()
            .unwrap_or_default();
        self.agent.run = Some(AgentRun::spawn(RunRequest {
            mode: self.agent.mode,
            id,
            generation: self.generation,
            vault,
            config,
            key,
            scope,
            excluded: self.agent.settings.excluded.clone(),
            snapshot,
            history,
            prompt,
        }));
        self.agent.stream_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let keep = this
                    .update_in(cx, |this, w, cx| {
                        if this.agent.run.as_ref().is_none_or(|run| run.id != id) {
                            return false;
                        }
                        this.poll_agent(w, cx);
                        this.agent.run.is_some()
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
        self.agent
            .prompt
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.agent.scroll.scroll_to_bottom();
        self.agent.persist();
        cx.notify();
    }
    fn agent_stop(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.agent.run {
            run.cancel.cancel();
        }
        if let Some(cancel) = &self.agent.applying {
            cancel.cancel();
        }
        cx.notify();
    }
    fn agent_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent.busy() {
            return;
        }
        // Reuse an untouched session instead of piling up empty entries.
        if !self.agent.session().messages.is_empty() {
            self.agent.sessions.push(Session::default());
            self.agent.selected = self.agent.sessions.len() - 1;
            self.agent.persist();
        }
        self.agent.scope_key.clear();
        self.agent.error.clear();
        window.focus(&self.agent.prompt.focus_handle(cx), cx);
        cx.notify();
    }
    fn agent_select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.agent.busy() || index >= self.agent.sessions.len() {
            return;
        }
        self.agent.selected = index;
        self.agent.scope_key.clear();
        self.agent.error.clear();
        self.agent.scroll.scroll_to_bottom();
        cx.notify();
    }
    fn agent_delete_session(&mut self, cx: &mut Context<Self>) {
        if self.agent.busy() {
            return;
        }
        self.agent.sessions.remove(self.agent.selected);
        if self.agent.sessions.is_empty() {
            self.agent.sessions.push(Session::default());
        }
        self.agent.selected = self.agent.sessions.len() - 1;
        self.agent.scope_key.clear();
        self.agent.persist();
        cx.notify();
    }
    fn agent_select_model(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent.busy() || index >= self.agent.settings.models.len() {
            return;
        }
        self.agent.settings.selected = index;
        self.agent.fill_fields(window, cx);
        cx.notify();
    }
    fn agent_suggest(&mut self, prompt: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.agent
            .prompt
            .update(cx, |s, cx| s.set_value(prompt, window, cx));
        window.focus(&self.agent.prompt.focus_handle(cx), cx);
        cx.notify();
    }
    fn agent_apply(&mut self, proposal_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent.busy() {
            return;
        }
        if self.agent.persistence_error {
            self.agent.error = "会话文件损坏，无法记录应用结果；请先修复会话存储".into();
            cx.notify();
            return;
        }
        let Some(proposal) = self.agent.session().proposals.get(proposal_index).cloned() else {
            return;
        };
        if proposal.status != "等待审阅" {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        let changes = proposal.changes;
        // A single open document is changed through its existing undoable editor.
        if changes.changes.len() == 1 {
            let change = &changes.changes[0];
            if let Some(tab) = self.tabs.iter().find(|t| t.path == change.path) {
                if change.before.as_deref() != Some(tab.save.editor.read(cx).value().as_ref())
                    || tab.save.persistence.is_saving()
                    || tab.save.persistence.has_conflict()
                    || self.has_pending_input(tab.id, window, cx)
                    || self.file_writes.operation_active()
                {
                    self.agent.error = "笔记已变化或正在保存，请重新生成预览".into();
                    cx.notify();
                    return;
                }
                let editor = tab.save.editor.clone();
                editor.update(cx, |state, cx| state.replace_all(&change.after, window, cx));
                self.document_changed(editor, window, cx);
                self.agent.session_mut().proposals[proposal_index].status =
                    "已应用到编辑器，可撤销；遵循原有保存设置".into();
                self.agent.session_mut().state = RunState::Completed;
                self.agent.persist();
                cx.notify();
                return;
            }
        }
        for change in &changes.changes {
            if self.tabs.iter().filter(|t| t.path == change.path).any(|t| {
                t.save.persistence.is_dirty()
                    || t.save.persistence.is_saving()
                    || t.save.persistence.has_conflict()
                    || self.has_pending_input(t.id, window, cx)
                    || change.before.as_deref() != Some(t.save.editor.read(cx).value().as_ref())
            }) {
                self.agent.error = "目标笔记有未保存编辑、冲突或组词，请先处理后重新审阅".into();
                cx.notify();
                return;
            }
        }
        let Some(ticket) = self.file_writes.try_begin_exclusive_operation() else {
            self.agent.error = "请等待现有文件操作或同步完成".into();
            cx.notify();
            return;
        };
        let cancel = Cancel::default();
        self.agent.applying = Some(cancel.clone());
        self.agent.session_mut().state = RunState::Applying;
        self.agent.session_mut().proposals[proposal_index].status = "正在应用".into();
        let session_id = self.agent.session().id.clone();
        let generation = self.generation;
        let root = vault.root.clone();
        self.agent.persist();
        if let Err(error) = self.agent.writer.flush() {
            self.file_writes.finish(ticket);
            self.agent.applying = None;
            self.agent.session_mut().proposals[proposal_index].status = "等待审阅".into();
            self.agent.error = error.to_string();
            cx.notify();
            return;
        }
        let task = cx.background_executor().spawn(async move {
            let report = changes.apply(&vault, || cancel.cancelled());
            (changes, report)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (changes, report) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation || this.agent.root.as_ref() != Some(&root) {
                    return;
                }
                this.agent.applying = None;
                this.flush_document_views(window, cx);
                for (change, (_, status)) in changes.changes.iter().zip(&report.files) {
                    if !matches!(status, ApplyStatus::Written { .. }) {
                        continue;
                    }
                    if let Some(tab) = this.tabs.iter().find(|t| t.path == change.path) {
                        let document = tab.save.clone();
                        let pending = this.has_pending_input(tab.id, window, cx);
                        if document.persistence.apply_reviewed_edit(
                            change.before.as_deref().unwrap_or_default(),
                            &change.after,
                            document.editor.read(cx).value().as_ref(),
                            pending,
                        ) {
                            document.editor.update(cx, |state, cx| {
                                state.replace_all(&change.after, window, cx)
                            });
                            this.document_changed(document.editor.clone(), window, cx);
                        }
                    }
                }
                let written = report
                    .files
                    .iter()
                    .filter(|(_, s)| matches!(s, ApplyStatus::Written { .. }))
                    .count();
                let message = format!(
                    "已写入 {written}/{} 篇；可在版本历史恢复",
                    report.files.len()
                );
                if let Some(session) = this.agent.sessions.iter_mut().find(|s| s.id == session_id) {
                    session.state = if written == report.files.len() {
                        RunState::Completed
                    } else {
                        RunState::Failed
                    };
                    if let Some(proposal) = session.proposals.get_mut(proposal_index) {
                        proposal.status = message.clone();
                        proposal.report = Some(report);
                    }
                }
                this.refresh_requested = true;
                this.rescan = true;
                this.notifications.publish(message);
                this.agent.persist();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn agent_source(&mut self, source: Source, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let current = self
            .tabs
            .iter()
            .find(|t| t.path == source.path)
            .map(|t| t.save.editor.read(cx).value().to_string());
        let text = current.or_else(|| vault.read(&source.path).ok().flatten());
        if text.as_deref().is_none_or(|t| hash(t) != source.hash) {
            self.agent.error = "引用来源已变化；打开当前版本，不再使用旧位置".into();
            self.open_note(source.path, window, cx);
        } else {
            self.pending_jump = Some((source.path.clone(), source.range.start));
            self.open_note(source.path.clone(), window, cx);
            self.apply_jump(window, cx);
            if let Some(tab) = self.tabs.iter().find(|t| t.path == source.path) {
                tab.save
                    .editor
                    .update(cx, |s, cx| s.set_selected_range(source.range, cx));
            }
        }
        cx.notify();
    }
    fn agent_test_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agent_save_settings(window, cx) || self.agent.busy() {
            return;
        }
        self.agent.prompt.update(cx, |s, cx| {
            s.set_value(
                "测试模型连接、流式回复与工具调用能力（不读取笔记）。",
                window,
                cx,
            )
        });
        self.agent.mode = RequestMode::Probe;
        self.agent_send(window, cx);
        self.agent.mode = RequestMode::Chat;
    }
    fn open_agent_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.settings = true;
        self.select_settings_tab(10, window, cx);
        window.focus(&self.ui.modal_focus, cx);
    }

    pub(super) fn agent_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.agent.busy();
        let mut rows = vec![
            self.settings_row(
                "agent-current-config",
                "当前配置",
                "切换要编辑和使用的模型配置。",
                self.agent_model_menu(
                    Button::new("agent-settings-model")
                        .h(px(32.))
                        .disabled(busy)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(self.agent_model_name())
                                .child(icon("chevrons-up-down")),
                        ),
                    false,
                    cx,
                ),
                false,
                16.,
            ),
        ];
        for (i, (id, title, description)) in [
            ("agent-config-name", "配置名称", "用于区分多个模型配置。"),
            (
                "agent-config-url",
                "服务地址",
                "模型服务的 API 基础地址，包含 /v1。",
            ),
            ("agent-config-model", "模型", "需要支持流式回复与工具调用。"),
            (
                "agent-config-key",
                "API Key",
                "密钥在本机以明文独立保存，不随笔记同步。",
            ),
            (
                "agent-config-context",
                "上下文容量",
                "单位为 token，包含输入与输出。",
            ),
            (
                "agent-config-output",
                "输出上限",
                "每次请求的最大输出 token，包含推理 token。",
            ),
            (
                "agent-config-excluded",
                "排除目录",
                "库内相对路径，以逗号分隔；对所有范围生效。",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            rows.push(self.settings_row(
                id,
                title,
                description,
                Input::new(&self.agent.fields[i]).w(px(320.)).disabled(busy),
                true,
                16.,
            ));
        }
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("agent-save-config")
                    .primary()
                    .label("保存配置")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.agent_save_settings(w, cx);
                    })),
            )
            .child(
                Button::new("agent-test-config")
                    .label("测试连接与工具")
                    .disabled(busy || self.vault.is_none())
                    .on_click(cx.listener(|this, _, w, cx| this.agent_test_model(w, cx))),
            )
            .child(
                Button::new("agent-add-config")
                    .label("新增配置")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.agent.settings.models.push(ModelConfig {
                            id: Session::default().id,
                            ..Default::default()
                        });
                        this.agent.settings.selected = this.agent.settings.models.len() - 1;
                        this.agent.fill_fields(w, cx);
                        cx.notify();
                    })),
            );
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                self.settings_content()
                    .gap_2()
                    .child(self.settings_title("Agent"))
                    .child(self.settings_group("模型与权限", rows))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(700.))
                            .mx_auto()
                            .px_4()
                            .child(actions),
                    )
                    .when(busy, |panel| panel.child("Agent 正在运行，请稍候。"))
                    .when(!self.agent.error.is_empty(), |panel| {
                        panel.child(self.agent.error.clone())
                    }),
            )
            .into_any_element()
    }

    fn agent_model_name(&self) -> String {
        let name = &self.agent.settings.models[self.agent.settings.selected].name;
        if name.trim().is_empty() {
            "未命名配置".into()
        } else {
            name.clone()
        }
    }
    fn agent_model_menu(&self, button: Button, manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let selected = self.agent.settings.selected;
        let names: Vec<_> = self
            .agent
            .settings
            .models
            .iter()
            .map(|m| m.name.clone())
            .collect();
        button
            .dropdown_menu(move |mut menu, _, _| {
                for (i, name) in names.iter().enumerate() {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(if name.trim().is_empty() {
                            "未命名配置".to_string()
                        } else {
                            name.clone()
                        })
                        .checked(i == selected)
                        .on_click(move |_, window, cx| {
                            let _ =
                                weak.update(cx, |this, cx| this.agent_select_model(i, window, cx));
                        }),
                    );
                }
                if manage {
                    let weak = weak.clone();
                    menu = menu.separator().item(
                        PopupMenuItem::new("管理模型配置…")
                            .icon(icon("settings"))
                            .on_click(move |_, window, cx| {
                                let _ = weak
                                    .update(cx, |this, cx| this.open_agent_settings(window, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn agent_header(&self, colors: crate::theme::Palette, cx: &mut Context<Self>) -> Div {
        let busy = self.agent.busy();
        let session = self.agent.session();
        let title = if session.title.trim().is_empty() {
            "新对话".to_string()
        } else {
            session.title.clone()
        };
        let status = if busy {
            Some(state_label(&session.state))
        } else {
            match session.state {
                RunState::Failed
                | RunState::Cancelled
                | RunState::Interrupted
                | RunState::Limited
                | RunState::AwaitingReview
                    if !session.messages.is_empty() =>
                {
                    Some(state_label(&session.state))
                }
                _ => None,
            }
        };
        let weak = cx.entity().downgrade();
        let selected = self.agent.selected;
        let titles: Vec<_> = self
            .agent
            .sessions
            .iter()
            .map(|s| {
                if s.title.trim().is_empty() {
                    "新对话".to_string()
                } else {
                    s.title.clone()
                }
            })
            .collect();
        let history = tool("agent-sessions", "history", "会话历史")
            .disabled(busy)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("最近会话").max_h(px(360.)).scrollable(true);
                // Newest first, matching how the sessions were created.
                for (i, title) in titles.iter().enumerate().rev() {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(title.clone())
                            .checked(i == selected)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    weak.update(cx, |this, cx| this.agent_select_session(i, cx));
                            }),
                    );
                }
                let weak = weak.clone();
                menu.separator().item(
                    PopupMenuItem::new("删除当前会话")
                        .icon(icon("trash"))
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| this.agent_delete_session(cx));
                        }),
                )
            });
        div()
            .flex()
            .items_center()
            .gap_2()
            .pl_3()
            .pr_2()
            .h(px(44.))
            .flex_shrink_0()
            .border_b_1()
            .border_color(colors.border)
            .child(icon("sparkles").text_color(colors.accent))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when_some(status, |col, status| {
                        col.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(colors.muted)
                                .when(busy, |row| {
                                    row.child(Spinner::new().xsmall().color(colors.accent.into()))
                                })
                                .child(status),
                        )
                    }),
            )
            .child(
                tool("agent-new", "message-square-plus", "新对话")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| this.agent_new_session(w, cx))),
            )
            .child(history)
            .child(
                tool("agent-settings", "settings", "Agent 设置")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_agent_settings(window, cx);
                    })),
            )
    }
    fn agent_empty(&self, colors: crate::theme::Palette, cx: &mut Context<Self>) -> Div {
        let mut suggestions = div().flex().flex_col().gap_2().w_full().max_w(px(320.));
        for (i, (ico, label, prompt)) in SUGGESTIONS.into_iter().enumerate() {
            suggestions = suggestions.child(
                div()
                    .id(("agent-suggestion", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(colors.border)
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.hover))
                    .child(icon(ico).text_color(colors.accent))
                    .child(label)
                    .on_click(cx.listener(move |this, _, w, cx| this.agent_suggest(prompt, w, cx))),
            );
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .px_3()
            .pt_8()
            .pb_4()
            .child(
                div()
                    .p_3()
                    .rounded(px(16.))
                    .bg(colors.selected)
                    .child(icon("sparkles").text_color(colors.accent)),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("今天想处理什么？"),
            )
            .child(
                div()
                    .text_center()
                    .text_color(colors.muted)
                    .child("梳理笔记、提炼要点或打磨文字，修改都会先由你审阅"),
            )
            .child(suggestions)
    }
    fn agent_message(
        &self,
        i: usize,
        message: &Message,
        thinking: bool,
        usage: Option<&serde_json::Value>,
        colors: crate::theme::Palette,
    ) -> AnyElement {
        let copy = message.text.clone();
        let group: SharedString = format!("agent-message-{i}").into();
        let copy_button = div()
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .child(
                Button::new(("agent-copy-message", i))
                    .ghost()
                    .xsmall()
                    .icon(icon("copy"))
                    .tooltip("复制")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
            );
        if message.role == "user" {
            return div()
                .group(group)
                .flex()
                .flex_col()
                .items_end()
                .gap_1()
                .child(
                    div()
                        .max_w(relative(0.88))
                        .px_3()
                        .py_2()
                        .rounded(px(14.))
                        .bg(colors.selected)
                        .child(message.text.clone()),
                )
                .child(copy_button)
                .into_any_element();
        }
        let body = if thinking {
            div()
                .flex()
                .items_center()
                .gap_2()
                .py_1()
                .text_color(colors.muted)
                .child(Spinner::new().small().color(colors.accent.into()))
                .child(format!("{}…", state_label(&self.agent.session().state)))
                .into_any_element()
        } else if message.text.is_empty() {
            div()
                .text_color(colors.muted)
                .child("（没有文字回复）")
                .into_any_element()
        } else {
            TextView::markdown(("agent-reply", i), message.text.clone())
                .selectable(true)
                .into_any_element()
        };
        div()
            .group(group)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(colors.muted)
                    .child(icon("sparkles").text_color(colors.accent))
                    .child("Agent"),
            )
            .child(body)
            .when(!thinking, |col| {
                let stats = usage.map(usage_stats).unwrap_or_default();
                col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_h(px(22.))
                        .when(!message.text.is_empty(), |row| row.child(copy_button))
                        .when(!stats.is_empty(), |row| {
                            row.child(self.agent_usage(i, &stats, colors))
                        }),
                )
            })
            .into_any_element()
    }
    fn agent_usage(
        &self,
        i: usize,
        stats: &[(&'static str, u64)],
        colors: crate::theme::Palette,
    ) -> impl IntoElement {
        let exact = stats
            .iter()
            .map(|(label, n)| format!("{label} {n} token"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut row = div()
            .id(("agent-usage", i))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_2()
            .gap_y_0p5()
            .text_xs()
            .text_color(colors.muted)
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(format!("最近一次请求用量\n{exact}"))
                    .build(window, cx)
            });
        for (j, (label, n)) in stats.iter().enumerate() {
            if j > 0 {
                row = row.child(div().size(px(3.)).rounded_full().bg(colors.border));
            }
            row = row.child(
                div().flex().items_baseline().gap_1().child(*label).child(
                    div()
                        .text_color(colors.foreground)
                        .font_weight(FontWeight::MEDIUM)
                        .child(compact_tokens(*n)),
                ),
            );
        }
        row
    }
    fn agent_proposal(
        &self,
        i: usize,
        proposal: &Proposal,
        colors: crate::theme::Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let busy = self.agent.busy();
        let pending = proposal.status == "等待审阅";
        let light = self.ui.prefs.light;
        let (added, removed) = if light {
            (rgb(0x2f7d4a), rgb(0xb4413c))
        } else {
            (rgb(0x86d19c), rgb(0xf08a84))
        };
        let (added_bg, removed_bg) = if light {
            (rgba(0x2f7d4a1a), rgba(0xb4413c17))
        } else {
            (rgba(0x86d19c1f), rgba(0xf08a841f))
        };
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let card = div()
            .debug_selector(move || format!("agent-proposal-{i}"))
            // Preserve the full card height inside the scrolling conversation.
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(if pending {
                colors.accent
            } else {
                colors.border
            })
            .bg(colors.background)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .bg(colors.surface)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(icon("square-pen").text_color(colors.accent))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "修改提案 {} · {} 篇",
                                i + 1,
                                proposal.changes.changes.len()
                            )),
                    )
                    .child(pill(
                        proposal.status.clone(),
                        if pending { colors.accent } else { colors.muted },
                        if pending {
                            colors.selected
                        } else {
                            colors.hover
                        },
                    )),
            );
        let mut body = div().flex().flex_col().gap_3().p_3();
        if proposal.changes.changes.len() > 1 {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(colors.muted)
                    .child("关联修改须整组接受，避免缺失链接目标。"),
            );
        }
        for (j, change) in proposal.changes.changes.iter().enumerate() {
            let diff = proposal
                .previews
                .get(j)
                .cloned()
                .unwrap_or_else(|| "旧会话未保存差异预览，请重新生成提案。".into());
            let mut lines = div().flex().flex_col().min_w_0();
            let total = diff.lines().count();
            for line in diff.lines().take(DIFF_LINES) {
                let (fg, bg) = match line.as_bytes().first() {
                    _ if line.starts_with("+++") || line.starts_with("---") => (colors.muted, None),
                    Some(b'+') => (added, Some(added_bg)),
                    Some(b'-') => (removed, Some(removed_bg)),
                    Some(b'@') => (colors.accent, None),
                    _ => (colors.foreground, None),
                };
                lines = lines.child(
                    div()
                        .px_2()
                        .text_color(fg)
                        .when_some(bg, |row, bg| row.bg(bg))
                        .child(if line.is_empty() {
                            " ".to_string()
                        } else {
                            line.to_string()
                        }),
                );
            }
            if total > DIFF_LINES {
                lines = lines.child(div().px_2().text_color(colors.muted).child(format!(
                    "……还有 {} 行未显示，完整修改仍保留。",
                    total - DIFF_LINES
                )));
            }
            let path = change.path.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon("file-text").text_color(colors.muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(change.path.display().to_string()),
                            )
                            .when(change.before.is_none(), |row| {
                                row.child(pill("新建", colors.accent, colors.selected))
                            })
                            .child(
                                Button::new(("agent-history", i * 100 + j))
                                    .ghost()
                                    .xsmall()
                                    .icon(icon("history"))
                                    .tooltip("查看版本历史 / 恢复")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.open_history_path(path.clone(), w, cx)
                                    })),
                            ),
                    )
                    .when(!change.reason.trim().is_empty(), |col| {
                        col.child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .child(change.reason.clone()),
                        )
                    })
                    .child(
                        div()
                            .id(("agent-diff", i * 100 + j))
                            .max_h(px(280.))
                            .overflow_y_scroll()
                            .py_1()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.surface)
                            .font_family(mono.clone())
                            .text_xs()
                            .child(lines),
                    ),
            );
        }
        if let Some(report) = &proposal.report {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(colors.muted)
                    .child(report_text(report)),
            );
        }
        if pending {
            body = body.child(
                div()
                    .debug_selector(move || format!("agent-proposal-actions-{i}"))
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new(("agent-reject", i))
                            .ghost()
                            .small()
                            .label("拒绝")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.agent.session_mut().proposals[i].status = "已拒绝".into();
                                this.agent.persist();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("agent-accept", i))
                            .primary()
                            .small()
                            .icon(icon("check"))
                            .label("接受并应用")
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.agent_apply(i, w, cx)),
                            ),
                    ),
            );
        }
        card.child(body)
    }
    fn agent_composer(&self, colors: crate::theme::Palette, cx: &mut Context<Self>) -> Div {
        let busy = self.agent.busy();
        let weak = cx.entity().downgrade();
        let scope = self.agent.scope;
        // Name what "current note" means right now so users know what will be sent.
        let current = self.current_pane().map(|pane| {
            let pane = pane.read(cx);
            let name = pane
                .current_path
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            (name, !pane.editor.read(cx).selected_range().is_empty())
        });
        let scope_label = match (scope, &current) {
            (0, Some((name, true))) => format!("选区 · {name}"),
            (0, Some((name, false))) if !name.is_empty() => name.clone(),
            (0, _) => "未打开笔记".into(),
            (i, _) => SCOPES[i].1.into(),
        };
        let scope_button = Button::new("agent-scope")
            .ghost()
            .small()
            .tooltip("发送给 Agent 的笔记范围")
            .disabled(busy)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .max_w(px(130.))
                    .child(icon(SCOPES[scope].0).text_color(colors.muted))
                    .child(div().truncate().child(scope_label))
                    .child(icon("chevron-down").text_color(colors.muted)),
            )
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("笔记范围");
                for (i, (ico, label)) in SCOPES.into_iter().enumerate() {
                    let weak = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(if i == 0 {
                            "当前笔记 / 选区"
                        } else {
                            label
                        })
                        .icon(icon(ico))
                        .checked(i == scope)
                        .on_click(move |_, window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.agent.scope = i;
                                if matches!(i, 1 | 2) {
                                    window.focus(&this.agent.scope_input.focus_handle(cx), cx);
                                }
                                cx.notify();
                            });
                        }),
                    );
                }
                menu
            });
        let model = self.agent_model_menu(
            Button::new("agent-model")
                .ghost()
                .small()
                .tooltip("切换模型配置")
                .disabled(busy)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .max_w(px(140.))
                        .child(div().truncate().child(self.agent_model_name()))
                        .child(icon("chevron-down").text_color(colors.muted)),
                ),
            true,
            cx,
        );
        let action = if busy {
            Button::new("agent-composer-stop")
                .icon(icon("stop"))
                .tooltip("停止")
                .rounded(px(10.))
                .on_click(cx.listener(|this, _, _, cx| this.agent_stop(cx)))
        } else {
            Button::new("agent-send")
                .primary()
                .icon(icon("arrow-up"))
                .tooltip("发送（Enter）")
                .rounded(px(10.))
                .disabled(self.vault.is_none())
                .on_click(cx.listener(|this, _, w, cx| this.agent_send(w, cx)))
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_3()
            .pt_2()
            .pb_2()
            .flex_shrink_0()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface)
                    .shadow_xs()
                    .when(matches!(scope, 1 | 2), |card| {
                        card.child(
                            div()
                                .px_2()
                                .pt_2()
                                .child(Input::new(&self.agent.scope_input).small().disabled(busy)),
                        )
                    })
                    .child(
                        // The textarea only propagates plain Enter (submit); handling it here
                        // also stops the key from falling back to a newline insertion.
                        div()
                            .min_w_0()
                            .px_1()
                            .pt_1()
                            .on_action(cx.listener(|this, _: &Enter, w, cx| this.agent_send(w, cx)))
                            .child(Textarea::new(&self.agent.prompt).appearance(false)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_1p5()
                            .pb_1p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(scope_button)
                                    .child(model),
                            )
                            .child(action.flex_shrink_0().w(px(30.)).h(px(30.))),
                    ),
            )
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(colors.muted)
                    .child("Enter 发送 · Shift+Enter 换行"),
            )
    }

    pub(super) fn agent_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = crate::theme::palette(self.ui.prefs.light);
        let busy = self.agent.busy();
        let mut panel = div()
            .id("agent-panel")
            .track_scroll(&self.agent.scroll)
            .min_w_0()
            .whitespace_normal()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .py_3()
            .gap_4();
        let session = self.agent.session();
        if session.messages.is_empty() {
            panel = panel.child(self.agent_empty(colors, cx));
        }
        let last = session.messages.len().saturating_sub(1);
        for (i, message) in session.messages.iter().enumerate() {
            let thinking = busy && i == last && message.role != "user" && message.text.is_empty();
            // Usage belongs to the latest request, so it sits under the latest reply.
            let usage = session.usage.as_ref().filter(|_| i == last);
            panel = panel.child(self.agent_message(i, message, thinking, usage, colors));
        }
        if !session.tools.is_empty() {
            let open = self.agent.show_tools;
            let mut log = div().flex().flex_col().gap_1().child(
                div()
                    .id("agent-toggle-log")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .py_1()
                    .text_xs()
                    .text_color(colors.muted)
                    .cursor_pointer()
                    .hover(|s| s.text_color(colors.foreground))
                    .child(icon(if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    }))
                    .child(icon("wrench"))
                    .child(format!("调用了 {} 次工具", session.tools.len()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agent.show_tools = !this.agent.show_tools;
                        cx.notify();
                    })),
            );
            if open {
                for (name, result) in &session.tools {
                    log = log.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .ml_2()
                            .pl_3()
                            .border_l_2()
                            .border_color(colors.border)
                            .text_xs()
                            .child(div().font_weight(FontWeight::MEDIUM).child(name.clone()))
                            .child(div().text_color(colors.muted).child(excerpt(result, 240))),
                    );
                }
            }
            panel = panel.child(log);
        }
        if !session.sources.is_empty() {
            let mut chips = div().flex().flex_wrap().gap_1p5();
            for (i, source) in session.sources.iter().enumerate() {
                let name = source
                    .path
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| source.path.display().to_string());
                let tip = format!(
                    "{}{}",
                    source.path.display(),
                    if source.unsaved {
                        "（编辑器快照）"
                    } else {
                        ""
                    }
                );
                let source = source.clone();
                chips = chips.child(
                    Button::new(("agent-source", i))
                        .outline()
                        .xsmall()
                        .icon(icon("file-text"))
                        .label(format!("{} · {name}", source.id))
                        .tooltip(tip)
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.agent_source(source.clone(), w, cx)
                        })),
                );
            }
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.muted)
                            .child(format!("引用来源 · {}", session.sources.len())),
                    )
                    .child(chips),
            );
        }
        for (i, proposal) in session.proposals.iter().enumerate() {
            panel = panel.child(self.agent_proposal(i, proposal, colors, cx));
        }
        if !self.agent.error.is_empty() {
            let ok = self.agent.error == PROBE_OK;
            panel = panel.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .rounded(px(10.))
                    .bg(colors.surface)
                    .border_1()
                    .border_color(colors.border)
                    .child(
                        div().pt_1p5().child(
                            icon(if ok { "circle-check" } else { "circle-alert" })
                                .text_color(colors.accent),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py_1()
                            .child(self.agent.error.clone()),
                    )
                    .child(
                        Button::new("agent-dismiss-error")
                            .ghost()
                            .xsmall()
                            .icon(icon("x"))
                            .tooltip("关闭")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.agent.error.clear();
                                cx.notify();
                            })),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .text_sm()
            .whitespace_normal()
            .text_color(colors.foreground)
            .bg(colors.background)
            .child(self.agent_header(colors, cx))
            .child(panel)
            .child(self.agent_composer(colors, cx))
            .into_any_element()
    }
}
const DIFF_LINES: usize = 400;
fn pill(text: impl Into<SharedString>, fg: Rgba, bg: Rgba) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded_full()
        .text_xs()
        .text_color(fg)
        .bg(bg)
        .child(text.into())
}
/// Reads token counts from either Responses or Chat Completions usage objects.
fn usage_stats(usage: &serde_json::Value) -> Vec<(&'static str, u64)> {
    let field = |paths: &[&str]| {
        paths
            .iter()
            .find_map(|path| usage.pointer(path).and_then(serde_json::Value::as_u64))
    };
    let input = field(&["/input_tokens", "/prompt_tokens"]);
    let output = field(&["/output_tokens", "/completion_tokens"]);
    [
        ("输入", input),
        (
            "缓存",
            field(&[
                "/input_tokens_details/cached_tokens",
                "/prompt_tokens_details/cached_tokens",
            ]),
        ),
        ("输出", output),
        (
            "推理",
            field(&[
                "/output_tokens_details/reasoning_tokens",
                "/completion_tokens_details/reasoning_tokens",
            ]),
        ),
        (
            "合计",
            field(&["/total_tokens"]).or(input.zip(output).map(|(a, b)| a + b)),
        ),
    ]
    .into_iter()
    .filter_map(|(label, n)| n.filter(|&n| n > 0 || label == "输出").map(|n| (label, n)))
    .collect()
}
fn compact_tokens(n: u64) -> String {
    let scaled = |n: u64, unit: f64, suffix: &str| {
        let value = n as f64 / unit;
        if value < 10. {
            format!("{:.1}{suffix}", value).replace(".0", "")
        } else {
            format!("{:.0}{suffix}", value)
        }
    };
    match n {
        0..1_000 => n.to_string(),
        1_000..999_500 => scaled(n, 1e3, "K"),
        _ => scaled(n, 1e6, "M"),
    }
}
fn excerpt(text: &str, limit: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.into(),
    }
}
fn state_label(state: &RunState) -> &'static str {
    match state {
        RunState::Preparing => "准备中",
        RunState::Requesting => "正在生成",
        RunState::Reading => "读取资料",
        RunState::AwaitingReview => "等待审阅",
        RunState::Applying => "正在应用",
        RunState::Completed => "已完成",
        RunState::Failed => "失败或部分失败",
        RunState::Cancelled => "已停止",
        RunState::Interrupted => "上次任务中断",
        RunState::Limited => "已达执行上限，可继续",
    }
}
fn report_text(report: &ApplyReport) -> String {
    report
        .files
        .iter()
        .map(|(path, status)| {
            format!(
                "{}：{}",
                path.display(),
                match status {
                    ApplyStatus::Written { .. } => "已写入".into(),
                    ApplyStatus::Failed(error) => error.clone(),
                    ApplyStatus::NotExecuted => "未执行".into(),
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use inkstone_core::changes::{Change, ChangeSet};
    fn proposal(before: &str, after: &str) -> Proposal {
        Proposal {
            previews: vec![],
            changes: ChangeSet {
                changes: vec![Change {
                    path: "a.md".into(),
                    before: Some(before.into()),
                    after: after.into(),
                    reason: "润色".into(),
                }],
                sources: vec![],
            },
            status: "等待审阅".into(),
            report: None,
        }
    }
    #[gpui::test]
    fn agent_review_preserves_new_edits_and_applies_through_editor(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("notes");
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(&root, dir.path().join("history")).unwrap();
        vault.create("a.md".as_ref(), "old").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.vault = Some(vault.clone());
                w.poll_agent(window, cx);
                w.add_tab("a.md".into(), Some("old".into()), false, window, cx);
                w.agent.session_mut().proposals.push(proposal("old", "new"));
                let editor = w.tabs[0].save.editor.clone();
                editor.update(cx, |s, cx| s.replace_all("user typing", window, cx));
                w.document_changed(editor.clone(), window, cx);
                w.agent_apply(0, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "user typing");
                assert_eq!(w.agent.session().proposals[0].status, "等待审阅");
                w.agent
                    .session_mut()
                    .proposals
                    .push(proposal("user typing", "reviewed"));
                w.agent_apply(1, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "reviewed");
                assert_eq!(vault.read("a.md".as_ref()).unwrap().as_deref(), Some("old"));
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), "user typing");
                // Duplicate acceptance is inert, even after additional edits.
                editor.update(cx, |s, cx| s.replace_all("new user edit", window, cx));
                w.agent_apply(1, window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "new user edit");
            })
            .unwrap();
    }
    #[gpui::test]
    fn agent_uses_focused_split_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.add_tab("a.md".into(), Some("中文选区".into()), false, window, cx);
                w.split_active(false, window, cx);
                w.focus_secondary(cx);
                let editor = w.views.split.as_ref().unwrap().pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_selected_range(0..6, cx));
                let (_, text, range) = w.agent_snapshot(window, cx).unwrap().unwrap();
                assert_eq!(&text[range], "中文");
            })
            .unwrap();
    }
    #[gpui::test]
    fn agent_panel_renders_and_commands_keep_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.add_tab("a.md".into(), Some("中文选区".into()), false, window, cx);
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.set_selected_range(0..6, cx));
                w.execute_command(113, window, cx);
                assert_eq!(w.ui.right_mode, 5);
                assert_eq!(w.tabs[0].save.editor.read(cx).selected_range(), 0..6);
                assert!(w.agent.prompt.read(cx).value().contains("润色"));
                w.open_agent_settings(window, cx);
                assert!(w.ui.settings);
                assert_eq!(w.ui.settings_tab, 10);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }
    #[gpui::test]
    fn agent_proposal_actions_fit_inside_card_with_long_conversation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.add_tab("a.md".into(), Some("old\n".into()), false, window, cx);
                w.open_agent(0, window, cx);
                let session = w.agent.session_mut();
                session.messages.push(Message {
                    role: "assistant".into(),
                    text: "这是较长的修改说明。\n\n".repeat(50),
                });
                let mut pending = proposal("old\n", &"new\n".repeat(100));
                pending.previews = pending.changes.previews();
                session.proposals.push(pending);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(600.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        handle
            .update(&mut visual, |w, _, cx| {
                w.agent.scroll.scroll_to_bottom();
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let card = visual.debug_bounds("agent-proposal-0").unwrap();
        let actions = visual.debug_bounds("agent-proposal-actions-0").unwrap();
        assert!(
            actions.top() >= card.top() && actions.bottom() <= card.bottom(),
            "proposal actions {actions:?} clipped by card {card:?}"
        );
    }
    #[gpui::test]
    fn agent_composer_sends_on_enter_and_keeps_shift_enter_newline(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("notes");
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(&root, dir.path().join("history")).unwrap();
        let handle = cx.add_window(Workspace::new);
        let prompt = handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.vault = Some(vault);
                w.poll_agent(window, cx);
                w.open_agent(0, window, cx);
                // An invalid model stops the send before any request is made.
                w.agent.settings.models[0].model.clear();
                w.agent.prompt.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus(&prompt.focus_handle(cx), cx);
        });
        visual.simulate_input("问题");
        visual.simulate_keystrokes("shift-enter");
        visual.simulate_input("补充");
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        prompt.read_with(&visual, |s, _| assert_eq!(s.value(), "问题\n补充"));
        handle
            .update(&mut visual, |w, _, _| {
                assert_eq!(w.agent.error, "请填写模型名称")
            })
            .unwrap();
    }
    #[gpui::test]
    fn agent_panel_renders_conversation_and_reuses_empty_session(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.startup_pending = false;
                w.open_agent(0, window, cx);
                w.agent_new_session(window, cx);
                assert_eq!(w.agent.sessions.len(), 1);
                let session = w.agent.session_mut();
                session.title = "整理笔记".into();
                session.messages.push(Message {
                    role: "user".into(),
                    text: "总结".into(),
                });
                session.messages.push(Message {
                    role: "assistant".into(),
                    text: "## 要点\n- 一 [1]".into(),
                });
                session.tools.push(("read".into(), "x".repeat(1000)));
                session.sources.push(Source {
                    id: "1".into(),
                    path: "a.md".into(),
                    hash: String::new(),
                    range: 0..1,
                    unsaved: true,
                });
                let mut pending = proposal("old\n", "new\n");
                pending.previews = pending.changes.previews();
                session.proposals.push(pending);
                session.usage =
                    Some(serde_json::json!({"input_tokens": 1_200, "output_tokens": 80}));
                w.agent.show_tools = true;
                w.agent.error = PROBE_OK.into();
                w.agent.scope = 1;
                w.agent_new_session(window, cx);
                assert_eq!((w.agent.sessions.len(), w.agent.selected), (2, 1));
                w.agent_select_session(0, cx);
                assert_eq!(w.agent.session().title, "整理笔记");
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        handle
            .update(&mut visual, |w, _, cx| {
                w.agent_delete_session(cx);
                assert_eq!(w.agent.sessions.len(), 1);
                assert!(w.agent.session().messages.is_empty());
            })
            .unwrap();
    }
    #[test]
    fn usage_reads_both_api_shapes_compactly() {
        let responses = serde_json::json!({
            "input_tokens": 12_345,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": 980,
            "output_tokens_details": {"reasoning_tokens": 512},
            "total_tokens": 13_325
        });
        assert_eq!(
            usage_stats(&responses),
            [
                ("输入", 12_345),
                ("输出", 980),
                ("推理", 512),
                ("合计", 13_325)
            ]
        );
        let chat = serde_json::json!({"prompt_tokens": 2_000, "completion_tokens": 0});
        assert_eq!(
            usage_stats(&chat),
            [("输入", 2_000), ("输出", 0), ("合计", 2_000)]
        );
        assert!(usage_stats(&serde_json::json!({"output_tokens": null})).is_empty());
        let labels: Vec<_> = [999, 1_000, 1_250, 12_345, 999_499, 999_500, 1_050_000]
            .map(compact_tokens)
            .into();
        assert_eq!(labels, ["999", "1K", "1.2K", "12K", "999K", "1M", "1.1M"]);
    }
    #[test]
    fn excerpt_truncates_on_char_boundaries() {
        assert_eq!(excerpt("  中文内容  ", 2), "中文…");
        assert_eq!(excerpt("短", 4), "短");
    }
}
