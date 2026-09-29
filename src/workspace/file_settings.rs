use super::*;
use gpui_component::{
    button::*,
    menu::{DropdownMenu, PopupMenuItem},
};
use inkstone::locations::Location;

impl Workspace {
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
                inkstone::rendering::attachment_target(&format!("file.{}", ext.to_string_lossy()))
            });
            if !supported {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let (target, markdown) = inkstone::locations::attachment_destination(
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
        use inkstone::locations::LinkFormat;
        let formats = [
            (LinkFormat::Shortest, "最短路径"),
            (LinkFormat::Relative, "相对当前笔记的路径"),
            (LinkFormat::Absolute, "相对库根目录的路径"),
        ];
        let format = self.ui.prefs.link_format;
        let weak = cx.entity().downgrade();
        let link_format = Button::new("new-link-format")
            .label(
                formats
                    .iter()
                    .find(|(value, _)| *value == format)
                    .unwrap()
                    .1,
            )
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
                .border_color(rgb(if self.ui.prefs.light {
                    0xe3e3e3
                } else {
                    0x363636
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .flex_1()
                                .child(if attachment {
                                    "附件默认存放路径"
                                } else {
                                    "新建笔记的存放位置"
                                })
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(if self.ui.prefs.light {
                                            0x777777
                                        } else {
                                            0x999999
                                        }))
                                        .child(if attachment {
                                            "设置新添加附件的存放位置。"
                                        } else {
                                            "指定新建笔记的存放路径。"
                                        }),
                                ),
                        )
                        .child(div().flex_shrink_0().child(selector)),
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
                                .child(div().w(px(190.)).child(Input::new(if attachment {
                                    &self.ui.attachment_folder_input
                                } else {
                                    &self.ui.note_folder_input
                                }))),
                        )
                    },
                )
                .when(settings.directory(None, attachment).is_err(), |s| {
                    s.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(0xe4a66a))
                            .child(settings.directory(None, attachment).unwrap_err()),
                    )
                })
        };
        div()
            .flex()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(self.settings_nav(cx))
            .child(
                div()
                    .id("settings-content")
                    .track_scroll(&self.ui.settings_scroll)
                    .relative()
                    .vertical_scrollbar(&self.ui.settings_scroll)
                    .overflow_y_scroll()
                    .min_h_0()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .p_3()
                    .flex()
                    .flex_col()
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
                            .child(
                                div().flex_1().child("内部链接类型").child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(if self.ui.prefs.light {
                                            0x777777
                                        } else {
                                            0x999999
                                        }))
                                        .child("设置链接到库内文件时使用的路径格式。"),
                                ),
                            )
                            .child(div().flex_shrink_0().child(link_format)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child("使用双链语法")
                            .child(
                                gpui_component::switch::Switch::new("use-wikilinks")
                                    .accessibility_label("使用双链语法")
                                    .checked(!self.ui.prefs.use_markdown_links)
                                    .on_click(cx.listener(|s, checked: &bool, _, cx| {
                                        s.ui.prefs.use_markdown_links = !*checked;
                                        s.sync_index_ui(cx);
                                        s.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div().flex_1().child("自动更新内部链接").child(
                                    div()
                                        .text_size(px(12.))
                                        .child("关闭时，重命名或移动后会询问是否更新链接。"),
                                ),
                            )
                            .child(
                                gpui_component::switch::Switch::new("always-update-links")
                                    .accessibility_label("自动更新内部链接")
                                    .checked(self.ui.prefs.always_update_links)
                                    .on_click(cx.listener(|s, value: &bool, _, cx| {
                                        s.ui.prefs.always_update_links = *value;
                                        s.persist_workspace(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().text_size(px(12.)).child(
                        "文件夹路径相对于当前笔记库。位置设置只影响之后创建的笔记和导入的附件。",
                    )),
            )
            .into_any_element()
    }
}
