//! Versioned parsing, decorations, and live preview projection.

use super::*;

impl EditorPane {
    pub(super) fn update_presentation(&mut self, cx: &mut Context<Self>) {
        let state = self.editor.read(cx);
        if state.is_composing() {
            // The component rebases prepared objects during preedit. Keep their
            // geometry and widget resources until the candidate is committed.
            self.last_presentation = None;
            return;
        }
        let text = state.value();
        let selections = state.selected_ranges();
        let search_query = state
            .search_session()
            .is_active()
            .then(|| state.search_session().query.clone());
        let search_matches = state.search_session().matcher.matched_ranges();
        if self.parse_source != text || self.parsed_context_revision != self.context_revision {
            self.pending_live_anchor = None;
            let initial_parse = self.parse_revision == 0;
            // Present only syntax for the current text. Dropping edited spans or
            // task markers while awaiting a background parse exposes source for
            // one frame on every keystroke (including spaces and punctuation).
            // Keep the initial load asynchronous so opening a note retains its
            // existing layout/focus initialization order. Subsequent edits must
            // never pass through that unstyled loading state.
            if !initial_parse {
                let snapshot = Arc::new(
                    self.syntax_snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.update_plain_paragraph(&text))
                        .unwrap_or_else(|| inkstone_core::syntax::Snapshot::new(&text)),
                );
                self.retain_live_graphics(&snapshot);
                self.spans = markdown::spans_snapshot(&snapshot);
                self.parsed = index::parse_snapshot(&snapshot);
                self.syntax_snapshot = Some(snapshot);
                self.typography_ready = true;
                *self.link_cache.borrow_mut() = (text.clone(), self.parsed.clone());
            }
            self.parse_source = text.clone();
            self.parsed_context_revision = self.context_revision;
            self.parse_revision += 1;
            let revision = self.parse_revision;
            let source = text.clone();
            let references = self.reference_index.clone();
            let path = self.current_path.clone();
            let context_revision = self.context_revision;
            let cached_snapshot = self.syntax_snapshot.clone();
            let graphics = crate::native_graphics::Service::get(cx);
            let light = self.light;
            let task = cx.background_executor().spawn(async move {
                let snapshot = cached_snapshot
                    .filter(|s| s.source.as_ref() == source.as_ref())
                    .unwrap_or_else(|| Arc::new(inkstone_core::syntax::Snapshot::new(&source)));
                let initial = initial_parse.then(|| {
                    (
                        snapshot.clone(),
                        markdown::spans_snapshot(&snapshot),
                        index::parse_snapshot(&snapshot),
                    )
                });
                let reading = inkstone_core::rendering::reading_snapshot(
                    &references,
                    &path,
                    snapshot.clone(),
                    0..source.len(),
                );
                let fragments =
                    inkstone_core::preview::fragments(&references, &path, snapshot, &reading)
                        .into_iter()
                        .map(|fragment| {
                            let measured = fragment
                                .graphic
                                .as_ref()
                                .map(|(kind, source)| graphics.measure_only(*kind, source, light));
                            (fragment, measured)
                        })
                        .collect();
                (initial, reading, fragments)
            });
            self.parse_task = Some(cx.spawn(async move |this, cx| {
                let (initial, reading, fragments) = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.editor.read(cx).is_composing()
                        || this.parse_revision != revision
                        || this.context_revision != context_revision
                        || this.editor.read(cx).value() != this.parse_source
                    {
                        return;
                    }
                    if let Some((snapshot, spans, parsed)) = initial {
                        this.syntax_snapshot = Some(snapshot);
                        this.spans = spans;
                        this.parsed = parsed;
                        this.typography_ready = true;
                        *this.link_cache.borrow_mut() =
                            (this.parse_source.clone(), this.parsed.clone());
                    }
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
                    this.preview
                        .update(cx, |state, cx| state.set_text(&reading.markdown, cx));
                    this.rendered = Arc::new(reading);
                    this.install_live_objects(fragments, cx);
                    this.last_presentation = None;
                    cx.notify();
                });
            }));
        }
        let key = PresentationSnapshot {
            text: text.clone(),
            selections: selections.clone(),
            live: self.live,
            light: self.light,
            search: search_query.clone(),
            font_size: self.font_size,
        };
        if self.last_presentation.as_ref() == Some(&key) {
            return;
        }
        self.last_presentation = Some(key);
        let mut decorations = Vec::new();
        let mut concealed = vec![];
        let mut concealed_lines = vec![];
        self.live_quotes.clear();
        self.live_rules.clear();
        self.live_lists.clear();
        if self.live {
            for span in &self.spans {
                let style = match span.kind {
                    Kind::Comment => continue,
                    Kind::Rule => {
                        let revealed = selections.iter().any(|selection| span.active(selection))
                            || search_query.is_some()
                                && search_matches
                                    .get(
                                        search_matches
                                            .partition_point(|r| r.end <= span.content.start),
                                    )
                                    .is_some_and(|r| r.start < span.content.end);
                        if !revealed {
                            concealed.push(span.content.clone());
                            self.live_rules.push(span.content.clone());
                        }
                        continue;
                    }
                    Kind::ListMarker => {
                        // Task widgets own their entire prefix, including the bullet.
                        if self.parsed.tasks.iter().any(|task| {
                            task.start <= span.content.start
                                && span.content.end <= task.marker.start
                        }) {
                            continue;
                        }
                        let revealed = selections.iter().any(|selection| span.active(selection))
                            || search_query.is_some()
                                && search_matches
                                    .get(
                                        search_matches
                                            .partition_point(|r| r.end <= span.content.start),
                                    )
                                    .is_some_and(|r| r.start < span.content.end);
                        if !revealed {
                            self.live_lists.push(span.content.clone());
                        }
                        continue;
                    }
                    Kind::QuoteContinuation => {
                        self.live_quotes.push(span.source.clone());
                        continue;
                    }
                    Kind::QuoteMarker => {
                        let revealed = selections.iter().any(|selection| span.active(selection))
                            || search_query.is_some()
                                && search_matches
                                    .get(
                                        search_matches
                                            .partition_point(|r| r.end <= span.content.start),
                                    )
                                    .is_some_and(|r| r.start < span.content.end);
                        decorations.push(TextDecoration::new(
                            span.content.clone(),
                            HighlightStyle {
                                color: Some(if revealed {
                                    rgb(if self.light { 0xababab } else { 0x666666 }).into()
                                } else {
                                    rgba(0x00000000).into()
                                }),
                                ..Default::default()
                            },
                        ));
                        self.live_quotes.push(span.content.start..span.source.end);
                        continue;
                    }
                    Kind::Heading => HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        color: Some(crate::theme::palette(self.light).foreground.into()),
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
                        color: Some(crate::theme::palette(self.light).foreground.into()),
                        background_color: Some(crate::theme::palette(self.light).surface.into()),
                        ..Default::default()
                    },
                    Kind::WikiLink | Kind::Link => HighlightStyle {
                        color: Some(crate::theme::palette(self.light).accent.into()),
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
                if !selections.iter().any(|selection| span.active(selection)) && !search_reveals {
                    if let Some(line) = span.setext_line(&text) {
                        concealed_lines.push(line);
                    }
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
        self.live_tasks.clear();
        let mut replacements: Vec<_> = concealed.into_iter().map(|range| (range, px(0.))).collect();
        // Transparent highlights blend with the foreground; they do not hide glyphs.
        // Replace each bullet source glyph with a reserved slot for the overlay instead.
        replacements.extend(
            self.live_lists
                .iter()
                .cloned()
                .map(|range| (range, px(self.font_size * 0.6))),
        );
        if self.live && !self.parsed.tasks.is_empty() {
            let baseline: Arc<str> = self
                .rendered
                .tasks
                .iter()
                .find(|task| {
                    task.path == self.current_path && task.baseline.as_ref() == text.as_ref()
                })
                .map(|task| task.baseline.clone())
                .unwrap_or_else(|| Arc::from(text.as_ref()));
            for task in &self.parsed.tasks {
                let bracket = task.marker.start.saturating_sub(1)..task.marker.end + 1;
                if text.get(bracket.clone()).is_none() {
                    continue;
                }
                // List-item positions can include a leading tab. Keep that
                // indentation visible while replacing the bullet and checkbox.
                let prefix = text.get(task.start..bracket.start).unwrap_or_default();
                let marker_prefix = prefix.trim_start_matches([' ', '\t']);
                let marker_start = task.start + prefix.len() - marker_prefix.len();
                let unordered = marker_prefix.starts_with(['-', '*', '+']);
                let range = if unordered {
                    marker_start..bracket.end
                } else {
                    bracket
                };
                let active = selections.iter().any(|selection| {
                    if selection.is_empty() {
                        range.start <= selection.start && selection.start <= range.end
                    } else {
                        selection.start < range.end && selection.end > range.start
                    }
                });
                let search_reveals = search_query.is_some()
                    && search_matches
                        .get(search_matches.partition_point(|matched| matched.end <= range.start))
                        .is_some_and(|matched| matched.start < range.end);
                if !active && !search_reveals {
                    let inset = if unordered { self.font_size * 0.6 } else { 0. };
                    replacements.push((range.clone(), px(self.font_size + inset)));
                    self.live_tasks.push(live_tasks::TaskWidget {
                        range,
                        inset,
                        checked: task.checked,
                        target: inkstone_core::rendering::TaskTarget {
                            rendered_start: task.start,
                            path: self.current_path.clone(),
                            marker: task.marker.clone(),
                            baseline: baseline.clone(),
                        },
                    });
                }
                if matches!(text.get(task.marker.clone()), Some("x" | "X")) {
                    let start = task.marker.end + 1;
                    let end = text[start..]
                        .find(['\r', '\n'])
                        .map_or(text.len(), |offset| start + offset);
                    decorations.push(TextDecoration::new(
                        start..end,
                        HighlightStyle {
                            color: Some(rgb(if self.light { 0x5c5c5c } else { 0xb3b3b3 }).into()),
                            strikethrough: Some(StrikethroughStyle {
                                thickness: px(1.),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ));
                }
            }
        }
        let mut styles = if !self.live {
            Some(vec![])
        } else if self.typography_ready {
            let heading_lines: std::collections::BTreeMap<_, _> = self
                .spans
                .iter()
                .filter(|span| span.kind == Kind::Heading)
                .map(|span| (span.source.start, span.heading_line_anchors(&text)))
                .collect();
            Some(
                self.parsed
                    .headings
                    .iter()
                    .flat_map(|heading| {
                        let anchors =
                            heading_lines
                                .get(&heading.offset)
                                .cloned()
                                .unwrap_or_else(|| {
                                    text.get(heading.offset..)
                                        .and_then(|raw| raw.chars().next())
                                        .map(|ch| heading.offset..heading.offset + ch.len_utf8())
                                        .into_iter()
                                        .collect()
                                });
                        let i = usize::from(heading.level.saturating_sub(1)).min(5);
                        let scale = markdown::HEADING_SCALES[i];
                        anchors.into_iter().map(move |anchor| {
                            gpui_base::input::LineTypography::new(
                                anchor,
                                scale,
                                scale * [1.2, 1.2, 1.3, 1.4, 1.5, 1.5][i] / 1.5,
                            )
                        })
                    })
                    .collect(),
            )
        } else {
            None
        };
        let objects = self.active_live_objects(
            &text,
            &selections,
            if search_query.is_some() {
                &search_matches
            } else {
                &[]
            },
        );
        let object_ranges: Vec<_> = objects.iter().map(|o| o.source.clone()).collect();
        let projection = self.editor.update(cx, |s, cx| {
            s.set_display_objects(&text, objects, cx);
            s.display_projection(px(self.font_size * 1.5), styles.take().unwrap_or_default())
        });
        let hidden = self.hidden_live_ranges(
            &text,
            &selections,
            if search_query.is_some() {
                &search_matches
            } else {
                &[]
            },
        );
        for range in &hidden {
            let raw = &text[range.clone()];
            if range.start > 0 {
                concealed_lines.push(range.start);
            } else {
                let end = range.start + raw.find(['\r', '\n']).unwrap_or(raw.len());
                if end > range.start {
                    replacements.push((range.start..end, px(0.)));
                }
            }
            concealed_lines.extend(
                raw.match_indices('\n')
                    .map(|(i, _)| range.start + i + 1)
                    .filter(|&i| i < range.end),
            );
        }
        self.live_quotes.retain(|r| {
            !object_ranges
                .iter()
                .any(|o| o.start <= r.start && r.end <= o.end)
        });
        self.live_lists.retain(|r| {
            !object_ranges
                .iter()
                .any(|o| o.start <= r.start && r.end <= o.end)
        });
        self.live_rules.retain(|r| {
            !object_ranges
                .iter()
                .any(|o| o.start <= r.start && r.end <= o.end)
        });
        self.live_tasks.retain(|t| {
            !object_ranges
                .iter()
                .any(|o| o.start <= t.range.start && t.range.end <= o.end)
        });
        replacements.retain(|(r, _)| {
            !projection
                .replacements
                .iter()
                .any(|(o, _)| r.start < o.end && o.start < r.end)
        });
        replacements.extend(projection.replacements);
        concealed_lines.extend(projection.hidden_lines);
        styles = Some(projection.typography);
        let was_visible = self.last_geometry.is_some_and(|(_, visible)| visible);
        if let Some(styles) = styles
            && self
                .editor
                .update(cx, |s, cx| s.set_line_typography(styles, cx))
            && was_visible
        {
            self.reveal_after_concealment = true;
        }
        for span in self.spans.iter().filter(|span| span.kind == Kind::Comment) {
            decorations.push(TextDecoration::new(
                span.source.clone(),
                HighlightStyle {
                    color: Some(rgb(if self.light { 0xababab } else { 0x666666 }).into()),
                    ..Default::default()
                },
            ));
        }
        self.decorations.set(decorations, cx);
        if self
            .editor
            .update(cx, |s, cx| s.set_concealed_lines(concealed_lines, cx))
            && was_visible
        {
            self.reveal_after_concealment = true;
        }
        let was_visible = self.last_geometry.is_some_and(|(_, visible)| visible);
        if self.editor.update(cx, |s, cx| {
            s.set_concealed_ranges_with_widths(replacements, cx)
        }) && (was_visible || search_query.is_some())
        {
            self.reveal_after_concealment = true;
        }
    }
}
