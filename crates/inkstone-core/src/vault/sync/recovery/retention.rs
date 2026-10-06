//! Advisory sync-backup retention. A preview never authorizes deletion.
use super::{Entry, Inventory};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Still requires a locked inventory refresh and payload integrity verification.
    Candidate,
    Recent,
    Protected,
    InUse,
    IncompleteInventory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub backup: Entry,
    pub decision: Decision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub records: Vec<Record>,
    pub keep_per_note: usize,
    pub candidates: usize,
    /// Payload lengths only, excluding descriptors and allocated-block overhead.
    pub candidate_bytes: u64,
}

/// Retain at least one backup for each recorded original path, including all
/// timestamp ties at the cutoff. A missing original does not remove protection.
/// `in_use` contains vault-relative payload paths being reviewed or restored.
/// Any unreadable, unindexed or unmeasured data blocks the entire preview from
/// producing candidates. Unknown records remain in the inventory, untouched.
pub fn preview(
    inventory: &Inventory,
    keep_per_note: usize,
    in_use: &BTreeSet<PathBuf>,
) -> Result<Preview> {
    let keep = keep_per_note.max(1);
    let mut groups: BTreeMap<&std::path::Path, Vec<SystemTime>> = BTreeMap::new();
    for entry in &inventory.entries {
        groups
            .entry(&entry.original)
            .or_default()
            .push(entry.modified);
    }
    let cutoffs: BTreeMap<_, _> = groups
        .into_iter()
        .map(|(path, mut times)| {
            times.sort_unstable_by(|a, b| b.cmp(a));
            (path, times[keep.min(times.len()) - 1])
        })
        .collect();
    let incomplete =
        inventory.unreadable > 0 || inventory.unindexed_files > 0 || inventory.unmeasured_files > 0;
    let mut result = Preview {
        records: Vec::new(),
        keep_per_note: keep,
        candidates: 0,
        candidate_bytes: 0,
    };
    for entry in &inventory.entries {
        let decision = if incomplete {
            Decision::IncompleteInventory
        } else if entry.protected {
            Decision::Protected
        } else if in_use.contains(&entry.backup) {
            Decision::InUse
        } else if entry.modified >= cutoffs[entry.original.as_path()] {
            Decision::Recent
        } else {
            Decision::Candidate
        };
        if decision == Decision::Candidate {
            result.candidates += 1;
            result.candidate_bytes = result
                .candidate_bytes
                .checked_add(entry.bytes)
                .context("同步备份清理候选容量溢出")?;
        }
        result.records.push(Record {
            backup: entry.clone(),
            decision,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn entry(id: &str, original: &str, time: u64) -> Entry {
        Entry {
            metadata: format!(".inkstone-sync-{id}.backup.json").into(),
            backup: format!(".inkstone-sync-{id}.backup").into(),
            original: original.into(),
            modified: UNIX_EPOCH + Duration::from_secs(time),
            bytes: 10,
            expected_sha256: "a".repeat(64),
            protected: false,
        }
    }
    fn inventory() -> Inventory {
        Inventory {
            entries: vec![
                entry("old", "one.md", 1),
                entry("active", "one.md", 2),
                entry("new", "one.md", 9),
                entry("tie", "one.md", 9),
                entry("only", "nested/two.md", 1),
            ],
            ..Default::default()
        }
    }
    #[test]
    fn retains_each_original_cutoff_ties_and_active_recoveries() {
        let inventory = inventory();
        let active = BTreeSet::from([inventory.entries[1].backup.clone()]);
        let report = preview(&inventory, 0, &active).unwrap();
        assert_eq!(report.keep_per_note, 1);
        assert_eq!((report.candidates, report.candidate_bytes), (1, 10));
        assert_eq!(
            report
                .records
                .iter()
                .map(|r| r.decision)
                .collect::<Vec<_>>(),
            [
                Decision::Candidate,
                Decision::InUse,
                Decision::Recent,
                Decision::Recent,
                Decision::Recent,
            ]
        );
        assert_eq!(preview(&inventory, 2, &active).unwrap().candidates, 1);
        assert_eq!(preview(&inventory, 99, &active).unwrap().candidates, 0);
        assert_eq!(
            preview(&Inventory::default(), 1, &active)
                .unwrap()
                .candidates,
            0
        );
    }
    #[test]
    fn every_kind_of_incomplete_inventory_protects_all_backups() {
        for kind in 0..3 {
            let mut inventory = inventory();
            match kind {
                0 => inventory.unreadable = 1,
                1 => inventory.unindexed_files = 1,
                _ => inventory.unmeasured_files = 1,
            }
            let report = preview(&inventory, 1, &BTreeSet::new()).unwrap();
            assert_eq!((report.candidates, report.candidate_bytes), (0, 0));
            assert!(
                report
                    .records
                    .iter()
                    .all(|r| r.decision == Decision::IncompleteInventory)
            );
        }
    }
    #[test]
    fn explicitly_protected_old_records_never_become_candidates() {
        let mut inventory = inventory();
        inventory.entries[0].protected = true;
        let result = preview(&inventory, 1, &BTreeSet::new()).unwrap();
        assert_eq!(result.records[0].decision, Decision::Protected);
        assert_eq!((result.candidates, result.candidate_bytes), (1, 10));
        assert_eq!(result.records[1].decision, Decision::Candidate);
    }

    #[test]
    fn candidate_size_overflow_is_an_error() {
        let mut inventory = inventory();
        inventory.entries[0].bytes = u64::MAX;
        assert!(preview(&inventory, 1, &BTreeSet::new()).is_err());
    }
}
