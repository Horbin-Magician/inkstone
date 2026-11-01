//! Local app-data persistence. Credentials never enter conversations or the vault.
use crate::{provider::ModelConfig, runtime::RunState, tools::Source};
use anyhow::{Context, Result, bail};
use inkstone_core::changes::{ApplyReport, ChangeSet};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, path::Path};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    pub models: Vec<ModelConfig>,
    pub selected: usize,
    pub excluded: Vec<std::path::PathBuf>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(default)]
    pub previews: Vec<String>,
    pub changes: ChangeSet,
    pub status: String,
    pub report: Option<ApplyReport>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub state: RunState,
    pub messages: Vec<Message>,
    pub sources: Vec<Source>,
    pub proposals: Vec<Proposal>,
    pub tools: Vec<(String, String)>,
    pub usage: Option<serde_json::Value>,
}
impl Default for Session {
    fn default() -> Self {
        let id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Self {
            id,
            title: "新会话".into(),
            state: RunState::Completed,
            messages: vec![],
            sources: vec![],
            proposals: vec![],
            tools: vec![],
            usage: None,
        }
    }
}
pub fn read<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => {
            Ok(serde_json::from_slice(&bytes).context("Agent 配置或会话损坏，未覆盖原文件")?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn write<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("无效保存路径")?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
pub fn load_sessions(dir: &Path, vault: &Path) -> Result<Vec<Session>> {
    let mut sessions: Vec<Session> = read(&sessions_path(dir, vault))?;
    for session in &mut sessions {
        if matches!(
            session.state,
            RunState::Preparing | RunState::Requesting | RunState::Reading | RunState::Applying
        ) {
            session.state = RunState::Interrupted;
        }
        for proposal in &mut session.proposals {
            if proposal.status == "正在应用" {
                proposal.status = "应用中断；请查看文件及版本历史，不可直接重放".into();
            }
        }
    }
    Ok(sessions)
}
pub fn sessions_path(dir: &Path, vault: &Path) -> std::path::PathBuf {
    dir.join("agent").join(format!(
        "{}.json",
        crate::tools::hash(&vault.to_string_lossy())
    ))
}
pub fn save_sessions(dir: &Path, vault: &Path, sessions: &[Session]) -> Result<()> {
    write(&sessions_path(dir, vault), sessions)
}
pub fn load_secrets(dir: &Path) -> Result<BTreeMap<String, String>> {
    read(&dir.join("agent-secrets.json"))
}
pub fn save_secrets(dir: &Path, secrets: &BTreeMap<String, String>) -> Result<()> {
    write(&dir.join("agent-secrets.json"), secrets)
}
pub fn check_exclusions(paths: &[std::path::PathBuf]) -> Result<()> {
    if paths.iter().any(|p| {
        p.is_absolute()
            || p.components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
    }) {
        bail!("排除目录须为库内相对路径");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_coalesces_without_losing_other_vaults_and_flushes_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        {
            let writer = SessionWriter::default();
            for i in 0..10 {
                writer.queue(
                    dir.path(),
                    Path::new("a"),
                    vec![Session {
                        title: i.to_string(),
                        ..Default::default()
                    }],
                );
            }
            writer.queue(dir.path(), Path::new("b"), vec![Session::default()]);
        }
        assert_eq!(
            load_sessions(dir.path(), Path::new("a")).unwrap()[0].title,
            "9"
        );
        assert_eq!(load_sessions(dir.path(), Path::new("b")).unwrap().len(), 1);
    }
    #[test]
    fn sessions_isolated_and_interrupted_without_replay() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session {
            state: RunState::Applying,
            ..Default::default()
        };
        save_sessions(dir.path(), Path::new("a"), &[session]).unwrap();
        assert_eq!(
            load_sessions(dir.path(), Path::new("a")).unwrap()[0].state,
            RunState::Interrupted
        );
        assert!(
            load_sessions(dir.path(), Path::new("b"))
                .unwrap()
                .is_empty()
        );
    }
}

/// Serializes and writes on one worker. Pending snapshots coalesce per vault,
/// so switching vaults cannot discard the previous vault's last checkpoint.
pub struct SessionWriter {
    shared: std::sync::Arc<(std::sync::Mutex<PendingWrites>, std::sync::Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub errors: std::sync::mpsc::Receiver<String>,
}
#[derive(Default)]
struct PendingWrites {
    snapshots: BTreeMap<std::path::PathBuf, Vec<Session>>,
    stopping: bool,
    writing: bool,
    error: Option<String>,
}
impl Default for SessionWriter {
    fn default() -> Self {
        let shared = std::sync::Arc::new((
            std::sync::Mutex::new(PendingWrites::default()),
            std::sync::Condvar::new(),
        ));
        let worker = shared.clone();
        let (tx, errors) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            loop {
                let (lock, signal) = &*worker;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                while state.snapshots.is_empty() && !state.stopping {
                    state = signal.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                if state.snapshots.is_empty() && state.stopping {
                    break;
                }
                let snapshots = std::mem::take(&mut state.snapshots);
                state.writing = true;
                drop(state);
                let mut error = None;
                for (path, sessions) in snapshots {
                    if let Err(e) = write(&path, &sessions) {
                        let text = format!("会话保存失败：{e}");
                        let _ = tx.send(text.clone());
                        error = Some(text);
                    }
                }
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                state.writing = false;
                state.error = error;
                signal.notify_all();
            }
        });
        Self {
            shared,
            thread: Some(thread),
            errors,
        }
    }
}
impl SessionWriter {
    pub fn queue(&self, dir: &Path, vault: &Path, sessions: Vec<Session>) {
        let (lock, signal) = &*self.shared;
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshots
            .insert(sessions_path(dir, vault), sessions);
        signal.notify_one();
    }
    /// Barrier used before starting writes, at vault switches and on shutdown.
    pub fn flush(&self) -> Result<()> {
        let (lock, signal) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        while state.writing || !state.snapshots.is_empty() {
            state = signal.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if let Some(error) = &state.error {
            bail!("{error}");
        }
        Ok(())
    }
}
impl Drop for SessionWriter {
    fn drop(&mut self) {
        let (lock, signal) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).stopping = true;
        signal.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
