use gpui::{Entity, Subscription};
use gpui_component::input::EditorState;
use std::cell::{Cell, RefCell};

/// Shared contents, history and save state, independent of any one view.
pub(super) struct DocumentState {
    pub path: RefCell<std::path::PathBuf>,
    pub editor: Entity<EditorState>,
    pub baseline: RefCell<Option<String>>,
    pub dirty: Cell<bool>,
    pub saving: Cell<bool>,
    pub conflict: Cell<bool>,
    pub error: RefCell<Option<String>>,
    pub recovery_text: RefCell<String>,
    _changes: Subscription,
}

impl DocumentState {
    pub(super) fn new(
        path: std::path::PathBuf,
        baseline: Option<String>,
        dirty: bool,
        editor: Entity<EditorState>,
        changes: Subscription,
    ) -> Self {
        Self {
            path: RefCell::new(path),
            editor,
            baseline: RefCell::new(baseline),
            dirty: Cell::new(dirty),
            saving: Cell::new(false),
            conflict: Cell::new(false),
            error: RefCell::new(None),
            recovery_text: RefCell::new(String::new()),
            _changes: changes,
        }
    }
}

use super::*;

impl Workspace {
    /// Bring committed view edits into their document owners before save/close.
    pub(super) fn flush_document_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.tabs.iter().map(|tab| tab.id).collect();
        for id in ids {
            self.document_view_changed(id, window, cx);
        }
        self.sync_from_split(window, cx);
    }

    pub(super) fn relocate_document(
        &mut self,
        document: &std::rc::Rc<DocumentState>,
        path: std::path::PathBuf,
    ) {
        document.path.replace(path.clone());
        for tab in self
            .tabs
            .iter_mut()
            .filter(|tab| std::rc::Rc::ptr_eq(&tab.save, document))
        {
            tab.path = path.clone();
        }
    }

    pub(super) fn document_changed(
        &mut self,
        owner: Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self
            .tabs
            .iter()
            .find(|tab| tab.save.editor.entity_id() == owner.entity_id())
            .map(|tab| tab.save.clone())
        else {
            return;
        };
        if owner.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let text = owner.read(cx).value();
        document
            .dirty
            .set(document.baseline.borrow().as_deref() != Some(text.as_ref()));
        let mut sources = Vec::new();
        for tab in self
            .tabs
            .iter_mut()
            .filter(|tab| std::rc::Rc::ptr_eq(&tab.save, &document))
        {
            sources.push(tab.id);
            let editor = tab.pane.read(cx).editor.clone();
            if editor.entity_id() == owner.entity_id() {
                tab.synced_text = text.clone();
                continue;
            }
            if editor.update(cx, |editor, cx| {
                editor.marked_text_range(window, cx).is_some()
            }) {
                continue;
            }
            let before = editor.read(cx).value();
            if before == text {
                tab.synced_text = text.clone();
                continue;
            }
            // Pending edits in another view need their own synchronization turn.
            if before != tab.synced_text {
                continue;
            }
            let edits = inkstone::text_changes::diff(&before, &text);
            let focused = editor.read(cx).focus_handle(cx).is_focused(window);
            if editor.update(cx, |editor, cx| {
                editor.apply_synced_text(
                    &text,
                    &edits,
                    gpui_base::input::SyncedHistory::Ignore,
                    focused,
                    window,
                    cx,
                )
            }) {
                tab.synced_text = text.clone();
            }
        }
        for source in sources {
            self.sync_to_split(source, window, cx);
        }
        cx.notify();
    }

    pub(super) fn document_view_changed(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        let editor = tab.pane.read(cx).editor.clone();
        if editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let after = editor.read(cx).value();
        if after == tab.synced_text {
            return;
        }
        let owner = tab.save.editor.clone();
        let before = owner.read(cx).value();
        if before != after {
            let edits = inkstone::text_changes::diff(&before, &after);
            let history = editor.read(cx).history_group_id().map_or(
                gpui_base::input::SyncedHistory::Record,
                gpui_base::input::SyncedHistory::Group,
            );
            if !owner.update(cx, |owner, cx| {
                owner.apply_synced_text(&after, &edits, history, false, window, cx)
            }) {
                return;
            }
        }
        self.document_changed(owner, window, cx);
    }
}
