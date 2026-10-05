mod counts;
mod editing;
mod presentation;
mod reading;
#[cfg(test)]
mod tests;
mod view;

use crate::editor_links::{self, ParsedCache, PathCache, WikiCompletions, WikiDefinitions};
use gpui::{prelude::*, *};
use gpui_base::text::{TextView, TextViewState};
use gpui_component::input::{Editor, EditorState, TextDecoration, TextDecorationCollection};
use inkstone_core::index::{self, ParsedNote};
use inkstone_core::markdown::{self, Kind};
use std::path::PathBuf;
use std::{cell::RefCell, rc::Rc, sync::Arc};
mod font_zoom;
mod footnotes;
#[cfg(test)]
mod frame_benchmark;
mod live_lists;
mod live_objects;
mod live_quotes;
mod live_rules;
mod live_tasks;

pub enum EditorEvent {
    CountsChanged,
    FontSizeDelta(i8),
    FollowLink(String),
    FollowLinkInNewTab(String),
    FollowMarkdownLink(String),
    FollowMarkdownLinkInNewTab(String),
    PasteFiles(Vec<PathBuf>),
    PasteImage(String, Vec<u8>),
    FollowReference(inkstone_core::rendering::Reference),
    FollowReferenceInNewTab(inkstone_core::rendering::Reference),
    ToggleTask(inkstone_core::rendering::TaskTarget, bool),
}
impl EventEmitter<EditorEvent> for EditorPane {}

#[derive(PartialEq)]
struct PresentationSnapshot {
    text: SharedString,
    selections: Vec<std::ops::Range<usize>>,
    live: bool,
    light: bool,
    search: Option<String>,
    font_size: f32,
    line_spacing: f32,
}

struct CountSnapshot {
    source: SharedString,
    selection: std::ops::Range<usize>,
    counts: inkstone_core::word_count::Counts,
}

struct PropertySnapshot {
    source: SharedString,
    properties: Vec<inkstone_core::properties::Property>,
}

pub struct EditorPane {
    pub navigation: inkstone_core::preferences::Navigation,
    count_cache: Option<CountSnapshot>,
    property_cache: Option<PropertySnapshot>,
    count_task: Option<Task<()>>,
    count_revision: u64,
    footnote_edit: Option<footnotes::FootnoteEdit>,
    live_tasks: Vec<live_tasks::TaskWidget>,
    live_quotes: Vec<std::ops::Range<usize>>,
    live_rules: Vec<std::ops::Range<usize>>,
    live_lists: Vec<std::ops::Range<usize>>,
    pub editor: Entity<EditorState>,
    decorations: TextDecorationCollection,
    pub live: bool,
    parse_source: SharedString,
    parse_revision: u64,
    spans: Vec<markdown::Span>,
    parse_task: Option<Task<()>>,
    pub parsed: ParsedNote,
    pub reading: bool,
    pub font_size: f32,
    pub line_spacing: f32,
    pub quick_font_size: bool,
    font_zoom: font_zoom::WheelZoom,
    pub text_font: SharedString,
    pub readable_width: bool,
    pub strict_line_breaks: bool,
    pub indentation: gpui_base::input::TabSize,
    pub smart_lists: bool,
    pub fold_headings: bool,
    pub fold_indentation: bool,
    pub light: bool,
    pub history_owner: Option<Entity<EditorState>>,
    preview: Entity<TextViewState>,
    pub image_dir: PathBuf,
    pub current_path: PathBuf,
    pub vault_root: PathBuf,
    reference_index: Arc<index::Index>,
    context_revision: u64,
    parsed_context_revision: u64,
    syntax_snapshot: Option<Arc<inkstone_core::syntax::Snapshot>>,
    live_objects: Vec<live_objects::Widget>,
    graphic_tasks: Vec<Task<()>>,
    pending_live_anchor: Option<gpui_base::input::DisplayScrollAnchor>,
    graphic_dpi: f32,
    graphic_appearance: Option<(u32, u32, bool)>,
    rendered: Arc<inkstone_core::rendering::ReadingDocument>,
    link_cache: ParsedCache,
    path_cache: PathCache,
    last_presentation: Option<PresentationSnapshot>,
    overlay_source: SharedString,
    _subscription: Subscription,
    _completion_subscription: Subscription,
    _preview_subscription: Subscription,
    pending_preview_jump: Option<usize>,
    pending_reading_position: Option<inkstone_core::preferences::ReadingPosition>,
    last_geometry: Option<(Size<Pixels>, bool)>,
    reveal_after_concealment: bool,
    typography_ready: bool,
}

