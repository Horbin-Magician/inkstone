//! Workspace-owned task accounting, independent of views and vault generations.
//! A vault switch must wait for these tasks; finishing an old/foreign ticket
//! cannot release another task's pending-write guard.
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub(super) struct FileWrites {
    owner: Arc<()>,
    next: u64,
    active: HashSet<u64>,
}

#[must_use = "retain the ticket until the file task has actually completed"]
pub(super) struct Ticket {
    owner: Arc<()>,
    id: u64,
}

impl FileWrites {
    pub fn begin(&mut self) -> Ticket {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("file task ids exhausted");
        self.active.insert(id);
        Ticket {
            owner: self.owner.clone(),
            id,
        }
    }

    pub fn finish(&mut self, ticket: Ticket) -> bool {
        Arc::ptr_eq(&self.owner, &ticket.owner) && self.active.remove(&ticket.id)
    }

    pub fn pending(&self) -> usize {
        self.active.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_is_owned_and_out_of_order_without_releasing_other_tasks() {
        let mut writes = FileWrites::default();
        let first = writes.begin();
        let second = writes.begin();
        let stale = Ticket {
            owner: first.owner.clone(),
            id: first.id,
        };
        assert_eq!(writes.pending(), 2);
        assert!(writes.finish(second));
        assert_eq!(writes.pending(), 1);
        assert!(writes.finish(first));
        let third = writes.begin();
        assert!(!writes.finish(stale));
        assert_eq!(writes.pending(), 1);
        assert!(writes.finish(third));
        assert_eq!(writes.pending(), 0);
    }

    #[test]
    fn foreign_workspace_ticket_cannot_release_a_matching_local_id() {
        let mut one = FileWrites::default();
        let mut two = FileWrites::default();
        let first = one.begin();
        let foreign = Ticket {
            owner: first.owner.clone(),
            id: first.id,
        };
        let second = two.begin();
        assert!(!two.finish(foreign));
        assert_eq!(two.pending(), 1);
        assert!(one.finish(first));
        assert!(two.finish(second));
    }
}
