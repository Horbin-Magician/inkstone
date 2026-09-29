use crate::editor_links::{self, ParsedCache, PathCache, WikiCompletions, WikiDefinitions};
use gpui::{prelude::*, *};
use gpui_base::text::{TextView, TextViewState};
use gpui_component::input::{Editor, EditorState, TextDecoration, TextDecorationCollection};
use inkstone::index::{self, ParsedNote};
use inkstone::markdown::{self, Kind};
use std::path::PathBuf;
use std::{cell::RefCell, rc::Rc, sync::Arc};
mod footnotes;

pub enum EditorEvent {
    CountsChanged,
    FollowLink(String),
    FollowMarkdownLink(String),
    PasteFiles(Vec<PathBuf>),
    PasteImage(String, Vec<u8>),
    FollowReference(inkstone::rendering::Reference),
    ToggleTask(inkstone::rendering::TaskTarget, bool),
}
impl EventEmitter<EditorEvent> for EditorPane {}

#[derive(PartialEq)]
struct PresentationSnapshot {
    text: SharedString,
    selection: std::ops::Range<usize>,
    live: bool,
    light: bool,
    search: Option<String>,
}

struct CountSnapshot {
    source: SharedString,
    selection: std::ops::Range<usize>,
    counts: inkstone::word_count::Counts,
}

pub struct EditorPane {
    count_cache: Option<CountSnapshot>,
    count_task: Option<Task<()>>,
    count_revision: u64,
    footnote_edit: Option<footnotes::FootnoteEdit>,
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
    rendered: Arc<inkstone::rendering::ReadingDocument>,
    link_cache: ParsedCache,
    path_cache: PathCache,
    last_presentation: Option<PresentationSnapshot>,
    _subscription: Subscription,
    _completion_subscription: Subscription,
    _preview_subscription: Subscription,
    pending_preview_jump: Option<usize>,
    pending_reading_position: Option<inkstone::preferences::ReadingPosition>,
    last_geometry: Option<(Size<Pixels>, bool)>,
    reveal_after_concealment: bool,
    typography_ready: bool,
}

impl EditorPane {
    pub fn text_counts(&mut self, cx: &mut Context<Self>) -> inkstone::word_count::Counts {
        let editor = self.editor.read(cx);
        let source = editor.value();
        let selection = if self.reading || editor.selected_range().is_empty() {
            0..0
        } else {
            editor.selected_range()
        };
        if let Some(cache) = &self.count_cache
            && cache.source == source
            && cache.selection == selection
        {
            return cache.counts;
        }
        self.count_revision = self.count_revision.wrapping_add(1);
        let revision = self.count_revision;
        self.count_task = None;
        let previous = self
            .count_cache
            .as_ref()
            .map_or(Default::default(), |cache| cache.counts);
        let length = if selection.is_empty() {
            source.len()
        } else {
            selection.len()
        };
        self.count_cache = Some(CountSnapshot {
            source: source.clone(),
            selection: selection.clone(),
            counts: previous,
        });
        let calculate = move || {
            let text = if selection.is_empty() {
                inkstone::word_count::document_body(&source)
            } else {
                &source[selection]
            };
            inkstone::word_count::count(text)
        };
        if length <= 4096 {
            let counts = calculate();
            self.count_cache.as_mut().unwrap().counts = counts;
            return counts;
        }
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(200));
        self.count_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let counts = cx
                .background_executor()
                .spawn(async move { calculate() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.count_revision == revision {
                    if let Some(cache) = &mut this.count_cache {
                        cache.counts = counts;
                    }
                    cx.emit(EditorEvent::CountsChanged);
                    cx.notify();
                }
            });
        }));
        previous
    }
    pub fn set_fold_options(
        &mut self,
        headings: bool,
        indentation: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold_headings == headings && self.fold_indentation == indentation {
            return;
        }
        self.fold_headings = headings;
        self.fold_indentation = indentation;
        let ranges = index::parse(&self.editor.read(cx).value()).fold_ranges(headings, indentation);
        self.editor.update(cx, |state, cx| {
            state.set_folding(headings || indentation, window, cx);
            state.apply_highlighter_fold_candidates(
                ranges
                    .iter()
                    .map(|r| gpui_component::input::FoldRange::new(r.start, r.end))
                    .collect(),
                cx,
            );
        });
        cx.notify();
    }
    pub fn set_auto_pairing(&mut self, brackets: bool, markdown: bool, cx: &mut Context<Self>) {
        use gpui_base::input::{AutoClosingPair, language_config::LanguageConfig};
        let mut pairs = vec![];
        if brackets {
            pairs.extend(
                [("(", ")"), ("[", "]"), ("{", "}"), ("'", "'"), ("\"", "\"")]
                    .into_iter()
                    .map(|(open, close)| AutoClosingPair::new(open, close)),
            );
        }
        if markdown {
            pairs.extend(
                ["```", "*", "_", "`"]
                    .into_iter()
                    .map(|marker| AutoClosingPair::new(marker, marker)),
            );
        }
        static LINE_START: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"^[>\s]*(?:(?:[*+-] |[0-9]+[.)] )(?:\[[^\r\n]\] )?)?").unwrap()
        });
        let mut config = LanguageConfig::default()
            .line_start_pattern(LINE_START.clone())
            .brackets([])
            .surround_selection(true)
            .skip_only_generated(true)
            .auto_closing_pairs(pairs);
        if markdown {
            static PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"^([>\s]*)([*+-] |[0-9]+[.)] )?").unwrap()
            });
            config = config.newline_closers(["```".into()], PREFIX.clone());
        }
        self.editor
            .update(cx, |editor, cx| editor.set_editing_rules(Some(config), cx));
    }
    pub fn callout_states(&self, cx: &App) -> std::collections::BTreeMap<u64, bool> {
        self.preview.read(cx).callout_states()
    }
    pub fn restore_callout_states(
        &mut self,
        states: &std::collections::BTreeMap<u64, bool>,
        cx: &mut Context<Self>,
    ) {
        self.preview
            .update(cx, |preview, cx| preview.restore_callout_states(states, cx));
    }
    pub fn reading_position(&self, cx: &App) -> inkstone::preferences::ReadingPosition {
        self.pending_reading_position.unwrap_or_else(|| {
            let position = self.preview.read(cx).scroll_position();
            inkstone::preferences::ReadingPosition {
                block: position.item_ix,
                offset: f32::from(position.offset_in_item),
            }
        })
    }
    pub fn restore_reading_position(
        &mut self,
        position: inkstone::preferences::ReadingPosition,
        cx: &mut Context<Self>,
    ) {
        self.pending_preview_jump = None;
        self.pending_reading_position = Some(position);
        cx.notify();
    }
    pub fn fold_sections(
        &mut self,
        all: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading {
            self.reading = false;
        }
        let parsed = index::parse(&self.editor.read(cx).value());
        self.editor.update(cx, |state, cx| {
            state.apply_highlighter_fold_candidates(
                parsed
                    .fold_ranges(self.fold_headings, self.fold_indentation)
                    .iter()
                    .map(|r| gpui_component::input::FoldRange::new(r.start, r.end))
                    .collect(),
                cx,
            );
            state.fold_sections(all, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    pub fn move_lines(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply_selection_transform(
                |text, selections| inkstone::line_edit::move_lines(text, selections, down),
                window,
                cx,
            );
            editor.focus(window, cx);
        });
    }

    pub fn copy_lines(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply_selection_transform(
                |text, selections| inkstone::line_edit::copy_lines(text, selections, down),
                window,
                cx,
            );
            editor.focus(window, cx);
        });
    }

    fn markdown_key(
        &mut self,
        key: inkstone::markdown_edit::Key,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.smart_lists
            && matches!(
                key,
                inkstone::markdown_edit::Key::Enter | inkstone::markdown_edit::Key::SoftEnter
            )
        {
            return;
        }
        if self
            .editor
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        {
            return;
        }
        let state = self.editor.read(cx);
        if self.reading
            || !state.focus_handle(cx).is_focused(window)
            || state.completion_menu_state().open
            || state.code_action_menu_state().open
        {
            return;
        }
        if state.has_multiple_selections() {
            if matches!(key, inkstone::markdown_edit::Key::Enter) {
                let indent = self.indentation;
                if self.editor.update(cx, |state, cx| {
                    state.apply_selection_transform(
                        |text, selections| {
                            inkstone::markdown_edit::enter_at_selections(
                                text,
                                selections,
                                indent.tab_size,
                                indent.hard_tabs,
                            )
                        },
                        window,
                        cx,
                    )
                }) {
                    cx.stop_propagation();
                }
                return;
            }
            if matches!(key, inkstone::markdown_edit::Key::SoftEnter) {
                self.editor.update(cx, |state, cx| {
                    state.apply_selection_replacements(
                        inkstone::markdown_edit::soft_break_replacements,
                        window,
                        cx,
                    );
                });
                cx.stop_propagation();
                return;
            }
            if matches!(
                key,
                inkstone::markdown_edit::Key::Indent | inkstone::markdown_edit::Key::Outdent
            ) {
                let indent = self.indentation;
                self.editor.update(cx, |state, cx| {
                    state.apply_line_edits(
                        |line| {
                            inkstone::markdown_edit::indentation_change(
                                line,
                                indent.tab_size,
                                indent.hard_tabs,
                                matches!(key, inkstone::markdown_edit::Key::Outdent),
                            )
                        },
                        window,
                        cx,
                    );
                });
                cx.stop_propagation();
            }
            return;
        }
        if matches!(
            key,
            inkstone::markdown_edit::Key::Indent | inkstone::markdown_edit::Key::Outdent
        ) && !state.has_multiple_selections()
            && let Some(navigation) = inkstone::tables::navigate(
                &state.value(),
                state.selected_range(),
                matches!(key, inkstone::markdown_edit::Key::Outdent),
            )
        {
            self.editor.update(cx, |s, cx| match navigation {
                inkstone::tables::Navigation::Select(range) => s.set_selected_range(range, cx),
                inkstone::tables::Navigation::Edit(edit) => {
                    s.apply_source_edit(edit.range, &edit.replacement, edit.selection, window, cx);
                }
            });
            cx.stop_propagation();
            return;
        }
        let Some(edit) = inkstone::markdown_edit::edit_with_options(
            &state.value(),
            state.selected_range(),
            key,
            self.indentation.tab_size,
            self.indentation.hard_tabs,
            self.smart_lists,
        ) else {
            if matches!(
                key,
                inkstone::markdown_edit::Key::Indent | inkstone::markdown_edit::Key::Outdent
            ) {
                cx.stop_propagation();
            }
            return;
        };
        if self.editor.update(cx, |s, cx| {
            s.apply_source_edit(edit.range, &edit.replacement, edit.selection, window, cx)
        }) {
            cx.stop_propagation();
        }
    }
    #[cfg(test)]
    pub fn reading_bounds(&self, cx: &App) -> Bounds<Pixels> {
        self.preview.read(cx).bounds()
    }
    pub fn toggle_task_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        {
            return;
        }
        let selected = self.editor.read(cx).selected_range();
        let text = self.editor.read(cx).value();
        let Some((range, replacement)) = markdown::toggle_task_lines(&text, selected.clone())
        else {
            return;
        };
        self.reading = false;
        let start = range.start;
        let delta = replacement.len() as isize - range.len() as isize;
        let end = range.start + replacement.len();
        self.editor.update(cx, |s, cx| {
            s.set_selected_range(range, cx);
            s.replace(replacement, window, cx);
            if !selected.is_empty() {
                s.set_selected_range(start..end, cx);
            } else {
                let cursor = selected.start.saturating_add_signed(delta).min(end);
                s.set_selected_range(cursor..cursor, cx);
            }
            s.focus(window, cx);
        });
        cx.notify();
    }
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
            self.current_path = path;
            self.vault_root = root;
            self.reference_index = index;
            self.context_revision += 1;
            self.rendered = Arc::default();
            cx.notify();
        }
    }
    fn toggle_task(
        &mut self,
        target: inkstone::rendering::TaskTarget,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            || self.history_owner.as_ref().is_some_and(|owner| {
                owner.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            })
        {
            return;
        }
        gpui_base::TextSelection::end(window, cx);
        self.preview.update(cx, |s, cx| s.clear_selection(cx));
        self.focus_view(window, cx);
        if target.path != self.current_path {
            cx.emit(EditorEvent::ToggleTask(target, checked));
            return;
        }
        let current = self.editor.read(cx).value();
        if current.as_ref() != target.baseline.as_ref()
            || index::set_task(&current, target.marker.clone(), checked).is_none()
        {
            return;
        }
        self.editor.update(cx, |state, cx| {
            let selection = state.selected_range();
            let scroll = state.scroll_offset();
            let marker = target.marker;
            let map = |offset: usize| {
                if offset <= marker.start {
                    offset
                } else if offset >= marker.end {
                    offset.saturating_add_signed(1 - marker.len() as isize)
                } else {
                    marker.start + 1
                }
            };
            let selection = map(selection.start)..map(selection.end);
            state.set_selected_range(marker, cx);
            state.replace(if checked { "x" } else { " " }, window, cx);
            state.set_selected_range(selection, cx);
            state.set_scroll_offset(scroll, cx);
        });
        cx.notify();
    }
    pub fn focus_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading {
            let handle = self.preview.read(cx).focus_handle().clone();
            window.focus(&handle, cx);
        } else {
            self.editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }
    pub fn set_paths(&mut self, paths: Arc<Vec<editor_links::CompletionPath>>) {
        *self.path_cache.borrow_mut() = paths;
    }
    pub fn jump(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_reading_position = None;
        if self.reading {
            self.pending_preview_jump = Some(offset);
            self.focus_view(window, cx);
            cx.notify();
            return;
        }
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset, cx);
            state.focus(window, cx);
        });
        cx.notify();
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
            state.lsp_mut().show_document = Some(Rc::new(move |params, window, cx| {
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
                let _ = weak.update(cx, |_, cx| cx.emit(EditorEvent::FollowLink(target)));
                true
            }));
        });
        editor.update(cx, |state, cx| state.focus(window, cx));
        Self {
            editor,
            footnote_edit: None,
            count_cache: None,
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
            font_size: 16.,
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
            rendered: Arc::default(),
            link_cache,
            path_cache,
            last_presentation: None,
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

    fn update_presentation(&mut self, cx: &mut Context<Self>) {
        let state = self.editor.read(cx);
        let text = state.value();
        let selection = state.selected_range();
        let search_query = state
            .search_session()
            .is_active()
            .then(|| state.search_session().query.clone());
        let search_matches = state.search_session().matcher.matched_ranges();
        if self.parse_source != text || self.parsed_context_revision != self.context_revision {
            self.spans =
                markdown::rebase_spans(&self.parse_source, &text, std::mem::take(&mut self.spans));
            self.parse_source = text.clone();
            self.parsed_context_revision = self.context_revision;
            self.parse_revision += 1;
            let revision = self.parse_revision;
            self.parsed = ParsedNote::default();
            self.typography_ready = false;
            *self.link_cache.borrow_mut() = (SharedString::default(), ParsedNote::default());
            let source = text.clone();
            let references = self.reference_index.clone();
            let path = self.current_path.clone();
            let context_revision = self.context_revision;
            let task = cx.background_executor().spawn(async move {
                let parsed = index::parse(&source);
                let reading = inkstone::rendering::reading_document(&references, &path, &source);
                (markdown::spans(&source), parsed, reading)
            });
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let (spans, parsed, reading) = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.parse_revision != revision
                        || this.context_revision != context_revision
                        || this.editor.read(cx).value() != this.parse_source
                    {
                        return;
                    }
                    this.spans = spans;
                    this.parsed = parsed;
                    this.editor.update(cx, |state, cx| {
                        state.apply_highlighter_fold_candidates(
                            this.parsed
                                .fold_ranges(this.fold_headings, this.fold_indentation)
                                .iter()
                                .map(|r| gpui_component::input::FoldRange::new(r.start, r.end))
                                .collect(),
                            cx,
                        )
                    });
                    this.typography_ready = true;
                    *this.link_cache.borrow_mut() =
                        (this.parse_source.clone(), this.parsed.clone());
                    this.preview
                        .update(cx, |state, cx| state.set_text(&reading.markdown, cx));
                    this.rendered = Arc::new(reading);
                    this.last_presentation = None;
                    cx.notify();
                });
            }));
        }
        let key = PresentationSnapshot {
            text: text.clone(),
            selection: selection.clone(),
            live: self.live,
            light: self.light,
            search: search_query.clone(),
        };
        if self.last_presentation.as_ref() == Some(&key) {
            return;
        }
        self.last_presentation = Some(key);
        let mut decorations = Vec::new();
        let mut concealed = vec![];
        if self.live {
            for span in &self.spans {
                let style = match span.kind {
                    Kind::Heading => HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        color: Some(rgb(if self.light { 0x222222 } else { 0xdadada }).into()),
                        ..Default::default()
                    },
                    Kind::Strong => HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    },
                    Kind::Emphasis => HighlightStyle {
                        font_style: Some(FontStyle::Italic),
                        ..Default::default()
                    },
                    Kind::Strike => HighlightStyle {
                        strikethrough: Some(StrikethroughStyle {
                            thickness: px(1.),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Kind::Highlight => HighlightStyle {
                        background_color: Some(
                            rgba(if self.light { 0xf4d03f66 } else { 0x9e7d2866 }).into(),
                        ),
                        ..Default::default()
                    },
                    Kind::Code => HighlightStyle {
                        color: Some(rgb(0xf5c77e).into()),
                        background_color: Some(
                            rgb(if self.light { 0xf2f2f2 } else { 0x303030 }).into(),
                        ),
                        ..Default::default()
                    },
                    Kind::WikiLink | Kind::Link => HighlightStyle {
                        color: Some(rgb(0xa88bfa).into()),
                        ..Default::default()
                    },
                };
                decorations.push(TextDecoration::new(span.content.clone(), style));
                let search_reveals = search_query.is_some()
                    && span.markers.iter().any(|marker| {
                        search_matches
                            .get(search_matches.partition_point(|r| r.end <= marker.start))
                            .is_some_and(|r| r.start < marker.end)
                    });
                if !span.active(&selection) && !search_reveals {
                    for marker in &span.markers {
                        // Newlines remain in the source/display row map.
                        let mut start = marker.start;
                        for (i, ch) in text[marker.clone()].char_indices() {
                            if matches!(ch, '\r' | '\n') {
                                if start < marker.start + i {
                                    concealed.push(start..marker.start + i);
                                }
                                start = marker.start + i + ch.len_utf8();
                            }
                        }
                        if start < marker.end {
                            concealed.push(start..marker.end);
                        }
                    }
                }
            }
        }
        let styles = if !self.live {
            Some(vec![])
        } else if self.typography_ready {
            Some(
                self.parsed
                    .headings
                    .iter()
                    .filter_map(|heading| {
                        let raw = text.get(heading.offset..)?;
                        let length = raw
                            .bytes()
                            .take_while(|b| matches!(b, b'#' | b' ' | b'\t'))
                            .count();
                        let length = if length == 0 {
                            raw.chars().next()?.len_utf8()
                        } else {
                            length
                        };
                        let i = usize::from(heading.level.saturating_sub(1)).min(5);
                        let scale = markdown::HEADING_SCALES[i];
                        Some(gpui_base::input::LineTypography::new(
                            heading.offset..heading.offset + length,
                            scale,
                            scale * [1.2, 1.2, 1.3, 1.4, 1.5, 1.5][i] / 1.5,
                        ))
                    })
                    .collect(),
            )
        } else {
            None
        };
        let was_visible = self.last_geometry.is_some_and(|(_, visible)| visible);
        if let Some(styles) = styles
            && self
                .editor
                .update(cx, |s, cx| s.set_line_typography(styles, cx))
            && was_visible
        {
            self.reveal_after_concealment = true;
        }
        self.decorations.set(decorations, cx);
        let was_visible = self.last_geometry.is_some_and(|(_, visible)| visible);
        if self
            .editor
            .update(cx, |s, cx| s.set_concealed_ranges(concealed, cx))
            && (was_visible || search_query.is_some())
        {
            self.reveal_after_concealment = true;
        }
    }
}