impl EditorPane {
    pub fn set_reference_context(
        &mut self,
        path: PathBuf,
        root: PathBuf,
        index: Arc<index::Index>,
        cx: &mut Context<Self>,
    ) {
        if self.current_path != path
            || self.vault_root != root
            || !Arc::ptr_eq(&self.reference_index, &index)
        {
            if self.current_path != path || self.vault_root != root {
                self.rendered = Arc::default();
            }
            self.current_path = path;
            self.vault_root = root;
            self.reference_index = index;
            self.context_revision += 1;
            cx.notify();
        }
    }
    pub fn set_paths(&mut self, paths: Arc<Vec<editor_links::CompletionPath>>) {
        *self.path_cache.borrow_mut() = paths;
    }
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let indentation = gpui_base::input::TabSize {
            tab_size: 4,
            hard_tabs: true,
        };
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .tab_size(indentation)
                .default_value(text)
                .line_number(false)
        });
        let decorations = editor.update(cx, |state, cx| {
            state.create_decorations_collection(vec![], cx)
        });
        let subscription = cx.observe_in(&editor, window, |this, editor, window, cx| {
            let state = editor.read(cx);
            let visible = state.cursor_layout().is_some_and(|(mut caret, _)| {
                caret.origin += state.scroll_offset();
                state.input_bounds().intersects(&caret)
            });
            let size = state.input_bounds().size;
            let previous = this.last_geometry.replace((size, visible));
            if !this.reading
                && state.focus_handle(cx).is_focused(window)
                && previous.is_some_and(|(old, was_visible)| old != size && was_visible)
            {
                // Geometry notifications arrive after paint, so the new map is ready.
                cx.on_next_frame(window, |this, window, cx| {
                    if !this.reading && this.editor.read(cx).focus_handle(cx).is_focused(window) {
                        this.editor.update(cx, |state, cx| state.reveal_cursor(cx));
                    }
                });
            }
            cx.notify();
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));
        let completion_subscription =
            cx.subscribe_in(&editor, window, |_, editor, event, window, cx| {
                if !matches!(event, gpui_component::input::InputEvent::Change) {
                    return;
                }
                editor.update(cx, |state, cx| {
                    if !state.focus_handle(cx).is_focused(window)
                        || !state.selected_range().is_empty()
                        || state.marked_text_range(window, cx).is_some()
                    {
                        return;
                    }
                    let text = state.value();
                    let before = &text[..state.selected_range().start];
                    if before
                        .rfind("[[")
                        .is_some_and(|start| !before[start + 2..].contains([']', '|', '\n', '\r']))
                    {
                        // The host provider filters the complete link query. A pasted prefix
                        // must not become the component's separate fuzzy-search query.
                        state.request_completions(window, cx);
                    }
                });
            });
        let preview_subscription = cx.observe(&preview, |this, _, cx| {
            if this.pending_preview_jump.is_some() || this.pending_reading_position.is_some() {
                cx.notify();
            }
        });
        let link_cache = Rc::new(RefCell::new((
            SharedString::default(),
            ParsedNote::default(),
        )));
        let path_cache = Rc::new(RefCell::new(Arc::new(vec![])));
        let weak = cx.entity().downgrade();
        editor.update(cx, |state, _| {
            state.lsp_mut().completion_menu.max_width = px(500.);
            state.lsp_mut().definition_provider =
                Some(Rc::new(WikiDefinitions(link_cache.clone())));
            state.lsp_mut().completion_provider = Some(Rc::new(WikiCompletions(
                path_cache.clone(),
                Some(editor.downgrade()),
            )));
            state.lsp_mut().show_document_with_modifiers =
                Some(Rc::new(move |params, modifiers, window, cx| {
                    if let Some(offset) = params
                        .uri
                        .as_str()
                        .strip_prefix("inkstone-footnote:")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        let weak = weak.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.editor
                                    .update(cx, |s, cx| s.set_selected_range(offset..offset, cx));
                                this.open_footnote(window, cx);
                            });
                        });
                        return true;
                    }
                    let Some(target) = editor_links::decode_uri(&params.uri.to_string()) else {
                        return false;
                    };
                    let _ = weak.update(cx, |this, cx| {
                        let new_tab = modifiers
                            .is_some_and(|keys| keys.secondary() && (this.live || keys.shift));
                        cx.emit(if new_tab {
                            EditorEvent::FollowLinkInNewTab(target)
                        } else {
                            EditorEvent::FollowLink(target)
                        });
                    });
                    true
                }));
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        Self {
            editor,
            footnote_edit: None,
            count_cache: None,
            property_cache: None,
            count_task: None,
            count_revision: 0,
            decorations,
            live: true,
            parse_source: "".into(),
            parse_revision: 0,
            spans: vec![],
            parse_task: None,
            parsed: ParsedNote::default(),
            reading: false,
            navigation: Default::default(),
            live_tasks: vec![],
            live_quotes: vec![],
            live_rules: vec![],
            live_lists: vec![],
            font_size: 16.,
            line_spacing: 1.5,
            quick_font_size: false,
            font_zoom: Default::default(),
            text_font: "Microsoft YaHei UI".into(),
            readable_width: true,
            strict_line_breaks: false,
            indentation,
            smart_lists: true,
            fold_headings: true,
            fold_indentation: true,
            light: false,
            history_owner: None,
            preview,
            image_dir: PathBuf::new(),
            current_path: PathBuf::new(),
            vault_root: PathBuf::new(),
            reference_index: Arc::default(),
            context_revision: 0,
            parsed_context_revision: 0,
            syntax_snapshot: None,
            live_objects: vec![],
            graphic_tasks: vec![],
            pending_live_anchor: None,
            graphic_dpi: 1.,
            graphic_appearance: None,
            rendered: Arc::default(),
            link_cache,
            path_cache,
            last_presentation: None,
            overlay_source: "".into(),
            _subscription: subscription,
            _completion_subscription: completion_subscription,
            _preview_subscription: preview_subscription,
            pending_preview_jump: None,
            pending_reading_position: None,
            last_geometry: None,
            reveal_after_concealment: false,
            typography_ready: false,
        }
    }
}
