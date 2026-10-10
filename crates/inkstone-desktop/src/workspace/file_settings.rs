use super::focus_reveal::FocusReveal;
use super::*;
use crate::theme::MIN_UI_FONT_SIZE;
use gpui_component::{
    button::*,
    menu::{DropdownMenu, PopupMenuItem},
};
use inkstone_core::locations::Location;

impl Workspace {
    pub(super) fn prepare_link_paths(&mut self) {
        let reusable = self
            .link_paths_key
            .as_ref()
            .is_some_and(|(index, format, markdown)| {
                *format == self.ui.prefs.link_format
                    && *markdown == self.ui.prefs.use_markdown_links
                    && (Arc::ptr_eq(index, &self.index)
                        || same_completion_targets(index, &self.index))
            });
        if !reusable {
            self.link_paths.clear();
        }
        // Retain the snapshot so its address cannot be recycled underneath the
        // cache. Body/time-only updates reuse completions across all open panes.
        self.link_paths_key = Some((
            self.index.clone(),
            self.ui.prefs.link_format,
            self.ui.prefs.use_markdown_links,
        ));
    }
    pub(super) fn cached_link_paths(
        &mut self,
        from: &std::path::Path,
    ) -> Arc<Vec<crate::editor_links::CompletionPath>> {
        self.prepare_link_paths();
        if let Some(paths) = self.link_paths.get(from) {
            return paths.clone();
        }
        let paths = self.link_paths_for(from);
        self.link_paths.insert(from.to_path_buf(), paths.clone());
        paths
    }
    pub(super) fn link_paths_for(
        &self,
        from: &std::path::Path,
    ) -> Arc<Vec<crate::editor_links::CompletionPath>> {
        let targets = self.ui.prefs.link_format.note_targets(&self.index, from);
        let mut entries: Vec<_> = self
            .index
            .notes
            .keys()
            .zip(targets)
            .flat_map(|(path, mut target)| {
                let markdown = self.ui.prefs.use_markdown_links;
                if markdown {
                    if target.contains('/') && !target.starts_with('.') && !target.starts_with('/')
                    {
                        target.insert(0, '/');
                    }
                    target.push_str(".md");
                }
                let entry = crate::editor_links::CompletionPath {
                    label: path.with_extension("").to_string_lossy().replace('\\', "/"),
                    target,
                    markdown,
                    alias: None,
                    anchors: crate::editor_links::anchors_from_parsed(
                        &self.index.notes[path].parsed,
                    ),
                    fragment: None,
                };
                let mut suggestions = vec![entry.clone()];
                let mut seen = std::collections::HashSet::new();
                for alias in &self.index.notes[path].parsed.aliases {
                    if alias.trim().is_empty() || !seen.insert(alias.to_lowercase()) {
                        continue;
                    }
                    let mut suggestion = entry.clone();
                    suggestion.alias = Some(alias.clone());
                    if !suggestion.markdown && alias.contains(['|', '[', ']', '\n', '\r']) {
                        suggestion.markdown = true;
                        if suggestion.target.contains('/')
                            && !suggestion.target.starts_with('.')
                            && !suggestion.target.starts_with('/')
                        {
                            suggestion.target.insert(0, '/');
                        }
                        suggestion.target.push_str(".md");
                    }
                    suggestions.push(suggestion);
                }
                suggestions
            })
            .collect();
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for path in &self.index.files {
            *counts
                .entry(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase(),
                )
                .or_default() += 1;
        }
        for path in &self.index.files {
            let supported = path.extension().is_some_and(|ext| {
                inkstone_core::rendering::attachment_target(&format!(
                    "file.{}",
                    ext.to_string_lossy()
                ))
            });
            if !supported {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let (target, markdown) = inkstone_core::locations::attachment_destination(
                from,
                path,
                self.ui.prefs.link_format,
                self.ui.prefs.use_markdown_links,
                counts.get(&name) == Some(&1),
            );
            entries.push(crate::editor_links::CompletionPath {
                label: path.to_string_lossy().replace('\\', "/"),
                target,
                markdown,
                alias: None,
                ..Default::default()
            });
        }
        entries.sort_by_cached_key(|entry| entry.label.to_lowercase());
        Arc::new(entries)
    }
    pub(super) fn prepare_file_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = &self.ui.prefs.locations;
        self.ui.note_folder_input.update(cx, |input, cx| {
            input.set_value(settings.note_folder.clone(), window, cx)
        });
        self.ui.attachment_folder_input.update(cx, |input, cx| {
            input.set_value(settings.attachment_folder.clone(), window, cx)
        });
    }
    pub(super) fn file_settings_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        use inkstone_core::locations::LinkFormat;
        let formats = [
            (LinkFormat::Shortest, "最短路径"),
            (LinkFormat::Relative, "相对当前笔记的路径"),
            (LinkFormat::Absolute, "相对库根目录的路径"),
        ];
        let format = self.ui.prefs.link_format;
        let weak = cx.entity().downgrade();
        let format_label = formats
            .iter()
            .find(|(value, _)| *value == format)
            .unwrap()
            .1;
        let link_format = Button::new("new-link-format")
            .accessibility_label(format!("内部链接类型：{format_label}"))
            .label(format_label)
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in formats {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(value == format).on_click(
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.ui.prefs.link_format = value;
                                this.sync_index_ui(cx);
                                this.persist_workspace(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
        let row = |attachment: bool, cx: &mut Context<Self>| {
            let settings = &self.ui.prefs.locations;
            let mode = if attachment {
                settings.attachments
            } else {
                settings.notes
            };
            let modes = [
                (Location::Root, "库根目录"),
                (Location::Current, "当前笔记所在目录"),
                (Location::Folder, "指定文件夹"),
                (Location::Subfolder, "当前目录下的子文件夹"),
            ];
            let label = modes.iter().find(|(m, _)| *m == mode).unwrap().1;
            let weak = cx.entity().downgrade();
            let selector = Button::new(if attachment {
                "attachment-location"
            } else {
                "note-location"
            })
            .label(label)
            .tooltip("仅影响之后创建的笔记和导入的附件；文件夹路径相对于当前笔记库。")
            .accessibility_label(format!(
                "{}：{label}",
                if attachment {
                    "附件默认存放路径"
                } else {
                    "新建笔记的存放位置"
                }
            ))
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in modes
                    .into_iter()
                    .filter(|(v, _)| attachment || *v != Location::Subfolder)
                {
                    let weak = weak.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(mode == value).on_click(
                        move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                if attachment {
                                    this.ui.prefs.locations.attachments = value;
                                } else {
                                    this.ui.prefs.locations.notes = value;
                                }
                                this.persist_workspace(cx);
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
            div()
                .flex()
                .flex_col()
                .gap_4()
                .pb_4()
                .border_b_1()
                .border_color(crate::theme::palette(self.ui.prefs.light).border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(div().flex_1().child(if attachment {
                            "附件默认存放路径"
                        } else {
                            "新建笔记的存放位置"
                        }))
                        .child(FocusReveal::new(
                            if attachment {
                                "attachment-location-focus"
                            } else {
                                "note-location-focus"
                            },
                            &self.ui.settings_scroll,
                            selector,
                        )),
                )
                .when(
                    matches!(mode, Location::Folder | Location::Subfolder),
                    |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(div().flex_1().child(if attachment {
                                    "附件文件夹路径"
                                } else {
                                    "存放新建笔记的文件夹"
                                }))
                                .child(FocusReveal::new(
                                    if attachment {
                                        "attachment-folder-focus"
                                    } else {
                                        "note-folder-focus"
                                    },
                                    &self.ui.settings_scroll,
                                    div().w(px(190.)).child(Input::new(if attachment {
                                        &self.ui.attachment_folder_input
                                    } else {
                                        &self.ui.note_folder_input
                                    })),
                                )),
                        )
                    },
                )
                .when(settings.directory(None, attachment).is_err(), |s| {
                    s.child(
                        div()
                            .text_size(px(MIN_UI_FONT_SIZE))
                            .text_color(rgb(0xe4a66a))
                            .child(settings.directory(None, attachment).unwrap_err()),
                    )
                })
        };
        div()
            .flex()
            .gap_0()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                self.settings_content()
                    .gap_5()
                    .child(row(false, cx))
                    .child(row(true, cx))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("链接"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(div().flex_1().child("内部链接类型"))
                            .child(FocusReveal::new(
                                "link-format-focus",
                                &self.ui.settings_scroll,
                                link_format,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child("使用双链语法")
                            .child(FocusReveal::new(
                                "use-wikilinks-focus",
                                &self.ui.settings_scroll,
                                super::settings_ui::setting_switch("use-wikilinks")
                                    .accessibility_label("使用双链语法")
                                    .checked(!self.ui.prefs.use_markdown_links)
                                    .on_click(cx.listener(|s, checked: &bool, _, cx| {
                                        s.ui.prefs.use_markdown_links = !*checked;
                                        s.sync_index_ui(cx);
                                        s.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                self.settings_label(
                                    "update-links-label",
                                    "自动更新内部链接",
                                    "关闭时，重命名或移动后会询问是否更新链接。",
                                )
                                .flex_1(),
                            )
                            .child(FocusReveal::new(
                                "always-update-links-focus",
                                &self.ui.settings_scroll,
                                super::settings_ui::setting_switch("always-update-links")
                                    .accessibility_label("自动更新内部链接")
                                    .checked(self.ui.prefs.always_update_links)
                                    .on_click(cx.listener(|s, value: &bool, _, cx| {
                                        s.ui.prefs.always_update_links = *value;
                                        s.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            )),
                    ),
            )
            .into_any_element()
    }
}

/// Completion entries use paths, aliases and anchor names, not body positions.
fn same_completion_targets(previous: &Index, next: &Index) -> bool {
    previous.files == next.files
        && previous.notes.len() == next.notes.len()
        && previous
            .notes
            .iter()
            .zip(&next.notes)
            .all(|((a, old), (b, new))| {
                a == b
                    && (Arc::ptr_eq(old, new)
                        || (old.parsed.aliases == new.parsed.aliases
                            && old.parsed.headings.iter().map(|h| &h.title).eq(new
                                .parsed
                                .headings
                                .iter()
                                .map(|h| &h.title))
                            && old.parsed.blocks.iter().map(|b| &b.id).eq(new
                                .parsed
                                .blocks
                                .iter()
                                .map(|b| &b.id))))
            })
}