impl Render for EditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "metrics")]
        if crate::metrics::needs_ready() {
            cx.on_next_frame(_window, |_, _, _| crate::metrics::record_ready());
        }
        self.update_presentation(cx);
        if std::mem::take(&mut self.reveal_after_concealment) && !self.reading {
            cx.on_next_frame(_window, |this, window, cx| {
                if this.reading {
                    return;
                }
                this.editor.update(cx, |state, cx| {
                    let search = state.search_session();
                    let found = search
                        .is_active()
                        .then(|| {
                            search
                                .matcher
                                .current()
                                .and_then(|i| search.matcher.matched_ranges().get(i).cloned())
                        })
                        .flatten();
                    if let Some(range) = found {
                        state.reveal_offset(range.start, cx);
                    } else if state.focus_handle(cx).is_focused(window) {
                        state.reveal_cursor(cx);
                    }
                });
            });
        }
        if self.reading
            && self.preview.read(cx).is_parsed()
            && self
                .rendered
                .source_matches(&self.current_path, &self.editor.read(cx).value())
            && let Some(offset) = self.pending_preview_jump.take()
        {
            let source = self
                .rendered
                .output_offset(&self.current_path, offset)
                .unwrap_or(0);
            if let Some(position) = self
                .preview
                .read(cx)
                .rendered_text()
                .position_for_source_offset(source)
            {
                self.preview.update(cx, |s, cx| {
                    let _ = s.reveal_range(position..position, cx);
                });
            }
        }
        if self.reading
            && self.preview.read(cx).is_parsed()
            && self
                .rendered
                .source_matches(&self.current_path, &self.editor.read(cx).value())
            && self.pending_reading_position.is_some()
        {
            cx.defer_in(_window, |this, _, cx| {
                if !this.reading
                    || !this.preview.read(cx).is_parsed()
                    || !this
                        .rendered
                        .source_matches(&this.current_path, &this.editor.read(cx).value())
                {
                    return;
                }
                let Some(position) = this.pending_reading_position.take() else {
                    return;
                };
                this.preview.update(cx, |s, cx| {
                    let count = s.list_state().item_count();
                    s.list_state().scroll_to(gpui::ListOffset {
                        item_ix: position.block.min(count.saturating_sub(1)),
                        offset_in_item: px(if position.offset.is_finite() {
                            position.offset.max(0.)
                        } else {
                            0.
                        }),
                    });
                    cx.notify();
                });
                cx.notify();
            });
        }
        let paste_weak = cx.entity().downgrade();
        let image_dir = self.image_dir.clone();
        let root = self.vault_root.clone();
        let asset_index = self.reference_index.clone();
        let rendered = self.rendered.clone();
        let task_weak = cx.entity().downgrade();
        let weak = cx.entity().downgrade();
        let font_size = self.font_size;
        let preview = TextView::new(&self.preview)
            .font_family(self.text_font.clone())
            .markdown_extensions(
                gpui_base::text::MarkdownExtensions::default()
                    .custom_task_markers(true)
                    .soft_line_breaks(!self.strict_line_breaks),
            )
            .text_size(px(font_size))
            .line_height(relative(1.5))
            .style(
                gpui_base::text::TextViewStyle::from_theme(&gpui_base::Theme::global(cx))
                    .with_paragraph_gap(rems(16. / f32::from(_window.rem_size()).max(1.)))
                    .with_heading(move |level| {
                        let i = usize::from(level.saturating_sub(1)).min(5);
                        StyleRefinement::default()
                            .text_size(px(font_size * markdown::HEADING_SCALES[i]))
                            .font_weight(if level == 1 {
                                FontWeight::BOLD
                            } else {
                                FontWeight::SEMIBOLD
                            })
                            .line_height(relative([1.2, 1.2, 1.3, 1.4, 1.5, 1.5][i]))
                    })
                    .with_code_block(StyleRefinement::default().text_size(px(font_size * 0.875)))
                    .with_table_cell(StyleRefinement::default().text_size(px(font_size)))
                    .with_table_head(
                        StyleRefinement::default()
                            .bg(rgb(if self.light { 0xf2f2f2 } else { 0x303030 }))
                            .text_color(rgb(if self.light { 0x222222 } else { 0xdadada })),
                    ),
            )
            .scrollable(true)
            .selectable(true)
            .on_task_toggle(move |offset, checked, window, cx| {
                let _ = task_weak.update(cx, |this, cx| {
                    if let Some(target) = this
                        .rendered
                        .tasks
                        .iter()
                        .find(|t| t.rendered_start == offset)
                        .cloned()
                    {
                        this.toggle_task(target, checked, window, cx);
                    }
                });
            })
            .image_source(move |uri| {
                let raw = uri.to_string();
                if let Some(id) = raw
                    .strip_prefix("inkstone-reference:")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let Some(reference) = rendered.references.get(id) {
                        if !reference.wiki
                            && inkstone::rendering::is_remote_image(&reference.target)
                        {
                            return SharedUri::from(reference.target.clone()).into();
                        }
                        if let Some(path) =
                            inkstone::rendering::asset_path(&root, reference, &asset_index.files)
                        {
                            return path.into();
                        }
                    }
                    root.join(".inkstone-missing-image").into()
                } else if inkstone::rendering::is_remote_image(&raw) {
                    uri.clone().into()
                } else {
                    image_dir
                        .join(
                            percent_encoding::percent_decode_str(raw.trim_start_matches("file://"))
                                .decode_utf8_lossy()
                                .as_ref(),
                        )
                        .into()
                }
            })
            .on_link_click(move |url, event, _, cx| {
                if event.is_right_click() {
                    return;
                }
                let _ = weak.update(cx, |this, cx| {
                    if let Some(id) = url
                        .strip_prefix("inkstone-reference:")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        if let Some(reference) = this.rendered.references.get(id).cloned() {
                            if !reference.wiki
                                && inkstone::rendering::is_external_link(&reference.target)
                            {
                                cx.open_url(&reference.target);
                            } else {
                                cx.emit(EditorEvent::FollowReference(reference));
                            }
                        }
                    } else if let Some(index) = url
                        .strip_prefix("inkstone-link:")
                        .and_then(|i| i.parse::<usize>().ok())
                    {
                        if let Some(link) = this.parsed.links.get(index) {
                            cx.emit(EditorEvent::FollowLink(link.target.clone()));
                        }
                    } else if inkstone::rendering::is_external_link(url) {
                        cx.open_url(url);
                    } else {
                        cx.emit(EditorEvent::FollowMarkdownLink(url.to_string()));
                    }
                });
            });
        div()
            .id("editor-pane")
            .capture_action(cx.listener(
                |this, action: &gpui_component::input::Enter, window, cx| {
                    if !action.secondary {
                        this.markdown_key(
                            if action.shift {
                                inkstone::markdown_edit::Key::SoftEnter
                            } else {
                                inkstone::markdown_edit::Key::Enter
                            },
                            window,
                            cx,
                        );
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::IndentInline, window, cx| {
                    this.markdown_key(inkstone::markdown_edit::Key::Indent, window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::OutdentInline, window, cx| {
                    this.markdown_key(inkstone::markdown_edit::Key::Outdent, window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::Backspace, window, cx| {
                    this.markdown_key(inkstone::markdown_edit::Key::Backspace, window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, action: &gpui_component::input::Undo, window, cx| {
                    if this.footnote_edit.is_some() {
                        return;
                    }
                    if let Some(owner) = &this.history_owner {
                        owner.update(cx, |state, cx| state.undo(action, window, cx));
                        cx.stop_propagation();
                    } else if this.reading {
                        this.editor.update(cx, |s, cx| s.undo(action, window, cx));
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, action: &gpui_component::input::Redo, window, cx| {
                    if this.footnote_edit.is_some() {
                        return;
                    }
                    if let Some(owner) = &this.history_owner {
                        owner.update(cx, |state, cx| state.redo(action, window, cx));
                        cx.stop_propagation();
                    } else if this.reading {
                        this.editor.update(cx, |s, cx| s.redo(action, window, cx));
                        cx.stop_propagation();
                    }
                },
            ))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(rgb(if self.light { 0xffffff } else { 0x262626 }))
            .text_color(rgb(if self.light { 0x222222 } else { 0xdadada }))
            .px(px(32.))
            .pt(px(20.))
            .children(self.footnote_panel(_window, cx))
            .child(
                div().flex().justify_center().flex_1().min_h_0().child(
                    div()
                        .w_full()
                        .h_full()
                        .min_w_0()
                        .when(self.readable_width, |s| s.max_w(px(700.)))
                        .when(self.reading, |s| s.child(preview))
                        .when(!self.reading, |s| {
                            s.child(
                                Editor::new(&self.editor)
                                    .on_paste(move |item, _, cx| {
                                        for entry in &item.entries {
                                            match entry {
                                                ClipboardEntry::ExternalPaths(paths) => {
                                                    let _ = paste_weak.update(cx, |_, cx| {
                                                        cx.emit(EditorEvent::PasteFiles(
                                                            paths.paths().to_vec(),
                                                        ))
                                                    });
                                                    return true;
                                                }
                                                ClipboardEntry::Image(image) => {
                                                    let _ = paste_weak.update(cx, |_, cx| {
                                                        cx.emit(EditorEvent::PasteImage(
                                                            format!(
                                                                "粘贴图片.{}",
                                                                image.format().extension()
                                                            ),
                                                            image.bytes().to_vec(),
                                                        ))
                                                    });
                                                    return true;
                                                }
                                                _ => (),
                                            }
                                        }
                                        false
                                    })
                                    .appearance(false)
                                    .bordered(false)
                                    .font_family(self.text_font.clone())
                                    .h_full()
                                    .text_size(px(self.font_size)),
                            )
                        }),
                ),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn reading_position_restores_after_parse_and_explicit_jump_wins(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = (0..60)
            .map(|i| format!("Paragraph {i} 中文内容"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.reading = true;
                p.restore_reading_position(
                    inkstone::preferences::ReadingPosition {
                        block: 20,
                        offset: 7.,
                    },
                    cx,
                );
                assert_eq!(p.reading_position(cx).block, 20);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for _ in 0..5 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, window, cx| {
                assert!(p.pending_reading_position.is_none());
                assert_eq!(p.reading_position(cx).block, 20);
                assert!((p.reading_position(cx).offset - 7.).abs() < 1.);
                p.restore_reading_position(
                    inkstone::preferences::ReadingPosition {
                        block: 30,
                        offset: 0.,
                    },
                    cx,
                );
                p.jump(0, window, cx);
            })
            .unwrap();
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.reading_position(cx).block, 0)
            })
            .unwrap();
    }
    #[gpui::test]
    fn committed_wiki_prefix_opens_attachment_completion(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.set_paths(Arc::new(vec!["图示.svg".into()]));
                p.editor
                    .update(cx, |s, cx| s.replace_text_in_range(None, "![[图", w, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                let state = p.editor.read(cx);
                let menu = state.completion_menu_state();
                assert!(menu.open);
                assert_eq!(menu.items.len(), 1);
                assert_eq!(menu.items[0].label, "图示.svg");
                assert!(menu.query.is_empty());
            })
            .unwrap();
    }
    #[gpui::test]
    fn footnote_numbers_follow_references_and_map_exact_markers(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source =
            "before[^b] middle[^a] again[^b]\n\n[^a]: Alpha\n\n[^b]: Beta\n\n    - item\n\nlast";
        let state = cx.new(|cx| TextViewState::markdown(source, cx));
        cx.run_until_parked();
        state.read_with(cx, |s, _| {
            let text = s.rendered_text();
            let text_ref = text.as_str();
            assert!(text_ref.starts_with("before1 middle2 again1"), "{text_ref}");
            assert!(text_ref.find("last").unwrap() < text_ref.find("Beta").unwrap());
            assert!(text_ref.find("Beta").unwrap() < text_ref.find("Alpha").unwrap());
            assert!(text_ref.contains("item"));
            let at = text
                .position_for_source_offset(source.find("again").unwrap() + 5)
                .unwrap();
            assert!(text_ref[at..].starts_with('1'), "{}", &text_ref[at..]);
        });
        let unused = cx.new(|cx| TextViewState::markdown("[^unused]: x[^b]\n\n[^b]: Secret", cx));
        cx.run_until_parked();
        unused.read_with(cx, |s, _| {
            assert!(!s.rendered_text().as_str().contains("Secret"))
        });
        let nested = cx.new(|cx| {
            TextViewState::markdown(
                "[^a] first\n\n[^a]: A[^c]\n\nlater[^b]\n\n[^b]: B\n\n[^c]: C",
                cx,
            )
        });
        cx.run_until_parked();
        nested.read_with(cx, |s, _| {
            let text = s.rendered_text();
            assert!(text.as_str().contains("later2"), "{}", text.as_str());
            assert!(text.as_str().contains("A3"));
        });
        let streamed = cx.new(|cx| TextViewState::markdown("body[^a]\n\n[^a]: A", cx));
        cx.run_until_parked();
        streamed.update(cx, |s, cx| s.push_str("\n\nnew[^b]\n\n[^b]: B", cx));
        cx.run_until_parked();
        streamed.read_with(cx, |s, _| {
            let text = s.rendered_text();
            assert_eq!(text.as_str().matches("body1").count(), 1);
            assert_eq!(text.as_str().matches("new2").count(), 1);
            assert!(text.as_str().find("new2").unwrap() < text.as_str().find("1. A").unwrap());
        });
    }
    #[gpui::test]
    fn footnote_links_scroll_to_definition_and_back(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!(
            "[^note] Intro\n\n{}\n[^note]: X",
            "paragraph\n\n".repeat(60)
        );
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.reading = true;
                p.update_presentation(cx);
                p.focus_view(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let bounds = handle
            .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
            .unwrap();
        visual.simulate_click(bounds.origin + point(px(3.), px(12.)), Modifiers::default());
        for _ in 0..5 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let footer = handle
            .update(&mut visual, |p, _, cx| {
                assert!(p.preview.read(cx).scroll_position().item_ix > 0);
                let offset = p.rendered.markdown.rfind("[^").unwrap();
                p.preview.read(cx).bounds_for_source_offset(offset).unwrap()
            })
            .unwrap();
        visual.simulate_click(
            point(footer.left() + px(40.), footer.bottom() - px(12.)),
            Modifiers::default(),
        );
        for _ in 0..5 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.preview.read(cx).scroll_position().item_ix, 0);
                assert_eq!(p.editor.read(cx).value(), source);
                assert!(p.reading);
            })
            .unwrap();
    }
    #[gpui::test]
    fn callout_state_survives_a_new_view_instance(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> [!note]- Restore\n> - [ ] task\n\nend";
        let first = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        first
            .update(cx, |p, _, cx| {
                p.reading = true;
                cx.notify();
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(first.into(), cx);
        visual.simulate_resize(size(px(900.), px(650.)));
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let bounds = first
            .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
            .unwrap();
        visual.simulate_click(
            point(bounds.right() - px(21.), bounds.top() + px(24.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        let saved = first
            .update(&mut visual, |p, _, cx| p.callout_states(cx))
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved.values().next(), Some(&false));
        let second = visual.add_window(|w, cx| {
            let mut p = EditorPane::new(source, w, cx);
            p.reading = true;
            p.restore_callout_states(&saved, cx);
            p
        });
        let mut restored = VisualTestContext::from_window(second.into(), &visual);
        restored.simulate_resize(size(px(900.), px(650.)));
        for _ in 0..4 {
            restored.run_until_parked();
            restored.update(|w, cx| w.draw(cx).clear(cx));
        }
        let bounds = second
            .update(&mut restored, |p, _, cx| p.reading_bounds(cx))
            .unwrap();
        restored.simulate_click(
            bounds.origin + point(px(32.), px(56.)),
            Modifiers::default(),
        );
        restored.run_until_parked();
        second
            .update(&mut restored, |p, _, cx| {
                assert!(p.editor.read(cx).value().contains("- [x] task"))
            })
            .unwrap();
    }

    #[gpui::test]
    fn callout_fold_and_task_click_preserve_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> [!warning]- **注意😀**\n> - [ ] body\n\nend";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.reading = true;
                p.update_presentation(cx);
                p.focus_view(w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(650.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let bounds = handle
            .update(&mut visual, |p, _, cx| {
                let rendered = p.preview.read(cx).rendered_text();
                assert!(rendered.as_str().contains("注意😀"));
                assert!(rendered.as_str().contains("body"));
                assert!(!rendered.as_str().contains("[!warning]"));
                p.preview.update(cx, |s, cx| {
                    s.set_selection_format(gpui_base::text::SelectionFormat::Source, cx);
                    s.select_all(cx);
                    assert!(s.selected_text().contains("[!warning]- **注意😀**"));
                    s.clear_selection(cx);
                });
                p.reading_bounds(cx)
            })
            .unwrap();
        visual.simulate_click(
            bounds.origin + point(px(32.), px(56.)),
            Modifiers::default(),
        );
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), source)
            })
            .unwrap();
        visual.simulate_click(
            point(bounds.right() - px(21.), bounds.top() + px(24.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_click(
            bounds.origin + point(px(32.), px(56.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        handle
            .update(&mut visual, |p, _, cx| {
                assert!(p.editor.read(cx).value().contains("- [x] body"))
            })
            .unwrap();
        handle
            .update(&mut visual, |p, w, cx| {
                p.reading = false;
                p.focus_view(w, cx);
                cx.notify();
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, w, cx| {
                p.reading = true;
                p.focus_view(w, cx);
                cx.notify();
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_click(
            bounds.origin + point(px(32.), px(56.)),
            Modifiers::default(),
        );
        visual.run_until_parked();
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), source)
            })
            .unwrap();
    }

    #[gpui::test]
    fn callout_parsing_handles_default_nested_and_lazy_content(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, required) in [
            ("> [!note]\n> body", "Note"),
            ("> [!tip] Title\nlazy body", "lazy body"),
            ("> [!note] Outer\n> > [!tip] Inner\n> > text", "Inner"),
            ("> [!danger] Hello", "Hello"),
            ("> [!note] ---", "---"),
            ("> [!note] # title", "# title"),
        ] {
            let state = cx.new(|cx| TextViewState::markdown(source, cx));
            cx.run_until_parked();
            state.read_with(cx, |s, _| {
                let rendered = s.rendered_text();
                assert!(
                    rendered.as_str().contains(required),
                    "{}",
                    rendered.as_str()
                );
                assert!(!rendered.as_str().contains("[!"));
            });
        }
        let source = "```\n> [!note] literal\n```";
        let state = cx.new(|cx| TextViewState::markdown(source, cx));
        cx.run_until_parked();
        state.read_with(cx, |s, _| {
            assert!(s.rendered_text().as_str().contains("[!note]"))
        });
    }

    #[gpui::test]
    fn callout_title_link_keeps_original_reference(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for source in [
            "> [!note] [go](other.md)\n> body",
            "> [!note] [go][ref]\n> body\n\n[ref]: other.md",
            "> [!note] [go][ref]\n> body\n\n[ref]: other.md\n[ref]: wrong.md",
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let received = Rc::new(RefCell::new(Vec::new()));
            let capture = received.clone();
            let _subscription = handle
                .update(cx, |p, w, cx| {
                    p.reading = true;
                    p.update_presentation(cx);
                    p.focus_view(w, cx);
                    cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                        if let EditorEvent::FollowReference(r) = event {
                            capture.borrow_mut().push(r.clone());
                        }
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.simulate_resize(size(px(900.), px(650.)));
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            let bounds = handle
                .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
                .unwrap();
            visual.simulate_click(
                bounds.origin + point(px(52.), px(24.)),
                Modifiers::default(),
            );
            let diagnostic = handle
                .update(&mut visual, |p, _, cx| {
                    format!(
                        "{} => {}",
                        p.rendered.markdown,
                        p.preview.read(cx).rendered_text().as_str()
                    )
                })
                .unwrap();
            assert_eq!(received.borrow().len(), 1, "{source}; {diagnostic}");
            assert_eq!(received.borrow()[0].target, "other.md");
        }
    }
    #[gpui::test]
    fn reading_jump_expands_callout_body(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> [!note]- Header\n> [body](other.md)";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let received = Rc::new(std::cell::Cell::new(false));
        let capture = received.clone();
        let _subscription = handle
            .update(cx, |p, w, cx| {
                p.reading = true;
                p.update_presentation(cx);
                p.focus_view(w, cx);
                cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                    if matches!(event, EditorEvent::FollowReference(_)) {
                        capture.set(true);
                    }
                })
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(650.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, w, cx| {
                p.jump(source.find("[body]").unwrap(), w, cx)
            })
            .unwrap();
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let bounds = handle
            .update(&mut visual, |p, _, cx| p.reading_bounds(cx))
            .unwrap();
        visual.simulate_click(
            bounds.origin + point(px(32.), px(56.)),
            Modifiers::default(),
        );
        assert!(received.get());
    }
    #[gpui::test]
    fn table_tab_navigation_adds_rows_and_undo_preserves_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "| a | b |\n| --- | --- |\n| c | d |";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(&s.value()[s.selected_range()], "b")
        });
        visual.simulate_keystrokes("shift-tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(&s.value()[s.selected_range()], "a")
        });
        let end = source.rfind('d').unwrap() + 1;
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(end..end, cx))
            })
            .unwrap();
        visual.simulate_keystrokes("tab");
        editor.read_with(&visual, |s, _| assert!(s.value().ends_with("\n|  |  |")));
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), end..end);
        });
    }
    #[gpui::test]
    fn folding_moves_the_caret_and_search_and_jump_reveal_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# 第一章\n正文\n## 子标题\n隐藏目标😀\n# 第二章\n末尾";
        let second = source.find("# 第二章").unwrap();
        let hidden = source.find("隐藏").unwrap();
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(hidden..hidden, cx));
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let before = editor.read_with(&visual, |s, _| {
            s.range_to_bounds(&(second..second)).unwrap().top()
        });
        handle
            .update(&mut visual, |p, w, cx| p.fold_sections(Some(true), w, cx))
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        editor.read_with(&visual, |s, _| {
            assert!(!s.folded_ranges().is_empty());
            assert!(s.selected_range().start < hidden);
            assert!(s.range_to_bounds(&(second..second)).unwrap().top() < before);
            assert_eq!(s.value(), source);
        });
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_search_query("隐藏目标", false, cx);
                    assert!(s.next_search_match(cx).is_some());
                    assert!(
                        s.folded_ranges()
                            .iter()
                            .all(|f| !(f.start_line < 3 && 3 < f.end_line))
                    );
                });
            })
            .unwrap();
        handle
            .update(&mut visual, |p, w, cx| {
                p.fold_sections(Some(true), w, cx);
                p.jump(hidden, w, cx);
                assert_eq!(p.editor.read(cx).selected_range(), hidden..hidden);
                assert!(
                    p.editor
                        .read(cx)
                        .folded_ranges()
                        .iter()
                        .all(|f| !(f.start_line < 3 && 3 < f.end_line))
                );
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-a ctrl-c");
        visual
            .update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
        handle
            .update(&mut visual, |p, w, cx| {
                p.fold_sections(Some(true), w, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(0..0, cx);
                    s.replace("\n", w, cx);
                });
                p.update_presentation(cx);
            })
            .unwrap();
        visual.run_until_parked();
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), format!("\n{source}"));
            assert!(s.folded_ranges().iter().any(|f| f.start_line == 5));
        });
    }

    #[gpui::test]
    fn folding_gutter_uses_heading_heights_and_does_not_edit_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# 大标题\nbody\n## 次标题\nhidden\n# 结尾\nlast";
        let target = source.find("##").unwrap();
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(target..target, cx));
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let row = editor.read_with(&visual, |s, _| {
            s.range_to_bounds(&(target..target)).unwrap()
        });
        visual.simulate_click(
            point(row.left() - px(20.), row.top() + row.size.height.half()),
            Modifiers::default(),
        );
        editor.read_with(&visual, |s, _| {
            assert!(s.folded_ranges().iter().any(|f| f.start_line == 2));
            assert_eq!(s.value(), source);
        });
    }
    #[gpui::test]
    fn triple_backticks_insert_a_closing_fence_and_preserve_context(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                for (prefix, continuation) in [
                    ("", ""),
                    ("  ", "  "),
                    ("> ", "> "),
                    ("- ", "  "),
                    ("> 12. ", ">     "),
                    ("- [ ] ", "  "),
                ] {
                    p.editor.update(cx, |s, cx| {
                        s.set_value(prefix, w, cx);
                        s.set_selected_range(prefix.len()..prefix.len(), cx);
                        for _ in 0..3 {
                            s.replace_text_in_range(None, "`", w, cx);
                        }
                        let expected = format!("{prefix}```\n{continuation}```");
                        assert_eq!(s.value(), expected);
                        assert_eq!(s.selected_range(), prefix.len() + 3..prefix.len() + 3);
                        let end = expected.len() - 3;
                        s.set_selected_range(end..end, cx);
                        s.replace_text_in_range(None, "`", w, cx);
                        assert_eq!(s.value(), format!("{expected}`"));
                        assert_eq!(s.selected_range(), end + 1..end + 1);
                    });
                }
                p.editor.update(cx, |s, cx| {
                    s.set_value("before\r\n", w, cx);
                    s.set_selected_range(8..8, cx);
                    for _ in 0..3 {
                        s.replace_text_in_range(None, "`", w, cx);
                    }
                    assert_eq!(s.value(), "before\r\n```\r\n```");
                    s.focus(w, cx);
                });
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), "before\r\n``")
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-y");
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), "before\r\n```\r\n```");
                assert_eq!(p.editor.read(cx).selected_range(), 11..11);
            })
            .unwrap();
    }

    #[gpui::test]
    fn synchronized_text_keeps_selection_on_grapheme_boundaries(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("a", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(1..1, cx);
                    assert!(s.apply_synced_text(
                        "a\u{301}",
                        &[(1..1, "\u{301}".into())],
                        gpui_base::input::SyncedHistory::Ignore,
                        false,
                        w,
                        cx
                    ));
                    assert_eq!(s.selected_range(), 0..0);
                    assert!(!s.apply_synced_text(
                        "wrong",
                        &[(1..2, "bad".into())],
                        gpui_base::input::SyncedHistory::Ignore,
                        false,
                        w,
                        cx
                    ));
                    assert_eq!(s.value(), "a\u{301}");
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn large_counts_are_deferred_and_old_requests_do_not_replace_new_selection(
        cx: &mut TestAppContext,
    ) {
        use inkstone::word_count::Counts;
        cx.update(gpui_kit::init);
        let long = "word ".repeat(3000);
        let handle = cx.add_window(|w, cx| EditorPane::new(&long, w, cx));
        handle
            .update(cx, |p, w, cx| {
                assert_eq!(p.text_counts(cx), Counts::default());
                p.editor.update(cx, |s, cx| s.set_value("short", w, cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 5
                    }
                );
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        handle
            .update(cx, |p, w, cx| {
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 5
                    }
                );
                p.editor
                    .update(cx, |s, cx| s.set_value(long.clone(), w, cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 5
                    }
                );
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 3000,
                        characters: 15000
                    }
                );
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(0..10000, cx));
                p.text_counts(cx);
                p.editor.update(cx, |s, cx| s.set_selected_range(0..4, cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 4
                    }
                );
            })
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(250));
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 4
                    }
                )
            })
            .unwrap();
    }

    #[gpui::test]
    fn text_counts_track_selection_reading_mode_and_source_changes(cx: &mut TestAppContext) {
        use inkstone::word_count::Counts;
        cx.update(gpui_kit::init);
        let source = "---\ntitle: Secret\n---\n你好 world😀";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 3,
                        characters: 10
                    }
                );
                let emoji = source.find('😀').unwrap();
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(emoji..source.len(), cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 0,
                        characters: 2
                    }
                );
                let secret = source.find("Secret").unwrap();
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(secret..secret + 6, cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 1,
                        characters: 6
                    }
                );
                p.reading = true;
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 3,
                        characters: 10
                    }
                );
                p.editor
                    .update(cx, |s, cx| s.set_value(format!("{source} ok"), w, cx));
                assert_eq!(
                    p.text_counts(cx),
                    Counts {
                        words: 4,
                        characters: 13
                    }
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiple_empty_ordered_items_exit_and_undo_as_one_change(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "7. \n8. \n9. text";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(3..3, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "\n\n7. text");
            assert!(s.has_multiple_selections());
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert!(s.has_multiple_selections());
        });
    }

    #[gpui::test]
    fn multiple_carets_continue_numbered_lists_and_undo_together(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "8. a\n9. b";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(4..4, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down enter");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), "8. a\n9. \n10. b\n11. ");
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), source);
        });
        visual.simulate_keystrokes("ctrl-y");
        visual.simulate_input("中");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "8. a\n9. 中\n10. b\n11. 中")
        });
    }

    #[gpui::test]
    fn multiple_carets_soft_break_together_and_restore_on_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "- 中文\n- 中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(8..8, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down shift-enter");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), "- 中文\n  \n- 中文\n  ");
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), source);
        });
        visual.simulate_keystrokes("ctrl-y");
        visual.simulate_input("x");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- 中文\n  x\n- 中文\n  x")
        });
    }

    #[gpui::test]
    fn shift_enter_replaces_multiline_selection_with_one_undo_step(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "    中文\r\n    tail";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(4..source.len(), cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("shift-enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "    \r\n");
            assert_eq!(s.selected_range(), 6..6);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 4..source.len());
        });
    }

    #[gpui::test]
    fn home_and_shift_home_respect_markdown_prefixes_and_soft_wrap(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> - [!] 中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.live = false;
                p.set_auto_pairing(false, false, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("home");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 8..8));
        visual.simulate_keystrokes("home");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 0..0));
        visual.simulate_keystrokes("end shift-home");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.selected_range(), 8..source.len())
        });
        visual.simulate_keystrokes("shift-home");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.selected_range(), 0..source.len())
        });
        let long = format!("> - [!] {}", "长文本".repeat(80));
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.set_value(long.clone(), w, cx);
                    s.set_selected_range(long.len()..long.len(), cx);
                })
            })
            .unwrap();
        visual.simulate_resize(size(px(300.), px(400.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("home");
        editor.read_with(&visual, |s, _| assert!(s.selected_range().start > 8));
        visual.simulate_keystrokes("home");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 8..8));
        visual.simulate_keystrokes("home");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.selected_range(), 0..0);
            assert_eq!(s.value(), long);
        });
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.set_value("> - [!] a\n> - [!] b", w, cx);
                    s.set_selected_range(9..9, cx);
                })
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down home");
        visual.simulate_input("X");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), "> - [!] Xa\n> - [!] Xb");
        });
    }

    #[gpui::test]
    fn custom_tasks_render_as_checkboxes_and_click_preserves_source_mapping(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let source = "- [✓] **中文**\n- [!] second";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.reading = true;
                p.update_presentation(cx);
                p.focus_view(w, cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(700.), px(400.)));
        for _ in 0..3 {
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
        }
        let bounds = handle
            .update(&mut visual, |p, _, cx| {
                let text = p.preview.read(cx).rendered_text();
                assert!(text.as_str().contains("中文"));
                assert!(!text.as_str().contains("[✓]"));
                assert!(!text.as_str().contains("[!]"));
                assert_eq!(
                    text.position_for_source_offset(source.find("中文").unwrap()),
                    text.as_str().find("中文")
                );
                p.preview.update(cx, |s, cx| {
                    s.set_selection_format(gpui_base::text::SelectionFormat::Source, cx);
                    s.select_all(cx);
                    assert_eq!(s.selected_text(), source);
                    s.clear_selection(cx);
                });
                p.preview.read(cx).bounds_for_source_offset(0).unwrap()
            })
            .unwrap();
        visual.simulate_click(bounds.origin + point(px(7.), px(12.)), Modifiers::default());
        handle
            .update(&mut visual, |p, w, cx| {
                assert_eq!(p.editor.read(cx).value(), "- [ ] **中文**\n- [!] second");
                assert_eq!(
                    p.editor.read(cx).selected_range(),
                    source.len() - 2..source.len() - 2
                );
                p.editor
                    .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
                assert_eq!(p.editor.read(cx).value(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn enter_after_soft_list_continuation_resumes_marker_and_undoes_once(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("- 中文", w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(8..8, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("shift-enter");
        visual.simulate_input("续行");
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- 中文\n  续行\n- ");
            assert_eq!(s.selected_range(), 20..20);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n  续行"));
    }

    #[gpui::test]
    fn reading_soft_breaks_follow_strict_setting_without_changing_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "第一行\r\n第二行\n\nhard  \nbreak";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.reading = true;
                p.update_presentation(cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(700.), px(400.)));
        let mut soft_height = None;
        for strict in [false, true, false] {
            handle
                .update(&mut visual, |p, _, cx| {
                    p.strict_line_breaks = strict;
                    cx.notify();
                })
                .unwrap();
            for _ in 0..3 {
                visual.run_until_parked();
                visual.update(|w, cx| w.draw(cx).clear(cx));
            }
            handle
                .update(&mut visual, |p, _, cx| {
                    let preview = p.preview.read(cx);
                    let text = preview.rendered_text();
                    assert!(
                        text.as_str().contains(if strict {
                            "第一行 第二行"
                        } else {
                            "第一行\n第二行"
                        }),
                        "{}",
                        text.as_str()
                    );
                    assert!(text.as_str().contains("hard\nbreak"));
                    let first = preview.bounds_for_source_offset(0).unwrap();
                    assert_eq!(
                        text.position_for_source_offset(source.find("第二行").unwrap()),
                        text.as_str().find("第二行")
                    );
                    if strict {
                        assert!(first.size.height < soft_height.unwrap());
                    } else {
                        soft_height = Some(first.size.height);
                    }
                    assert_eq!(p.editor.read(cx).value(), source);
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn fold_options_remove_disabled_ranges_and_keep_other_folds(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# Heading\ntext\n- parent\n  - child\n```\ncode\n```";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.fold_sections(Some(true), w, cx);
                assert_eq!(
                    p.editor
                        .read(cx)
                        .folded_ranges()
                        .iter()
                        .map(|f| f.start_line)
                        .collect::<Vec<_>>(),
                    vec![0, 2, 4]
                );
                p.set_fold_options(false, true, w, cx);
                assert_eq!(
                    p.editor
                        .read(cx)
                        .folded_ranges()
                        .iter()
                        .map(|f| f.start_line)
                        .collect::<Vec<_>>(),
                    vec![2, 4]
                );
                p.set_fold_options(false, false, w, cx);
                assert!(p.editor.read(cx).folded_ranges().is_empty());
                p.fold_sections(Some(true), w, cx);
                assert!(p.editor.read(cx).folded_ranges().is_empty());
                p.set_fold_options(true, false, w, cx);
                p.fold_sections(Some(true), w, cx);
                assert_eq!(
                    p.editor
                        .read(cx)
                        .folded_ranges()
                        .iter()
                        .map(|f| f.start_line)
                        .collect::<Vec<_>>(),
                    vec![0, 4]
                );
                assert_eq!(p.editor.read(cx).value(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiple_carets_indent_quote_lines_once_and_undo_together(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> 中文\n> 中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(5..5, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down tab");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), "> \t中文\n> \t中文");
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), source);
        });
        visual.simulate_keystrokes("ctrl-y shift-tab");
        editor.read_with(&visual, |s, _| {
            assert!(s.has_multiple_selections());
            assert_eq!(s.value(), source);
        });
        editor.update(&mut visual, |s, cx| s.set_selected_range(2..2, cx));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let at = editor.read_with(&visual, |s, _| s.range_to_bounds(&(5..5)).unwrap().origin);
        visual.simulate_click(
            at + point(px(1.), px(8.)),
            Modifiers {
                alt: true,
                ..Default::default()
            },
        );
        editor.read_with(&visual, |s, _| assert!(s.has_multiple_selections()));
        visual.simulate_keystrokes("tab");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "> \t中文\n> 中文"));
    }

    #[gpui::test]
    fn tab_indents_the_line_from_the_middle_of_plain_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> 中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(5..5, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "> \t中文");
            assert_eq!(s.selected_range(), 6..6);
        });
        visual.simulate_keystrokes("shift-tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 5..5);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "> \t中文"));
    }

    #[gpui::test]
    fn exiting_an_ordered_item_restores_numbers_and_caret_on_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "1. 中文\n2. \n3. following";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(13..13, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "1. 中文\n\n2. following");
            assert_eq!(s.selected_range(), 10..10);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 13..13);
        });
    }

    #[gpui::test]
    fn ordered_list_enter_and_renumber_are_one_undoable_edit(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "9. 中文\n10. following\n11. end";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(9..9, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "9. 中文\n10. \n11. following\n12. end");
            assert_eq!(s.selected_range(), 14..14);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 9..9);
        });
        visual.simulate_keystrokes("ctrl-y");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "9. 中文\n10. \n11. following\n12. end")
        });
    }

    #[gpui::test]
    fn smart_lists_toggle_enter_but_keep_tab_and_soft_continuation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "- 中文";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("shift-enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n  "));
        visual.simulate_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |p, _, _| p.smart_lists = false)
            .unwrap();
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- 中文\n"));
        visual.simulate_keystrokes("ctrl-z tab");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "\t- 中文"));
    }

    #[gpui::test]
    fn select_all_occurrences_keeps_adjacent_ranges_and_undoes_once(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, selection, expected) in [
            ("cat scatter cat Cat", 12..15, "x sxter x Cat"),
            ("中文中文", 0..6, "xx"),
            ("e\u{301}e\u{301}", 0..3, "xx"),
            ("😀😀", 0..4, "xx"),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(selection.clone(), cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes("ctrl-shift-l");
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        assert!(s.has_multiple_selections());
                        assert_eq!(s.selected_range(), selection);
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(s.value(), expected);
                    });
                })
                .unwrap();
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), source);
                assert_eq!(s.selected_range(), selection);
                assert!(s.has_multiple_selections());
            });
        }
    }

    #[gpui::test]
    fn select_all_occurrences_respects_empty_selection_and_reference_limit(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    for count in [1001, 1002] {
                        let source = "a ".repeat(count);
                        s.set_value(&source, w, cx);
                        s.set_selected_range(0..0, cx);
                        s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, w, cx);
                        assert_eq!(s.selected_range(), 0..0);
                        s.set_selected_range(0..1, cx);
                        s.select_all_occurrences(&gpui_base::input::SelectAllOccurrences, w, cx);
                        assert_eq!(s.has_multiple_selections(), count == 1001);
                        assert_eq!(s.value(), source);
                    }
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn ctrl_d_selects_words_wraps_and_edits_all_occurrences(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "cat scatter cat Cat cat";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(13..13, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-d");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 12..15));
        visual.simulate_keystrokes("ctrl-d ctrl-d ctrl-d");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "犬", w, cx);
                    assert_eq!(s.value(), "犬 scatter 犬 Cat 犬");
                });
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert!(s.has_multiple_selections());
        });
    }

    #[gpui::test]
    fn ctrl_d_preserves_unicode_words_and_explicit_substrings(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, selection, expected) in [
            ("中文 中文", 0..0, "x x"),
            ("e\u{301} e\u{301}", 0..0, "x x"),
            ("foobar barfoo", 3..6, "foox xfoo"),
            ("😀 😀", 0..4, "x x"),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(selection, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes("ctrl-d ctrl-d");
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(s.value(), expected);
                    });
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn selected_code_after_double_backticks_gets_fence_line_breaks(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, range, expected, selected) in [
            ("``中文``", 2..8, "```\n中文\n```", 4..10),
            ("``\n中文\n``", 2..10, "```\n中文\n```", 3..11),
            (
                "> ``中文``\r\n",
                4..10,
                "> ```\r\n> 中文\r\n> ```\r\n",
                9..15,
            ),
            ("``\r\n中文\r\n``", 2..12, "```\r\n中文\r\n```", 3..13),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.set_auto_pairing(true, true, cx);
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(range.clone(), cx);
                        s.focus(w, cx);
                        s.replace_text_in_range(None, "`", w, cx);
                        assert_eq!(s.value(), expected);
                        assert_eq!(s.selected_range(), selected);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), source);
                assert_eq!(s.selected_range(), range);
            });
            visual.simulate_keystrokes("ctrl-y");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), expected);
                assert_eq!(s.selected_range(), selected);
            });
        }
    }

    #[gpui::test]
    fn multiple_selected_code_ranges_become_fences_and_keep_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "``中``\n``文``";
        let expected = "```\n中\n```\n```\n文\n```";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(2..2, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down shift-right");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "`", w, cx);
                    assert_eq!(s.value(), expected);
                    assert!(s.has_multiple_selections());
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), "```\nx\n```\n```\nx\n```");
                });
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), expected));
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
    }

    #[gpui::test]
    fn multiple_carets_pair_symmetric_markers_and_code_fences(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, offset, typed, paired, inserted) in [
            ("\n", 0, "*", "**\n**", "*x*\n*x*"),
            ("\n", 0, "_", "__\n__", "_x_\n_x_"),
            ("\n", 0, "'", "''\n''", "'x'\n'x'"),
            ("\n", 0, "\"", "\"\"\n\"\"", "\"x\"\n\"x\""),
            ("\n", 0, "`", "``\n``", "`x`\n`x`"),
            (
                "``\n``",
                2,
                "`",
                "```\n```\n```\n```",
                "```x\n```\n```x\n```",
            ),
            (
                "> ``\r\n> ``",
                4,
                "`",
                "> ```\r\n> ```\r\n> ```\r\n> ```",
                "> ```x\r\n> ```\r\n> ```x\r\n> ```",
            ),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.set_auto_pairing(true, true, cx);
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(offset..offset, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes("ctrl-alt-down");
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        s.replace_text_in_range(None, typed, w, cx);
                        assert_eq!(s.value(), paired);
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(s.value(), inserted);
                    });
                })
                .unwrap();
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), paired));
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), source);
                assert!(s.has_multiple_selections());
            });
        }
    }

    #[gpui::test]
    fn symmetric_pairing_can_skip_and_insert_in_one_batch(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("\n", w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(0..0, cx);
                    s.replace_text_in_range(None, "*", w, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down");
        for _ in 0..2 {
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        s.replace_text_in_range(None, "*", w, cx);
                        assert_eq!(s.value(), "**\n**");
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(s.value(), "**x\n*x*");
                    });
                })
                .unwrap();
            visual.simulate_keystrokes("ctrl-z ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), "**\n"));
        }
    }

    #[gpui::test]
    fn multiple_carets_insert_and_skip_pairs_without_extra_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("\n", w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(0..0, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-alt-down");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "(", w, cx);
                    assert_eq!(s.value(), "()\n()");
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "()\n()");
                    s.replace_text_in_range(None, "x", w, cx);
                    assert_eq!(s.value(), "()x\n()x");
                });
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "()\n()"));
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "\n");
            assert!(s.has_multiple_selections());
        });
        visual.simulate_keystrokes("ctrl-y");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "()\n()");
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn mixed_carets_and_selections_pair_only_when_every_position_allows_it(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        for (source, select, enabled, typed, expected) in [
            ("中文\n", true, true, "(", "(中文)\n()"),
            ("\nx", false, true, "(", "(\n(x"),
            ("\n", false, false, "(", "(\n("),
            ("中文\n", true, true, "*", "*中文*\n**"),
            ("中文\n", true, true, "\"", "\"中文\"\n\"\""),
            ("x\n", false, true, "*", "*x\n*"),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.set_auto_pairing(enabled, enabled, cx);
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(0..0, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes("ctrl-alt-down");
            if select {
                visual.simulate_keystrokes("shift-end");
            }
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        s.replace_text_in_range(None, typed, w, cx);
                        assert_eq!(s.value(), expected);
                        assert!(s.has_multiple_selections());
                    });
                })
                .unwrap();
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
        }
    }

    #[gpui::test]
    fn multiple_selections_surround_preserve_direction_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for reversed in [false, true] {
            let source = "中文\n中文";
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.set_auto_pairing(true, true, cx);
                    p.editor.update(cx, |s, cx| {
                        let offset = if reversed { 6 } else { 0 };
                        s.set_selected_range(offset..offset, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes(if reversed {
                "ctrl-alt-down shift-home"
            } else {
                "ctrl-alt-down shift-end"
            });
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        assert!(s.has_multiple_selections());
                        s.replace_text_in_range(None, "*", w, cx);
                        assert_eq!(s.value(), "*中文*\n*中文*");
                        s.replace_text_in_range(None, "(", w, cx);
                        assert_eq!(s.value(), "*(中文)*\n*(中文)*");
                    });
                })
                .unwrap();
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), "*中文*\n*中文*"));
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
            visual.simulate_keystrokes("ctrl-y");
            if reversed {
                visual.simulate_keystrokes("shift-right");
            }
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        assert!(s.has_multiple_selections());
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(
                            s.value(),
                            if reversed {
                                "*中x*\n*中x*"
                            } else {
                                "*x*\n*x*"
                            }
                        );
                    });
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn multiline_selection_pairing_distinguishes_inline_markers_and_code(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("中\n文", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    for (typed, expected) in [("(", "()"), ("*", "**"), ("`", "`中\n文`")] {
                        s.set_value("中\n文", w, cx);
                        s.set_selected_range(0..7, cx);
                        s.replace_text_in_range(None, typed, w, cx);
                        assert_eq!(s.value(), expected);
                    }
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiple_carets_delete_pairs_together_and_restore_carets_on_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (source, offset, enabled, select, deleted, inserted) in [
            ("中()\n文[]", 4, true, false, "中\n文", "中(x)\n文[x]"),
            ("()\nab", 1, true, false, ")\nb", "(x)\naxb"),
            ("()\n[]", 1, false, false, ")\n]", "(x)\n[x]"),
            ("**\n__", 1, true, false, "\n", "*x*\n_x_"),
            ("()\n[]", 1, true, true, ")\n]", "x)\nx]"),
            ("\\()\n ()", 2, true, false, "\\)\n )", "\\(x)\n (x)"),
        ] {
            let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
            let editor = handle
                .update(cx, |p, w, cx| {
                    p.set_auto_pairing(enabled, enabled, cx);
                    p.editor.update(cx, |s, cx| {
                        s.set_selected_range(offset..offset, cx);
                        s.focus(w, cx);
                    });
                    p.editor.clone()
                })
                .unwrap();
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.simulate_keystrokes("ctrl-alt-down");
            if select {
                visual.simulate_keystrokes("shift-left");
            }
            visual.simulate_keystrokes("backspace");
            editor.read_with(&visual, |s, _| {
                assert_eq!(s.value(), deleted, "{source}");
                assert!(s.has_multiple_selections());
            });
            visual.simulate_keystrokes("ctrl-z");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
            visual.simulate_keystrokes("ctrl-y");
            editor.read_with(&visual, |s, _| assert_eq!(s.value(), deleted));
            visual.simulate_keystrokes("ctrl-z");
            handle
                .update(&mut visual, |_, w, cx| {
                    editor.update(cx, |s, cx| {
                        assert!(s.has_multiple_selections());
                        s.replace_text_in_range(None, "x", w, cx);
                        assert_eq!(s.value(), inserted);
                    });
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn pair_tracking_expires_when_keyboard_navigation_leaves_the_line(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("\nnext", w, cx));
        let editor = handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(0..0, cx);
                    s.replace_text_in_range(None, "(", w, cx);
                    s.focus(w, cx);
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("down up");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    assert_eq!(s.selected_range(), 1..1);
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "())\nnext");
                    s.set_value("", w, cx);
                    s.replace_text_in_range(None, "(", w, cx);
                });
            })
            .unwrap();
        visual.simulate_keystrokes("left right");
        handle
            .update(&mut visual, |_, w, cx| {
                editor.update(cx, |s, cx| {
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "()");
                    assert_eq!(s.selected_range(), 2..2);
                })
            })
            .unwrap();
    }

    #[gpui::test]
    fn paired_closers_skip_once_but_existing_text_is_never_swallowed(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    for closer in [")", "]", "}", "*", "_", "`", "\""] {
                        s.set_value(closer, w, cx);
                        s.set_selected_range(0..0, cx);
                        s.replace_text_in_range(None, closer, w, cx);
                        assert_eq!(s.value(), closer.repeat(2));
                    }
                    s.set_value("", w, cx);
                    s.replace_text_in_range(None, "(", w, cx);
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "()");
                    assert_eq!(s.selected_range(), 2..2);
                    s.set_selected_range(1..1, cx);
                    s.replace_text_in_range(None, ")", w, cx);
                    assert_eq!(s.value(), "())");
                    s.set_value("中文😀", w, cx);
                    s.set_selected_range(0..10, cx);
                    s.replace_text_in_range(None, "*", w, cx);
                    s.set_selected_range(11..11, cx);
                    s.replace_text_in_range(None, "*", w, cx);
                    assert_eq!(s.value(), "*中文😀*");
                    assert_eq!(s.selected_range(), 12..12);
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn automatic_pairs_are_independent_and_respect_escape_words_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        handle
            .update(cx, |p, w, cx| {
                for (brackets, markdown) in
                    [(true, true), (true, false), (false, true), (false, false)]
                {
                    p.set_auto_pairing(brackets, markdown, cx);
                    for (typed, paired, enabled) in [
                        ("(", "()", brackets),
                        ("[", "[]", brackets),
                        ("\"", "\"\"", brackets),
                        ("*", "**", markdown),
                        ("_", "__", markdown),
                        ("`", "``", markdown),
                    ] {
                        p.editor.update(cx, |s, cx| {
                            s.set_value("", w, cx);
                            s.replace_text_in_range(None, typed, w, cx);
                            assert_eq!(s.value(), if enabled { paired } else { typed });
                            assert_eq!(s.selected_range(), 1..1);
                        });
                    }
                }
                p.set_auto_pairing(true, true, cx);
                p.editor.update(cx, |s, cx| {
                    s.set_value("中文😀", w, cx);
                    s.set_selected_range(0..10, cx);
                    s.replace_text_in_range(None, "*", w, cx);
                    assert_eq!(s.value(), "*中文😀*");
                    assert_eq!(s.selected_range(), 1..11);
                });
                for source in ["word", "\\"] {
                    p.editor.update(cx, |s, cx| {
                        s.set_value(source, w, cx);
                        s.set_selected_range(source.len()..source.len(), cx);
                        s.replace_text_in_range(None, "*", w, cx);
                        assert_eq!(s.value(), format!("{source}*"));
                    });
                }
                let other = cx.new(|cx| EditorPane::new("", w, cx));
                other.update(cx, |other, cx| other.set_auto_pairing(false, false, cx));
                p.editor.update(cx, |s, cx| {
                    s.set_value("", w, cx);
                    s.replace_text_in_range(None, "(", w, cx);
                    assert_eq!(s.value(), "()");
                    s.focus(w, cx);
                });
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_keystrokes("backspace");
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), "")
            })
            .unwrap();
        visual.simulate_keystrokes("ctrl-z");
        handle
            .update(&mut visual, |p, _, cx| {
                assert_eq!(p.editor.read(cx).value(), "()")
            })
            .unwrap();
    }

    #[gpui::test]
    fn markdown_keys_continue_indent_and_undo_as_single_edits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "- [x] 中文👩‍💻";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
        });
        visual.simulate_keystrokes("tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- [x] 中文👩‍💻\n\t- [ ] ")
        });
        visual.simulate_keystrokes("shift-tab");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
        });
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "- [x] 中文👩‍💻\n"));
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ");
            assert_eq!(s.selected_range(), s.value().len()..s.value().len());
        });
        visual.simulate_keystrokes("ctrl-z ctrl-z ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), source.len()..source.len());
        });
        visual.simulate_keystrokes("ctrl-y");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "- [x] 中文👩‍💻\n- [ ] ")
        });
    }

    #[gpui::test]
    fn configured_tabs_apply_to_plain_text_and_markdown_lists(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("", w, cx));
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for (width, hard_tabs) in [(2, false), (6, false), (4, true)] {
            for source in ["plain", "- 中文😀"] {
                let editor = handle
                    .update(&mut visual, |p, w, cx| {
                        p.indentation = gpui_base::input::TabSize {
                            tab_size: width,
                            hard_tabs,
                        };
                        p.editor.update(cx, |s, cx| {
                            s.set_tab_size(p.indentation, cx);
                            s.set_value(source, w, cx);
                            s.set_selected_range(0..0, cx);
                            s.focus(w, cx);
                        });
                        p.editor.clone()
                    })
                    .unwrap();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                visual.simulate_keystrokes("tab");
                let indent = if hard_tabs {
                    "\t".into()
                } else {
                    " ".repeat(width)
                };
                editor.read_with(&visual, |s, _| {
                    assert_eq!(s.value(), format!("{indent}{source}"))
                });
                visual.simulate_keystrokes("ctrl-z");
                editor.read_with(&visual, |s, _| assert_eq!(s.value(), source));
            }
        }
    }

    #[gpui::test]
    fn markdown_keys_respect_literal_blocks_completion_and_composition(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "```\n- code\n```";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(10..10, cx));
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("enter");
        editor.read_with(&visual, |s, _| assert_eq!(s.value(), "```\n- code\n\n```"));
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_value("- 中文", w, cx);
                    s.set_selected_range(8..8, cx);
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
                });
                let before = p.editor.read(cx).value();
                p.markdown_key(inkstone::markdown_edit::Key::Enter, w, cx);
                assert_eq!(p.editor.read(cx).value(), before);
                p.editor.update(cx, |s, cx| {
                    s.unmark_text(w, cx);
                    s.present_completion_items(
                        2,
                        "",
                        vec![lsp_types::CompletionItem {
                            label: "候选".into(),
                            ..Default::default()
                        }],
                        cx,
                    );
                });
                p.markdown_key(inkstone::markdown_edit::Key::Enter, w, cx);
                assert_eq!(p.editor.read(cx).value(), before);
            })
            .unwrap();
    }

    #[gpui::test]
    fn markdown_prefix_backspace_restores_caret_on_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "> - [ ] 中文👩‍💻";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.live = false;
                p.editor.update(cx, |s, cx| s.set_selected_range(8..8, cx));
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("backspace");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), "> 中文👩‍💻");
            assert_eq!(s.selected_range(), 2..2);
        });
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), source);
            assert_eq!(s.selected_range(), 8..8);
        });
        visual.simulate_keystrokes("ctrl-end shift-enter");
        editor.read_with(&visual, |s, _| {
            assert_eq!(s.value(), format!("{source}\n>       "))
        });
    }
    #[gpui::test]
    fn wrapped_headings_reflow_and_keep_the_following_caret_visible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = format!("# {}\nbody", "长标题😀".repeat(35));
        let end = source.len();
        let handle = cx.add_window(|w, cx| EditorPane::new(&source, w, cx));
        let state = handle
            .update(cx, |p, _, cx| {
                p.editor
                    .update(cx, |s, cx| s.set_selected_range(end..end, cx));
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for width in [700., 280., 900., 330.] {
            visual.simulate_resize(size(px(width), px(320.)));
            visual.update(|w, cx| w.draw(cx).clear(cx));
            visual.run_until_parked();
            visual.update(|w, cx| w.draw(cx).clear(cx));
            state.read_with(&visual, |s, _| {
                assert_eq!(s.selected_range(), end..end);
                assert_eq!(s.value().as_ref(), source);
                let (mut caret, _) = s.cursor_layout().unwrap();
                caret.origin += s.scroll_offset();
                assert!(
                    s.input_bounds().intersects(&caret),
                    "width={width}, caret={caret:?}, viewport={:?}",
                    s.input_bounds()
                );
            });
        }
    }
    #[gpui::test]
    fn live_heading_font_height_hit_testing_and_ime_agree(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# Heading\nbody\n## Sub\nlast";
        let body = source.find("body").unwrap();
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        let editor = handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.update_presentation(cx);
                p.editor.clone()
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1000.), px(650.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let (header, paragraph) = editor.read_with(&visual, |s, _| {
            (
                s.range_to_bounds(&(0..0)).unwrap(),
                s.range_to_bounds(&(body..body)).unwrap(),
            )
        });
        assert!(header.size.height > paragraph.size.height);
        assert!((header.bottom() - paragraph.top()).abs() < px(0.1));
        visual.simulate_click(
            paragraph.origin + point(px(1.), px(8.)),
            Modifiers::default(),
        );
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range(), body..body));
        visual.simulate_keystrokes("up");
        editor.read_with(&visual, |s, _| assert!(s.selected_range().start < body));
        visual.simulate_keystrokes("down");
        editor.read_with(&visual, |s, _| assert_eq!(s.selected_range().start, body));
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    assert_eq!(
                        s.character_index_for_point(
                            paragraph.origin + point(px(1.), px(8.)),
                            w,
                            cx
                        ),
                        Some(body)
                    );
                })
            })
            .unwrap();
        handle
            .update(&mut visual, |p, _, cx| {
                p.editor.update(cx, |s, cx| s.set_selected_range(2..2, cx));
                p.update_presentation(cx);
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        handle
            .update(&mut visual, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    let source_bounds = s.range_to_bounds(&(2..2)).unwrap();
                    let viewport = s.input_bounds();
                    let candidate = s.bounds_for_range(2..2, viewport, w, cx).unwrap();
                    assert_eq!(candidate.size.height, source_bounds.size.height);
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
                })
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        editor.read_with(&visual, |s, _| {
            assert!(s.range_to_bounds(&(2..2)).unwrap().size.height > paragraph.size.height)
        });
    }

    struct ConcealProbe {
        state: Entity<EditorState>,
    }
    impl Render for ConcealProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                Editor::new(&self.state)
                    .appearance(false)
                    .bordered(false)
                    .h_full()
                    .text_size(px(16.)),
            )
        }
    }
    #[gpui::test]
    fn tab_display_width_updates_geometry_wrapping_and_mouse_selection(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "\t中文😀\n \tsecond\n**bold**\tend\né👩‍💻\tend";
        let handle = cx.add_window(|w, cx| {
            let state = cx.new(|cx| {
                EditorState::new(w, cx)
                    .default_value(source)
                    .line_number(false)
            });
            state.update(cx, |s, cx| {
                s.set_tab_size(
                    gpui_base::input::TabSize {
                        tab_size: 2,
                        hard_tabs: true,
                    },
                    cx,
                );
                s.focus(w, cx);
            });
            ConcealProbe { state }
        });
        let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(700.), px(300.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let small = state.read_with(&visual, |s, _| s.range_to_bounds(&(0..1)).unwrap());
        assert!(small.size.width > px(0.));
        state.update(&mut visual, |s, cx| {
            s.set_tab_size(
                gpui_base::input::TabSize {
                    tab_size: 8,
                    hard_tabs: true,
                },
                cx,
            );
            let start = source.find("**bold").unwrap();
            s.set_concealed_ranges(vec![start..start + 2, start + 6..start + 8], cx);
        });
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let large = state.read_with(&visual, |s, _| {
            let tab = s.range_to_bounds(&(0..1)).unwrap();
            assert!((f32::from(tab.size.width) - f32::from(small.size.width) * 4.).abs() < 0.1);
            let space = source.find(" \t").unwrap();
            let after_space = s.range_to_bounds(&(space + 1..space + 2)).unwrap();
            assert!(
                (f32::from(after_space.size.width) - f32::from(tab.size.width) * 7. / 8.).abs()
                    < 0.1
            );
            let marker = source.find("**bold").unwrap();
            let unicode_tab = source.rfind('\t').unwrap();
            let unicode_width = s
                .range_to_bounds(&(unicode_tab..unicode_tab + 1))
                .unwrap()
                .size
                .width;
            assert!((f32::from(unicode_width) - f32::from(tab.size.width) * 6. / 8.).abs() < 0.1);
            assert_eq!(
                s.range_to_bounds(&(marker..marker + 2)).unwrap().size.width,
                px(0.)
            );
            tab
        });
        visual.simulate_click(
            large.origin + point(large.size.width - px(1.), px(8.)),
            Modifiers::default(),
        );
        state.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 1..1));
        visual.simulate_resize(size(px(75.), px(300.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        state.read_with(&visual, |s, _| {
            let before = s.range_to_bounds(&(0..0)).unwrap();
            let after = s.range_to_bounds(&(7..7)).unwrap();
            assert!(after.origin.y > before.origin.y);
            assert_eq!(s.value().as_ref(), source);
        });
        visual.simulate_keystrokes("ctrl-a ctrl-c");
        visual
            .update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
    }

    #[gpui::test]
    fn concealment_removes_marker_width_without_changing_source_or_copy(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "**中文😀** [[很长的路径/笔记|别名]]";
        let spans = markdown::spans(source);
        let masks: Vec<_> = spans.iter().flat_map(|s| s.markers.clone()).collect();
        let handle = cx.add_window(|w, cx| {
            let state = cx.new(|cx| {
                EditorState::new(w, cx)
                    .default_value(source)
                    .line_number(false)
            });
            state.update(cx, |s, cx| {
                s.set_concealed_ranges(masks, cx);
                s.focus(w, cx);
            });
            ConcealProbe { state }
        });
        let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(300.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        state.read_with(&visual, |s, _| {
            assert_eq!(s.value().as_ref(), source);
            for span in &spans {
                for marker in &span.markers {
                    assert_eq!(s.range_to_bounds(marker).unwrap().size.width, px(0.));
                }
                assert!(s.range_to_bounds(&span.content).unwrap().size.width > px(0.));
            }
        });
        let origin = state.read_with(&visual, |s, _| s.range_to_bounds(&(2..2)).unwrap().origin);
        visual.simulate_click(origin + point(px(1.), px(8.)), Modifiers::default());
        state.read_with(&visual, |s, _| assert_eq!(s.selected_range(), 2..2));
        visual.simulate_keystrokes("ctrl-a ctrl-c");
        visual
            .update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source));
    }
    #[gpui::test]
    fn concealed_prefix_does_not_create_a_blank_wrapped_row(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "**👨‍👩‍👧‍👦**";
        let handle = cx.add_window(|w, cx| {
            let state = cx.new(|cx| {
                EditorState::new(w, cx)
                    .default_value(source)
                    .line_number(false)
            });
            state.update(cx, |s, cx| {
                s.set_concealed_ranges(vec![0..2, source.len() - 2..source.len()], cx);
                s.focus(w, cx);
            });
            ConcealProbe { state }
        });
        let state = handle.update(cx, |p, _, _| p.state.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(25.), px(300.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        state.read_with(&visual, |s, _| {
            let prefix = s.range_to_bounds(&(0..0)).unwrap();
            let content = s.range_to_bounds(&(2..2)).unwrap();
            assert_eq!(
                prefix.origin.y, content.origin.y,
                "a concealed marker must not become a blank visual row"
            );
            assert_eq!(s.value().as_ref(), source);
        });
    }
    #[gpui::test]
    fn concealment_rejects_partial_graphemes_and_does_not_enter_history(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "**👩‍💻e\u{301}**\r\n尾行";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_concealed_ranges(
                        vec![
                            0..2,
                            2..6,
                            source.find('\r').unwrap()..source.find('\r').unwrap() + 2,
                        ],
                        cx,
                    );
                    assert_eq!(s.concealed_ranges(), vec![0..2]);
                    s.undo(&gpui_component::input::Undo, w, cx);
                    assert_eq!(s.value().as_ref(), source);
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.replace("新", w, cx);
                    s.undo(&gpui_component::input::Undo, w, cx);
                    assert_eq!(s.value().as_ref(), source);
                });
            })
            .unwrap();
    }
    #[gpui::test]
    fn live_markers_reveal_for_the_caret_and_for_syntax_search(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "**粗体**\n其他行";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.update_presentation(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                p.update_presentation(cx);
                assert_eq!(p.editor.read(cx).concealed_ranges().len(), 2);
                p.editor.update(cx, |s, cx| s.set_selected_range(3..3, cx));
                p.update_presentation(cx);
                assert!(p.editor.read(cx).concealed_ranges().is_empty());
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx);
                    s.set_search_query("**", false, cx);
                });
                p.update_presentation(cx);
                assert!(p.editor.read(cx).concealed_ranges().is_empty());
                p.editor.update(cx, |s, cx| s.close_search(cx));
                p.update_presentation(cx);
                assert_eq!(p.editor.read(cx).concealed_ranges().len(), 2);
                p.live = false;
                p.update_presentation(cx);
                assert!(p.editor.read(cx).concealed_ranges().is_empty());
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
    #[gpui::test]
    fn ime_keeps_other_lines_compact_and_commits_only_source_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "**Title**\n文本";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_selected_range(source.len()..source.len(), cx)
                });
                p.update_presentation(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, w, cx| {
                p.update_presentation(cx);
                assert_eq!(p.editor.read(cx).concealed_ranges(), vec![0..2, 7..9]);
                p.editor.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
                });
                p.update_presentation(cx);
                assert_eq!(p.editor.read(cx).concealed_ranges(), vec![0..2, 7..9]);
                p.editor.update(cx, |s, cx| {
                    assert_eq!(s.marked_text_range(w, cx), Some(12..14));
                    s.replace_text_in_range(None, "你", w, cx);
                });
                p.update_presentation(cx);
                assert_eq!(p.editor.read(cx).value().as_ref(), "**Title**\n文本你");
                p.editor
                    .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
    #[gpui::test]
    fn edits_that_change_regional_indicator_boundaries_drop_invalid_masks(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "🇦🇧🇨🇩";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, w, cx| {
                p.editor.update(cx, |s, cx| {
                    s.set_concealed_ranges(std::iter::once(8..16).collect(), cx);
                    assert_eq!(s.concealed_ranges().len(), 1);
                    s.set_selected_range(0..0, cx);
                    s.replace("🇪", w, cx);
                    assert!(s.concealed_ranges().is_empty());
                    assert_eq!(s.value().as_ref(), "🇪🇦🇧🇨🇩");
                })
            })
            .unwrap();
    }
    #[gpui::test]
    fn reading_block_anchor_reveals_the_correct_list_item(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "# 标题\n\n- 第一项\n- 第二项 ^second\n";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.reading = true;
                p.update_presentation(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, _, cx| {
                let original = index::anchor_range(source, &p.parsed, "^second")
                    .unwrap()
                    .start;
                let mapped = p.rendered.output_offset(&p.current_path, original).unwrap();
                let rendered = p.preview.read(cx).rendered_text();
                let position = rendered.position_for_source_offset(mapped).unwrap();
                assert!(
                    rendered.as_str()[position..].starts_with("第二项"),
                    "{} at {position}",
                    rendered.as_str()
                );
                assert!(!rendered.as_str().contains("^second"));
            })
            .unwrap();
    }
    #[gpui::test]
    fn reading_link_click_emits_the_original_source_context(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("[跳转](../目的.md)", w, cx));
        let received = Rc::new(RefCell::new(Vec::new()));
        let capture = received.clone();
        let (preview, _subscription) = handle
            .update(cx, |p, w, cx| {
                p.reading = true;
                p.readable_width = false;
                p.set_reference_context(
                    "folder/note.md".into(),
                    PathBuf::new(),
                    Arc::default(),
                    cx,
                );
                p.update_presentation(cx);
                p.focus_view(w, cx);
                (
                    p.preview.clone(),
                    cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                        if let EditorEvent::FollowReference(reference) = event {
                            capture.borrow_mut().push(reference.clone());
                        }
                    }),
                )
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let bounds = preview.read_with(&visual, |p, _| p.bounds());
        visual.simulate_click(bounds.origin + point(px(8.), px(8.)), Modifiers::default());
        visual.simulate_mouse_down(
            bounds.origin + point(px(8.), px(8.)),
            MouseButton::Right,
            Modifiers::default(),
        );
        visual.simulate_mouse_up(
            bounds.origin + point(px(8.), px(8.)),
            MouseButton::Right,
            Modifiers::default(),
        );
        assert_eq!(received.borrow().len(), 1, "preview bounds: {bounds:?}");
        assert_eq!(received.borrow()[0].from, PathBuf::from("folder/note.md"));
        assert_eq!(received.borrow()[0].target, "../目的.md");
    }
    #[gpui::test]
    fn reading_undo_action_uses_the_document_history(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-z", gpui_component::input::Undo, None)]);
        });
        let source = "- [ ] 可以撤销\n";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| p.update_presentation(cx))
            .unwrap();
        cx.run_until_parked();
        let editor = handle
            .update(cx, |p, w, cx| {
                p.reading = true;
                p.toggle_task(p.rendered.tasks[0].clone(), true, w, cx);
                p.focus_view(w, cx);
                p.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(900.), px(600.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-z");
        editor.read_with(&visual, |e, _| assert_eq!(e.value().as_ref(), source));
    }
    #[gpui::test]
    fn reading_task_toggle_preserves_source_mode_cursor_and_undo(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "---\ntags: [测试]\n---\n[[长目标|短名]]\n\n- [ ] 中文😀\n";
        let handle = cx.add_window(|w, cx| EditorPane::new(source, w, cx));
        handle
            .update(cx, |p, _, cx| {
                p.reading = true;
                p.update_presentation(cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, w, cx| {
                let target = p.rendered.tasks[0].clone();
                assert_eq!(&source[target.marker.clone()], " ");
                p.toggle_task(target, true, w, cx);
                assert!(p.reading);
                assert_eq!(p.editor.read(cx).selected_range(), 0..0);
                assert_eq!(
                    p.editor.read(cx).value().as_ref(),
                    source.replace("[ ]", "[x]")
                );
                p.editor
                    .update(cx, |s, cx| s.undo(&gpui_component::input::Undo, w, cx));
                assert_eq!(p.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }
    #[gpui::test]
    fn stale_reading_task_does_not_toggle_another_item(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|w, cx| EditorPane::new("- [ ] 第一项\n", w, cx));
        handle
            .update(cx, |p, _, cx| p.update_presentation(cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |p, w, cx| {
                let stale = p.rendered.tasks[0].clone();
                let newer = "- [ ] 插入的新项\n- [ ] 第一项\n";
                p.editor.update(cx, |s, cx| s.set_value(newer, w, cx));
                p.toggle_task(stale, true, w, cx);
                assert_eq!(p.editor.read(cx).value().as_ref(), newer);
            })
            .unwrap();
    }

    #[gpui::test]
    fn ctrl_hover_click_emits_wiki_target_without_changing_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "中文😀 [[目录/笔记|标签]]";
        let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
        let targets = Rc::new(RefCell::new(Vec::new()));
        let captured = targets.clone();
        let _subscription = handle
            .update(cx, |pane, _, cx| {
                pane.editor.update(cx, |state, cx| {
                    let offset = "中文😀 [[目".len();
                    state.set_selected_range(offset..offset, cx);
                });
                cx.subscribe(&cx.entity(), move |_, _, event: &EditorEvent, _| {
                    if let EditorEvent::FollowLink(target) = event {
                        captured.borrow_mut().push(target.clone());
                    }
                })
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let position = handle
            .update(&mut visual, |pane, _, cx| {
                let state = pane.editor.read(cx);
                let (caret, _) = state.cursor_layout().unwrap();
                caret.origin + state.scroll_offset() + point(px(1.), px(5.))
            })
            .unwrap();
        let modifiers = Modifiers {
            control: true,
            ..Default::default()
        };
        visual.simulate_mouse_move(position, None, modifiers);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_click(position, modifiers);
        assert_eq!(targets.borrow().as_slice(), &["目录/笔记"]);
        handle
            .update(&mut visual, |pane, _, cx| {
                assert_eq!(pane.editor.read(cx).value().as_ref(), source);
            })
            .unwrap();
    }

    #[gpui::test]
    fn resize_keeps_visible_caret_and_selection_direction(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "中文 English 👩‍💻 e\u{301}".repeat(500);
        let handle = cx.add_window(|window, cx| EditorPane::new(&source, window, cx));
        let editor = handle.update(cx, |pane, _, _| pane.editor.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_keystrokes("ctrl-end shift-left");
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let before = editor.read_with(&visual, |state, _| (state.selected_range(), state.cursor()));
        for width in [1900., 900.] {
            visual.simulate_resize(size(px(width), px(800.)));
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
            editor.read_with(&visual, |state, _| {
                assert_eq!((state.selected_range(), state.cursor()), before);
                let (mut caret, _) = state.cursor_layout().expect("caret should be laid out");
                caret.origin += state.scroll_offset();
                assert!(state.input_bounds().intersects(&caret),
                    "caret should remain visible at width {width}: caret={caret:?}, input={:?}, scroll={:?}",state.input_bounds(),state.scroll_offset());
                assert_eq!(state.value().as_ref(), source);
            });
        }
    }

    #[gpui::test]
    fn keyboard_selection_and_delete_preserve_whole_graphemes(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let source = "A👩‍💻e\u{301}";
        let handle = cx.add_window(|window, cx| EditorPane::new(source, window, cx));
        let editor = handle
            .update(cx, |pane, _, cx| {
                pane.editor.update(cx, |state, cx| {
                    state.set_selected_range(source.len()..source.len(), cx)
                });
                pane.editor.clone()
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.simulate_keystrokes("shift-left");
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.selected_range(), 12..15)
        });
        visual.simulate_keystrokes("shift-left");
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.selected_range(), 1..15)
        });
        visual.simulate_keystrokes("backspace");
        editor.read_with(&visual, |state, _| assert_eq!(state.value().as_ref(), "A"));
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-z"
        } else {
            "ctrl-z"
        });
        editor.read_with(&visual, |state, _| {
            assert_eq!(state.value().as_ref(), source)
        });
    }

    #[gpui::test]
    fn newer_parse_wins_and_reading_keeps_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle =
            cx.add_window(|window, cx| EditorPane::new("# 旧文档\n[[旧链接]]", window, cx));
        handle
            .update(cx, |pane, window, cx| {
                pane.update_presentation(cx);
                pane.editor.update(cx, |state, cx| {
                    state.set_value(
                        "# 新文档😀\n\n|列|值|\n|-|-|\n|中文|数据|\n\n[[新链接]]",
                        window,
                        cx,
                    )
                });
                pane.update_presentation(cx);
                pane.reading = true;
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |pane, _, cx| {
                assert_eq!(pane.parsed.headings[0].title, "新文档😀");
                assert_eq!(pane.parsed.links[0].target, "新链接");
                assert!(pane.editor.read(cx).value().contains("|中文|数据|"));
                assert!(pane.editor.read(cx).value().contains("[[新链接]]"));
            })
            .unwrap();
    }

    #[gpui::test]
    fn component_ime_utf16_commit_cancel_and_source_preservation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(|window, cx| EditorPane::new("**中😀**\n", window, cx));
        handle
            .update(cx, |pane, window, cx| {
                let editor = pane.editor.clone();
                editor.update(cx, |state, cx| {
                    state.set_selected_range(12..12, cx);
                    state.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\nni");
                    assert_eq!(state.marked_text_range(window, cx), Some(8..10));
                    state.replace_text_in_range(None, "你", window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\n你");
                    assert_eq!(state.marked_text_range(window, cx), None);
                    state.replace_and_mark_text_in_range(None, "hao", Some(3..3), window, cx);
                    state.replace_and_mark_text_in_range(None, "", Some(0..0), window, cx);
                    state.unmark_text(window, cx);
                    assert_eq!(state.value().as_ref(), "**中😀**\n你");
                    let mut actual = None;
                    assert_eq!(
                        state
                            .text_for_range(3..5, &mut actual, window, cx)
                            .as_deref(),
                        Some("😀")
                    );
                    assert_eq!(actual, Some(3..5));
                });
                pane.update_presentation(cx);
                assert_eq!(pane.editor.read(cx).value().as_ref(), "**中😀**\n你");
            })
            .unwrap();
    }
}
