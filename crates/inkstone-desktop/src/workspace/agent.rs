//! Workspace adapter: the agent can stage proposals, only this layer applies them.
use super::*;
use gpui_component::{Disableable, button::Button};
use inkstone_agent::{
    AgentEvent, AgentRun, RunState, Scope,
    provider::ModelConfig,
    runtime::{Cancel, RequestMode, RunRequest},
    storage::{self, Message, Proposal, Session, Settings},
    tools::{Source, hash},
};
use inkstone_core::changes::{ApplyReport, ApplyStatus};
use std::collections::BTreeMap;

pub(super) struct State {
    settings: Settings,
    secrets: BTreeMap<String, String>,
    fields: Vec<Entity<InputState>>,
    prompt: Entity<InputState>,
    scope_input: Entity<InputState>,
    scope: usize,
    mode: RequestMode,
    scope_key: String,
    sessions: Vec<Session>,
    selected: usize,
    root: Option<PathBuf>,
    run: Option<AgentRun>,
    stream_task: Option<Task<()>>,
    sequence: u64,
    applying: Option<Cancel>,
    pub(super) show_settings: bool,
    show_tools: bool,
    error: String,
    persistence_error: bool,
    writer: storage::SessionWriter,
    last_checkpoint: std::time::Instant,
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
        let mut state = Self {
            settings,
            secrets: secrets.unwrap_or_default(),
            fields,
            prompt: cx
                .new(|cx| InputState::new(window, cx).placeholder("向 Agent 提问或描述修改任务")),
            scope_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("库内目录 / 笔记路径，逗号分隔")),
            scope: 0,
            mode: RequestMode::Chat,
            scope_key: String::new(),
            sessions: vec![Session::default()],
            selected: 0,
            root: None,
            run: None,
            stream_task: None,
            sequence: 0,
            applying: None,
            show_settings: false,
            show_tools: false,
            writer: Default::default(),
            last_checkpoint: std::time::Instant::now(),
            persistence_error: false,
            error,
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
        self.agent.mode = if command == 113 {
            RequestMode::WriteCurrent
        } else {
            RequestMode::Chat
        };
        let prompt = match command {
            113 => "请润色当前选区或笔记，保留原意，提出修改供我审阅。",
            114 => "请总结当前选区或笔记，列出要点并引用来源。",
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
        if self.agent.root != root {
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
                AgentEvent::State(state) => self.agent.session_mut().state = state,
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
                AgentEvent::Probe { model_id, tools } => {
                    if let Some(model) = self
                        .agent
                        .settings
                        .models
                        .iter_mut()
                        .find(|m| m.id == model_id)
                    {
                        model.tools = tools;
                    }
                    self.agent.error = if tools {
                        "流式连接与工具参数测试通过"
                    } else {
                        "流式连接通过；未返回有效工具调用，已切换基础问答与写作"
                    }
                    .into();
                    if let Err(error) =
                        storage::write(&app_dir().join("agent-settings.json"), &self.agent.settings)
                    {
                        self.agent.error = error.to_string();
                    }
                }
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
            tools: old.tools,
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
                self.agent.show_settings = false;
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
            self.agent.show_settings = true;
            cx.notify();
            return;
        }
        self.flush_document_views(window, cx);
        let current = self.active.and_then(|i| self.tabs.get(i));
        let mut snapshot = None;
        if let Some(tab) = current {
            if self.has_pending_input(tab.id, window, cx) {
                self.agent.error = "请先完成中文组词再发送".into();
                cx.notify();
                return;
            }
            if tab.image.is_none() {
                let editor = tab.save.editor.read(cx);
                let text = editor.value().to_string();
                let selection = editor.selected_range();
                let range = if self.agent.scope == 0 && !selection.is_empty() {
                    selection
                } else {
                    0..text.len()
                };
                snapshot = Some((tab.path.clone(), text, range));
            }
        }
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
        self.agent.persist();
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
        self.agent.prompt.update(cx, |s, cx| s.set_value("连接测试：请回复‘连接成功’，如果提供工具，请调用 list_notes 一次，然后报告工具是否成功。不要修改笔记。", window, cx));
        self.agent.mode = RequestMode::Probe;
        self.agent_send(window, cx);
        self.agent.mode = RequestMode::Chat;
    }
    pub(super) fn agent_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.agent.busy();
        let mut panel = div()
            .id("agent-panel")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .text_sm();
        panel = panel.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .child(
                    Button::new("agent-settings")
                        .label("模型设置")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.agent.show_settings = !this.agent.show_settings;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("agent-new")
                        .label("新会话")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.agent.sessions.push(Session::default());
                            this.agent.selected = this.agent.sessions.len() - 1;
                            this.agent.scope_key.clear();
                            this.agent.persist();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("agent-stop")
                        .label("停止")
                        .disabled(!busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(run) = &this.agent.run {
                                run.cancel.cancel();
                            }
                            if let Some(cancel) = &this.agent.applying {
                                cancel.cancel();
                            }
                            cx.notify();
                        })),
                ),
        );
        if self.agent.show_settings {
            for (i, title) in [
                "名称",
                "服务地址",
                "模型",
                "API Key（本地明文）",
                "上下文 token",
                "最大输出 token",
                "排除目录（逗号分隔）",
            ]
            .iter()
            .enumerate()
            {
                panel = panel
                    .child((*title).to_owned())
                    .child(Input::new(&self.agent.fields[i]));
            }
            let tools = self.agent.settings.models[self.agent.settings.selected].tools;
            panel = panel
                .child(
                    Button::new("agent-tools-toggle")
                        .label(if tools {
                            "工具调用：启用"
                        } else {
                            "工具调用：关闭（基础问答）"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            let i = this.agent.settings.selected;
                            this.agent.settings.models[i].tools =
                                !this.agent.settings.models[i].tools;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(Button::new("agent-save-config").label("保存配置").on_click(
                            cx.listener(|this, _, w, cx| {
                                this.agent_save_settings(w, cx);
                            }),
                        ))
                        .child(
                            Button::new("agent-test-config")
                                .label("测试连接与工具")
                                .on_click(
                                    cx.listener(|this, _, w, cx| this.agent_test_model(w, cx)),
                                ),
                        )
                        .child(Button::new("agent-add-config").label("新增配置").on_click(
                            cx.listener(|this, _, w, cx| {
                                let config = ModelConfig {
                                    id: Session::default().id,
                                    ..Default::default()
                                };
                                this.agent.settings.models.push(config);
                                this.agent.settings.selected = this.agent.settings.models.len() - 1;
                                this.agent.fill_fields(w, cx);
                                cx.notify();
                            }),
                        )),
                );
        }
        let model_label = format!(
            "模型：{}（点击切换）",
            self.agent.settings.models[self.agent.settings.selected].name
        );
        panel = panel.child(
            Button::new("agent-model")
                .label(model_label)
                .disabled(busy)
                .on_click(cx.listener(|this, _, w, cx| {
                    this.agent.settings.selected =
                        (this.agent.settings.selected + 1) % this.agent.settings.models.len();
                    this.agent.fill_fields(w, cx);
                    cx.notify();
                })),
        );
        let scopes = ["当前笔记 / 选区", "指定文件夹", "指定笔记", "整个笔记库"];
        panel = panel.child(
            Button::new("agent-scope")
                .label(format!("范围：{}", scopes[self.agent.scope]))
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent.scope = (this.agent.scope + 1) % 4;
                    cx.notify();
                })),
        );
        if matches!(self.agent.scope, 1 | 2) {
            panel = panel.child(Input::new(&self.agent.scope_input));
        }
        panel = panel.child(
            Button::new("agent-writing-mode")
                .label(if self.agent.mode == RequestMode::WriteCurrent {
                    "写作模式：生成可审阅修改"
                } else {
                    "问答 / 工具模式"
                })
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent.mode = if this.agent.mode == RequestMode::WriteCurrent {
                        RequestMode::Chat
                    } else {
                        RequestMode::WriteCurrent
                    };
                    cx.notify();
                })),
        );
        panel = panel.child("只发送所选范围内的必要内容至配置的模型服务。所有修改先审阅。");
        for (i, session) in self.agent.sessions.iter().enumerate() {
            panel = panel.child(
                Button::new(("agent-session", i))
                    .label(format!(
                        "{}{}",
                        if i == self.agent.selected { "• " } else { "" },
                        session.title
                    ))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.agent.selected = i;
                        this.agent.scope_key.clear();
                        cx.notify();
                    })),
            );
        }
        panel = panel.child(
            Button::new("agent-delete-session")
                .label("删除当前会话")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent.sessions.remove(this.agent.selected);
                    if this.agent.sessions.is_empty() {
                        this.agent.sessions.push(Session::default());
                    }
                    this.agent.selected = 0;
                    this.agent.scope_key.clear();
                    this.agent.persist();
                    cx.notify();
                })),
        );
        let session = self.agent.session();
        panel = panel.child(format!("状态：{}", state_label(&session.state)));
        if !self.agent.error.is_empty() {
            panel = panel.child(div().child(self.agent.error.clone()));
        }
        for message in &session.messages {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .child(if message.role == "user" {
                        "你"
                    } else {
                        "Agent"
                    })
                    .child(message.text.clone()),
            );
        }
        panel = panel.child(format!(
            "本轮用量：{}",
            session
                .usage
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "未知".into())
        ));
        panel = panel.child(
            Button::new("agent-toggle-log")
                .label(format!("工具记录（{}）", session.tools.len()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.agent.show_tools = !this.agent.show_tools;
                    cx.notify();
                })),
        );
        if self.agent.show_tools {
            for (name, result) in &session.tools {
                panel = panel.child(format!("{name}\n{}", recovery::preview_text(result)));
            }
        }
        for (i, source) in session.sources.iter().enumerate() {
            let source = source.clone();
            panel = panel.child(
                Button::new(("agent-source", i))
                    .label(format!(
                        "[{}] {}{}",
                        source.id,
                        source.path.display(),
                        if source.unsaved {
                            "（编辑器快照）"
                        } else {
                            ""
                        }
                    ))
                    .on_click(
                        cx.listener(move |this, _, w, cx| this.agent_source(source.clone(), w, cx)),
                    ),
            );
        }
        for (i, proposal) in session.proposals.iter().enumerate() {
            let mut review = div()
                .flex()
                .flex_col()
                .gap_2()
                .border_1()
                .p_2()
                .child(format!("修改组 {}：{}", i + 1, proposal.status));
            if proposal.changes.changes.len() > 1 {
                review = review.child("关联修改须整组接受，避免缺失链接目标。");
            }
            for (j, change) in proposal.changes.changes.iter().enumerate() {
                let diff = proposal
                    .previews
                    .get(j)
                    .cloned()
                    .unwrap_or_else(|| "旧会话未保存差异预览，请重新生成提案。".into());
                review = review
                    .child(format!(
                        "{}{}\n{}",
                        change.path.display(),
                        if change.before.is_none() {
                            "（新建）"
                        } else {
                            ""
                        },
                        change.reason
                    ))
                    .child(recovery::preview_text(&diff));
                let path = change.path.clone();
                review = review.child(
                    Button::new(("agent-history", i * 100 + j))
                        .label("查看版本历史 / 恢复")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.open_history_path(path.clone(), w, cx)
                        })),
                );
            }
            if let Some(report) = &proposal.report {
                review = review.child(report_text(report));
            }
            review = review.child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(("agent-accept", i))
                            .label("接受并应用")
                            .disabled(busy || proposal.status != "等待审阅")
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.agent_apply(i, w, cx)),
                            ),
                    )
                    .child(
                        Button::new(("agent-reject", i))
                            .label("拒绝")
                            .disabled(busy || proposal.status != "等待审阅")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.agent.session_mut().proposals[i].status = "已拒绝".into();
                                this.agent.persist();
                                cx.notify();
                            })),
                    ),
            );
            panel = panel.child(review);
        }
        panel
            .child(Input::new(&self.agent.prompt))
            .child(
                Button::new("agent-send")
                    .label("发送 / 继续")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| this.agent_send(w, cx))),
            )
            .into_any_element()
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
                w.agent.show_settings = true;
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
    }
}
