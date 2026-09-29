use std::cell::{Cell, RefCell};

/// Save and conflict state belongs to the document, independently of its views.
#[derive(Default)]
pub(super) struct DocumentSaveState {
    pub baseline: RefCell<Option<String>>,
    pub dirty: Cell<bool>,
    pub saving: Cell<bool>,
    pub conflict: Cell<bool>,
    pub error: RefCell<Option<String>>,
    pub recovery_text: RefCell<String>,
}

impl DocumentSaveState {
    pub(super) fn new(baseline: Option<String>, dirty: bool) -> Self {
        Self {
            baseline: RefCell::new(baseline),
            dirty: Cell::new(dirty),
            ..Self::default()
        }
    }
}
