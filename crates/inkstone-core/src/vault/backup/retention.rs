//! Advisory retention preview. Never authorizes deletion from cached metadata.
use super::{capacity::*, *};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Older than the retained set; still requires a fresh, locked integrity check.
    Candidate,
    Recent,
    InUse,
    Protected,
    UnknownSource,
    IncompleteInventory,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub backup: Summary,
    pub decision: Decision,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub records: Vec<Record>,
    pub candidates: usize,
    /// Payload and manifest logical bytes only, not a promise of disk space freed.
    pub candidate_bytes: u64,
    pub keep_per_source: usize,
}

/// Protect at least one recent backup per source, including all ties at the
/// cutoff. Unknown provenance or an incomplete inventory never yields deletion
/// candidates. In-use paths are additional protection, not substitutes for recent
/// backups. Callers must pass paths from the same canonical inventory root.
pub fn preview(
    inventory: &Inventory,
    keep_per_source: usize,
    in_use: &BTreeSet<PathBuf>,
) -> io::Result<Preview> {
    let keep = keep_per_source.max(1);
    let mut groups: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for backup in &inventory.entries {
        if let Some(id) = &backup.source_id {
            groups.entry(id).or_default().push(backup.created);
        }
    }
    let cutoffs: BTreeMap<_, _> = groups
        .into_iter()
        .map(|(id, mut times)| {
            times.sort_unstable_by(|a, b| b.cmp(a));
            (id, times[keep.min(times.len()) - 1])
        })
        .collect();
    let mut result = Preview {
        records: Vec::new(),
        candidates: 0,
        candidate_bytes: 0,
        keep_per_source: keep,
    };
    for backup in &inventory.entries {
        let decision = if inventory.unreadable > 0 || !inventory.interrupted.is_empty() {
            Decision::IncompleteInventory
        } else if backup.protected {
            Decision::Protected
        } else if in_use.contains(&backup.directory) {
            Decision::InUse
        } else if let Some(id) = &backup.source_id {
            if backup.created >= cutoffs[id.as_str()] {
                Decision::Recent
            } else {
                Decision::Candidate
            }
        } else {
            Decision::UnknownSource
        };
        if decision == Decision::Candidate {
            result.candidates += 1;
            result.candidate_bytes = result
                .candidate_bytes
                .checked_add(backup.payload_bytes)
                .and_then(|bytes| bytes.checked_add(backup.manifest_bytes))
                .ok_or_else(|| invalid("清理候选容量溢出"))?;
        }
        result.records.push(Record {
            backup: backup.clone(),
            decision,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn backup(path: &str, source: Option<&str>, created: u64) -> Summary {
        Summary {
            directory: path.into(),
            source_id: source.map(str::to_owned),
            protected: false,
            source_name: "同名笔记库".into(),
            created,
            files: 1,
            payload_bytes: 100,
            manifest_bytes: 10,
        }
    }
    #[test]
    fn preview_separates_sources_and_protects_legacy_active_and_cutoff_ties() {
        let inventory = Inventory {
            entries: vec![
                backup("a-old", Some("a"), 1),
                backup("b-only", Some("b"), 1),
                backup("a-new", Some("a"), 9),
                backup("a-tie", Some("a"), 9),
                backup("legacy", None, 1),
                backup("a-active", Some("a"), 2),
            ],
            ..Default::default()
        };
        let protected = [PathBuf::from("a-active")].into();
        let result = preview(&inventory, 0, &protected).unwrap();
        assert_eq!(result.keep_per_source, 1);
        assert_eq!(
            result
                .records
                .iter()
                .map(|r| r.decision)
                .collect::<Vec<_>>(),
            [
                Decision::Candidate,
                Decision::Recent,
                Decision::Recent,
                Decision::Recent,
                Decision::UnknownSource,
                Decision::InUse
            ]
        );
        assert_eq!((result.candidates, result.candidate_bytes), (1, 110));
        assert_eq!(preview(&inventory, 2, &protected).unwrap().candidates, 1);
        assert_eq!(preview(&inventory, 99, &protected).unwrap().candidates, 0);
        let incomplete = Inventory {
            unreadable: 1,
            ..inventory
        };
        let result = preview(&incomplete, 1, &protected).unwrap();
        assert_eq!(result.candidates, 0);
        assert!(
            result
                .records
                .iter()
                .all(|r| r.decision == Decision::IncompleteInventory)
        );
    }
    #[test]
    fn empty_preview_and_overflow_are_explicit() {
        assert_eq!(
            preview(&Inventory::default(), 3, &BTreeSet::new())
                .unwrap()
                .candidate_bytes,
            0
        );
        let mut old = backup("old", Some("source"), 1);
        old.payload_bytes = u64::MAX;
        let inventory = Inventory {
            entries: vec![old, backup("new", Some("source"), 2)],
            ..Default::default()
        };
        assert!(preview(&inventory, 1, &BTreeSet::new()).is_err());
    }
}
