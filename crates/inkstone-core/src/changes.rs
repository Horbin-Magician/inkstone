//! Reviewed, conditional changes. No model output is an authorization to write.
use crate::vault::Vault;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub path: PathBuf,
    /// None means create exclusively; Some is the exact reviewed baseline.
    pub before: Option<String>,
    pub after: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ApplyStatus {
    Written { recovery: PathBuf },
    Failed(String),
    NotExecuted,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub files: Vec<(PathBuf, ApplyStatus)>,
}
impl ChangeSet {
    /// Bounded display-only previews; application always uses the full reviewed text.
    pub fn previews(&self) -> Vec<String> {
        self.changes
            .iter()
            .map(|change| {
                let diff = similar::TextDiff::configure()
                    .timeout(std::time::Duration::from_millis(50))
                    .diff_lines(change.before.as_deref().unwrap_or_default(), &change.after)
                    .unified_diff()
                    .context_radius(2)
                    .header("修改前", "修改后")
                    .to_string();
                let mut end = diff.len().min(128 * 1024);
                while !diff.is_char_boundary(end) {
                    end -= 1;
                }
                if end < diff.len() {
                    format!("{}\n……差异预览已截断，完整修改仍保留。", &diff[..end])
                } else {
                    diff
                }
            })
            .collect()
    }

    pub fn validate(&self, vault: &Vault) -> Result<(), String> {
        if self.changes.is_empty() || self.changes.len() > 100 {
            return Err("修改方案须包含 1–100 篇笔记".into());
        }
        let mut paths = BTreeSet::new();
        let mut bytes = 0usize;
        for change in &self.changes {
            vault.path(&change.path).map_err(|e| e.to_string())?;
            let key = change.path.to_string_lossy().to_lowercase();
            if !paths.insert(key) {
                return Err("同一方案不能重复修改同一路径".into());
            }
            bytes = bytes.saturating_add(change.after.len());
            if bytes > 4 * 1024 * 1024 {
                return Err("修改内容超过 4 MiB，请拆分任务".into());
            }
            let current = vault.read(&change.path).map_err(|e| e.to_string())?;
            if current != change.before {
                return Err(format!("{} 已变化，请重新生成预览", change.path.display()));
            }
        }
        Ok(())
    }
    /// Checks the complete set before starting, then checks each baseline again at
    /// the Vault write boundary. Stops on first failure; never rolls back new edits.
    pub fn apply(&self, vault: &Vault, cancelled: impl Fn() -> bool) -> ApplyReport {
        let mut report = ApplyReport {
            files: self
                .changes
                .iter()
                .map(|c| (c.path.clone(), ApplyStatus::NotExecuted))
                .collect(),
        };
        if let Err(error) = self.validate(vault) {
            if let Some(first) = report.files.first_mut() {
                first.1 = ApplyStatus::Failed(error);
            }
            return report;
        }
        for (change, (_, status)) in self.changes.iter().zip(&mut report.files) {
            if cancelled() {
                break;
            }
            let result = match &change.before {
                Some(before) => vault.save(&change.path, Some(before), &change.after),
                None => vault.create(&change.path, &change.after),
            };
            match result {
                Ok(receipt) => {
                    *status = ApplyStatus::Written {
                        recovery: receipt.recovery,
                    }
                }
                Err(error) => {
                    *status = ApplyStatus::Failed(error.to_string());
                    break;
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("notes")).unwrap();
        let vault = Vault::open(dir.path().join("notes"), dir.path().join("history")).unwrap();
        (dir, vault)
    }
    fn create(path: &str) -> Change {
        Change {
            path: path.into(),
            before: None,
            after: "新内容😀".into(),
            reason: "test".into(),
        }
    }
    #[test]
    fn all_baselines_checked_before_any_write() {
        let (_dir, vault) = fixture();
        vault.create("b.md".as_ref(), "existing").unwrap();
        let set = ChangeSet {
            changes: vec![create("a.md"), create("b.md")],
            ..Default::default()
        };
        let report = set.apply(&vault, || false);
        assert!(matches!(report.files[0].1, ApplyStatus::Failed(_)));
        assert_eq!(vault.read("a.md".as_ref()).unwrap(), None);
        assert_eq!(
            vault.read("b.md".as_ref()).unwrap().as_deref(),
            Some("existing")
        );
    }
    #[test]
    fn cancellation_preserves_completed_write_and_replay_is_rejected() {
        let (_dir, vault) = fixture();
        let set = ChangeSet {
            changes: vec![create("a.md"), create("b.md")],
            ..Default::default()
        };
        let report = set.apply(&vault, || vault.read("a.md".as_ref()).unwrap().is_some());
        assert!(matches!(report.files[0].1, ApplyStatus::Written { .. }));
        assert!(matches!(report.files[1].1, ApplyStatus::NotExecuted));
        assert!(set.validate(&vault).is_err());
    }
    #[test]
    fn rejects_paths_and_case_collisions() {
        let (_dir, vault) = fixture();
        for changes in [
            vec![create("../escape.md")],
            vec![create("a.md"), create("A.md")],
        ] {
            assert!(
                ChangeSet {
                    changes,
                    ..Default::default()
                }
                .validate(&vault)
                .is_err()
            );
        }
    }
}
