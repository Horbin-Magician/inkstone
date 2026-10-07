//! GPUI view composition and input event routing.

use super::*;

impl Render for EditorPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = (
            self.font_size.to_bits(),
            _window.scale_factor().to_bits(),
            self.light,
        );
        if self
            .graphic_appearance
            .replace(appearance)
            .is_some_and(|old| old != appearance)
        {
            self.context_revision += 1;
        }
        self.graphic_dpi = _window.scale_factor();
        self.update_presentation(cx);
        self.refresh_visible_graphics(cx);
        if let Some(anchor) = self.pending_live_anchor.take() {
            self.reveal_after_concealment = false;
            cx.on_next_frame(_window, move |pane, _, cx| {
                pane.editor.update(cx, |state, cx| {
                    state.restore_display_scroll_anchor(anchor, cx);
                });
            });
        }
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
            && self.pending_preview_jump.is_some()
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
            && self.pending_reading_position.is_some()
            && self.preview.read(cx).is_parsed()
            && self
                .rendered
                .source_matches(&self.current_path, &self.editor.read(cx).value())
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
            .markdown_extensions(crate::native_graphics::extensions(
                font_size,
                _window.scale_factor(),
                self.light,
                self.strict_line_breaks,
                cx.entity().downgrade(),
                cx,
            ))
            .text_size(px(font_size))
            .line_height(relative(self.line_spacing))
            .style(
                gpui_base::text::TextViewStyle::from_theme(&gpui_base::Theme::global(cx))
                    .with_foreground(crate::theme::palette(self.light).foreground.into())
                    .with_link(crate::theme::palette(self.light).accent.into())
                    .with_code_background(crate::theme::palette(self.light).surface.into())
                    .with_inline_code(HighlightStyle {
                        color: Some(crate::theme::palette(self.light).foreground.into()),
                        ..Default::default()
                    })
                    .with_border(crate::theme::palette(self.light).border.into())
                    .with_task_checkbox(
                        StyleRefinement::default()
                            .rounded_full()
                            .border_color(rgb(if self.light { 0xababab } else { 0x666666 })),
                    )
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
                            .bg(crate::theme::palette(self.light).background)
                            .text_color(crate::theme::palette(self.light).foreground),
                    ),
            )
            .scrollable(true)
            .scrollbar_visible(false)
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
                            && inkstone_core::rendering::is_remote_image(&reference.target)
                        {
                            return SharedUri::from(reference.target.clone()).into();
                        }
                        if let Some(path) = inkstone_core::rendering::asset_path(
                            &root,
                            reference,
                            &asset_index.files,
                        ) {
                            return path.into();
                        }
                    }
                    root.join(".inkstone-missing-image").into()
                } else if inkstone_core::rendering::is_remote_image(&raw) {
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
                let new_tab = event.modifiers().secondary();
                let _ = weak.update(cx, |this, cx| {
                    if let Some(id) = url
                        .strip_prefix("inkstone-reference:")
                        .and_then(|s| s.parse::<usize>().ok())
                    {
                        if let Some(reference) = this.rendered.references.get(id).cloned() {
                            if !reference.wiki
                                && inkstone_core::rendering::is_external_link(&reference.target)
                            {
                                cx.open_url(&reference.target);
                            } else {
                                cx.emit(if new_tab {
                                    EditorEvent::FollowReferenceInNewTab(reference)
                                } else {
                                    EditorEvent::FollowReference(reference)
                                });
                            }
                        }
                    } else if let Some(index) = url
                        .strip_prefix("inkstone-link:")
                        .and_then(|i| i.parse::<usize>().ok())
                    {
                        if let Some(link) = this.parsed.links.get(index) {
                            cx.emit(if new_tab {
                                EditorEvent::FollowLinkInNewTab(link.target.clone())
                            } else {
                                EditorEvent::FollowLink(link.target.clone())
                            });
                        }
                    } else if inkstone_core::rendering::is_external_link(url) {
                        cx.open_url(url);
                    } else {
                        cx.emit(if new_tab {
                            EditorEvent::FollowMarkdownLinkInNewTab(url.to_string())
                        } else {
                            EditorEvent::FollowMarkdownLink(url.to_string())
                        });
                    }
                });
            });
        let scrollbar = if self.reading {
            gpui_base::Scrollbar::vertical(self.preview.read(cx).list_state())
                .viewport_from_layout()
                .into_any_element()
        } else {
            self.editor.update(cx, |state, cx| {
                state.external_scrollbar(cx).into_any_element()
            })
        };
        div()
            .id("editor-pane")
            .relative()
            .capture_action(cx.listener(
                |this, action: &gpui_component::input::Enter, window, cx| {
                    if !action.secondary {
                        this.markdown_key(
                            if action.shift {
                                inkstone_core::markdown_edit::Key::SoftEnter
                            } else {
                                inkstone_core::markdown_edit::Key::Enter
                            },
                            window,
                            cx,
                        );
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::IndentInline, window, cx| {
                    this.markdown_key(inkstone_core::markdown_edit::Key::Indent, window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::OutdentInline, window, cx| {
                    this.markdown_key(inkstone_core::markdown_edit::Key::Outdent, window, cx);
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_component::input::Backspace, window, cx| {
                    this.markdown_key(inkstone_core::markdown_edit::Key::Backspace, window, cx);
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
            .bg(crate::theme::palette(self.light).background)
            .text_color(crate::theme::palette(self.light).foreground)
            // Both modes share this inset, so the text column keeps the same
            // width when switching between editing and reading.
            .px(px(32.))
            .pt(px(12.))
            .children(self.footnote_panel(_window, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .justify_center()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w_full()
                            .h_full()
                            .min_w_0()
                            .when(self.readable_width, |s| s.max_w(px(700.)))
                            .when(self.reading, |s| s.child(preview))
                            .when(!self.reading, |s| {
                                // The gutter narrows the wrap. Extend the editor by
                                // the gutter and shift it left, so the text column
                                // starts and wraps where the reading view's does,
                                // while the gutter sits in the pane inset.
                                let gutter = self
                                    .editor
                                    .read(cx)
                                    .gutter_width(px(self.font_size), _window);
                                s.relative().child(div().size_full()).child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(-gutter)
                                        .right_0()
                                        .h_full()
                                        .child(
                                            Editor::new(&self.editor)
                                                .w_full()
                                                .on_paste(move |item, _, cx| {
                                                    for entry in &item.entries {
                                                        match entry {
                                                            ClipboardEntry::ExternalPaths(
                                                                paths,
                                                            ) => {
                                                                let _ =
                                                                    paste_weak.update(
                                                                        cx,
                                                                        |_, cx| {
                                                                            cx.emit(
                                                                        EditorEvent::PasteFiles(
                                                                            paths.paths().to_vec(),
                                                                        ),
                                                                    )
                                                                        },
                                                                    );
                                                                return true;
                                                            }
                                                            ClipboardEntry::Image(image) => {
                                                                let _ =
                                                                    paste_weak.update(
                                                                        cx,
                                                                        |_, cx| {
                                                                            cx.emit(
                                                                        EditorEvent::PasteImage(
                                                                            format!(
                                                                                "粘贴图片.{}",
                                                                                image
                                                                                    .format()
                                                                                    .extension()
                                                                            ),
                                                                            image.bytes().to_vec(),
                                                                        ),
                                                                    )
                                                                        },
                                                                    );
                                                                return true;
                                                            }
                                                            _ => (),
                                                        }
                                                    }
                                                    false
                                                })
                                                .appearance(false)
                                                .bordered(false)
                                                .flush(true)
                                                .trailing_margin(px(0.))
                                                .font_family(self.text_font.clone())
                                                .h_full()
                                                .text_size(px(self.font_size))
                                                .line_height(relative(self.line_spacing)),
                                        ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("editor-pane-scrollbar")
                            .debug_selector(|| "editor-pane-scrollbar".into())
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(-32.))
                            .right(px(-32.))
                            .child(scrollbar),
                    ),
            )
            .child(font_zoom::capture(cx.entity().downgrade()))
            .when(!self.reading && self.live, |view| {
                view.child(live_objects::overlay(
                    cx.entity().downgrade(),
                    self.editor.clone(),
                    self.live_objects.clone(),
                    (
                        self.vault_root.clone(),
                        self.reference_index.clone(),
                        self.font_size,
                        self.light,
                        self.strict_line_breaks,
                        self.text_font.clone(),
                    ),
                ))
            })
            .when(!self.reading && self.live, |view| {
                view.child(live_lists::overlay(
                    self.editor.clone(),
                    self.live_lists.clone(),
                    self.font_size,
                    self.light,
                ))
            })
            .when(!self.reading && self.live, |view| {
                view.child(live_rules::overlay(
                    self.editor.clone(),
                    self.live_rules.clone(),
                    self.light,
                ))
            })
            .when(!self.reading && self.live, |view| {
                view.child(live_quotes::overlay(
                    self.editor.clone(),
                    self.live_quotes.clone(),
                    self.font_size,
                ))
            })
            .when(!self.reading && self.live, |view| {
                view.child(live_tasks::overlay(
                    cx.entity().downgrade(),
                    self.editor.clone(),
                    self.live_tasks.clone(),
                    self.font_size,
                    self.light,
                ))
            })
    }
}
