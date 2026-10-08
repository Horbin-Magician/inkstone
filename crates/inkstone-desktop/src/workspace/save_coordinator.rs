//! Background save transaction. No UI entities or view state belong here.
use inkstone_core::vault::{SaveReceipt, Vault, VaultError, drafts::DraftSession};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub(super) struct DraftIo {
    pub session: Mutex<DraftSession>,
    pub revision: AtomicU64,
}

pub(super) struct SaveJob {
    vault: Vault,
    path: PathBuf,
    baseline: Option<String>,
    text: String,
    draft: Option<Arc<DraftIo>>,
}

pub(super) struct SaveOutcome {
    pub result: Result<SaveReceipt, VaultError>,
    /// Cleanup failure does not undo a successful authoritative save.
    pub cleanup_error: Option<VaultError>,
}

impl SaveJob {
    /// Construct before scheduling the worker, invalidating already queued draft
    /// writes immediately. The shared mutex serializes any write already running.
    pub fn new(
        vault: Vault,
        path: PathBuf,
        baseline: Option<String>,
        text: String,
        draft: Option<Arc<DraftIo>>,
    ) -> Self {
        if let Some(io) = &draft {
            io.revision.fetch_add(1, Ordering::SeqCst);
        }
        Self {
            vault,
            path,
            baseline,
            text,
            draft,
        }
    }
    pub fn run(self) -> SaveOutcome {
        let _span =
            inkstone_core::performance::span(inkstone_core::performance::Stage::SaveTransaction);
        let mut draft = self.draft.as_ref().map(|io| io.session.lock().unwrap());
        let result = if self.baseline.is_none() {
            self.vault.create(&self.path, &self.text)
        } else {
            self.vault
                .save(&self.path, self.baseline.as_deref(), &self.text)
        };
        let cleanup_error = if result.is_ok() {
            draft.as_mut().and_then(|draft| draft.clear().err())
        } else {
            None
        };
        SaveOutcome {
            result,
            cleanup_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_retains_recovery_then_success_clears_only_owned_draft() {
        let root = std::env::temp_dir().join(format!(
            "inkstone-save-job-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        vault
            .create(std::path::Path::new("note.md"), "original")
            .unwrap();
        let mut session = DraftSession::new(vault.clone(), "note.md".into()).unwrap();
        session.persist(Some("original"), "local draft").unwrap();
        let owned = vault
            .recoveries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.record.draft == "local draft")
            .unwrap()
            .journal;
        let mut other = DraftSession::new(vault.clone(), "other.md".into()).unwrap();
        other.persist(None, "unrelated draft").unwrap();
        let unrelated = vault
            .recoveries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.record.draft == "unrelated draft")
            .unwrap()
            .journal;
        let draft = Arc::new(DraftIo {
            session: Mutex::new(session),
            revision: AtomicU64::new(0),
        });
        std::fs::write(vault.root.join("note.md"), "external").unwrap();
        let job = SaveJob::new(
            vault.clone(),
            "note.md".into(),
            Some("original".into()),
            "local draft".into(),
            Some(draft.clone()),
        );
        assert_eq!(
            draft.revision.load(Ordering::SeqCst),
            1,
            "invalidate before worker runs"
        );
        let outcome = job.run();
        assert!(matches!(outcome.result, Err(VaultError::Conflict { .. })));
        assert!(outcome.cleanup_error.is_none());
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "external"
        );
        assert!(
            vault
                .recoveries()
                .unwrap()
                .iter()
                .any(|e| e.record.draft == "local draft")
        );
        let outcome = SaveJob::new(
            vault.clone(),
            "copy.md".into(),
            None,
            "local draft".into(),
            Some(draft),
        )
        .run();
        assert!(outcome.result.is_ok());
        assert!(outcome.cleanup_error.is_none());
        assert!(!owned.exists());
        assert!(unrelated.exists());
        assert_eq!(
            std::fs::read_to_string(vault.root.join("copy.md")).unwrap(),
            "local draft"
        );
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "external"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
