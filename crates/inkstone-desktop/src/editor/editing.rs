//! Source editing commands, folding, pairing, and Markdown keys.

use super::*;

impl EditorPane {
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
                |text, selections| inkstone_core::line_edit::move_lines(text, selections, down),
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
                |text, selections| inkstone_core::line_edit::copy_lines(text, selections, down),
                window,
                cx,
            );
            editor.focus(window, cx);
        });
    }

    pub(super) fn markdown_key(
        &mut self,
        key: inkstone_core::markdown_edit::Key,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.smart_lists
            && matches!(
                key,
                inkstone_core::markdown_edit::Key::Enter
                    | inkstone_core::markdown_edit::Key::SoftEnter
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
            if matches!(key, inkstone_core::markdown_edit::Key::Enter) {
                let indent = self.indentation;
                if self.editor.update(cx, |state, cx| {
                    state.apply_selection_transform(
                        |text, selections| {
                            inkstone_core::markdown_edit::enter_at_selections(
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
            if matches!(key, inkstone_core::markdown_edit::Key::SoftEnter) {
                self.editor.update(cx, |state, cx| {
                    state.apply_selection_replacements(
                        inkstone_core::markdown_edit::soft_break_replacements,
                        window,
                        cx,
                    );
                });
                cx.stop_propagation();
                return;
            }
            if matches!(
                key,
                inkstone_core::markdown_edit::Key::Indent
                    | inkstone_core::markdown_edit::Key::Outdent
            ) {
                let indent = self.indentation;
                self.editor.update(cx, |state, cx| {
                    state.apply_line_edits(
                        |line| {
                            inkstone_core::markdown_edit::indentation_change(
                                line,
                                indent.tab_size,
                                indent.hard_tabs,
                                matches!(key, inkstone_core::markdown_edit::Key::Outdent),
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
            inkstone_core::markdown_edit::Key::Indent | inkstone_core::markdown_edit::Key::Outdent
        ) && !state.has_multiple_selections()
            && let Some(navigation) = inkstone_core::tables::navigate(
                &state.value(),
                state.selected_range(),
                matches!(key, inkstone_core::markdown_edit::Key::Outdent),
            )
        {
            self.editor.update(cx, |s, cx| match navigation {
                inkstone_core::tables::Navigation::Select(range) => s.set_selected_range(range, cx),
                inkstone_core::tables::Navigation::Edit(edit) => {
                    s.apply_source_edit(edit.range, &edit.replacement, edit.selection, window, cx);
                }
            });
            cx.stop_propagation();
            return;
        }
        let Some(edit) = inkstone_core::markdown_edit::edit_with_options(
            &state.value(),
            state.selected_range(),
            key,
            self.indentation.tab_size,
            self.indentation.hard_tabs,
            self.smart_lists,
        ) else {
            if matches!(
                key,
                inkstone_core::markdown_edit::Key::Indent
                    | inkstone_core::markdown_edit::Key::Outdent
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
    pub(super) fn toggle_task(
        &mut self,
        target: inkstone_core::rendering::TaskTarget,
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
        if current.as_ref() != target.baseline.as_ref() {
            return;
        }
        let Some(updated) = index::set_task(&current, target.marker.clone(), checked) else {
            return;
        };
        self.editor.update(cx, |state, cx| {
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
            state.apply_selection_transform(
                |_, selections| {
                    Some((
                        updated,
                        selections
                            .iter()
                            .map(|selection| map(selection.start)..map(selection.end))
                            .collect(),
                    ))
                },
                window,
                cx,
            );
            state.set_scroll_offset(scroll, cx);
        });
        cx.notify();
    }
}
