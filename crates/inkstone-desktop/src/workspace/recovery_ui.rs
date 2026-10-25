//! Recovery hub: one source at a time, with a stable toolbar and scrollable records.
use super::accessible_text::label;
use super::focus_reveal::FocusReveal;
use super::ui::{icon, tool};
use super::*;
use gpui_component::{Disableable, Selectable, button::*};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Source {
    #[default]
    Trash,
    Drafts,
    History,
    Sync,
}

impl Workspace {
    pub(super) fn recovery_hub_is_open(&self) -> bool {
        self.ui.trash_open
            && self.ui.bulk_edit.is_none()
            && self.ui.table_editor.is_none()
            && self.ui.link_health.is_none()
            && self.ui.attachment_manager.is_none()
            && self.ui.history.is_none()
            && self.ui.conflict_review.is_none()
    }

    pub(super) fn recovery_hub_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let compact = window.viewport_size().height < px(600.);
        let palette = crate::theme::palette(self.ui.prefs.light);
        let source = self.ui.recovery_source;
        let query = self
            .ui
            .recovery_filter
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let matches =
            |path: &std::path::Path| path.to_string_lossy().to_lowercase().contains(&query);
        let trash: Vec<_> = self
            .ui
            .trash
            .iter()
            .enumerate()
            .filter(|(_, e)| matches(&e.original))
            .collect();
        let drafts: Vec<_> = self
            .recoveries
            .iter()
            .enumerate()
            .filter(|(_, e)| matches(&e.relative))
            .collect();
        let (title, description, count) = match source {
            Source::Trash => (
                "回收站",
                "找回删除的文件，让内容回到原来的位置。",
                trash.len(),
            ),
            Source::Drafts => (
                "未保存草稿",
                "查看尚未写入笔记的内容，比较后再决定是否恢复。",
                drafts.len(),
            ),
            Source::History => (
                "整库历史",
                "查找所有笔记的版本，包括已删除或重命名的旧路径。",
                0,
            ),
            Source::Sync => (
                "同步备份",
                "查看同步过程中保留的内容，按需恢复为独立副本。",
                0,
            ),
        };
        let files = matches!(source, Source::Trash | Source::Drafts);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px(px(24.))
                    .border_b_1()
                    .border_color(palette.border)
                    .children(
                        [
                            (
                                Source::Trash,
                                "trash",
                                format!("回收站  {}", self.ui.trash.len()),
                            ),
                            (
                                Source::Drafts,
                                "file",
                                format!("未保存草稿  {}", self.recoveries.len()),
                            ),
                            (Source::History, "history", "整库历史".into()),
                            (Source::Sync, "cloud", "同步备份".into()),
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (tab, symbol, name))| {
                            Button::new(("recovery-source", i))
                                .ghost()
                                .h(px(48.))
                                .px_3()
                                .rounded_none()
                                .icon(icon(symbol))
                                .label(name)
                                .selected(source == tab)
                                .when(source == tab, |b| {
                                    b.text_color(palette.accent)
                                        .border_b_2()
                                        .border_color(palette.accent)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ui.recovery_source = tab;
                                    this.ui.recovery_scroll.set_offset(point(px(0.), px(0.)));
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(24.))
                    .pt(px(16.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        label("recovery-source-title", title)
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .when(!compact, |s| {
                                        s.child(
                                            label("recovery-source-description", description)
                                                .text_sm()
                                                .text_color(palette.muted),
                                        )
                                    }),
                            )
                            .child(
                                tool("recovery-refresh", "refresh-cw", "刷新恢复记录")
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh_trash(cx))),
                            ),
                    )
                    .when(files, |s| {
                        s.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.ui.recovery_filter)),
                                )
                                .child(
                                    label("recovery-count", format!("{count} 项"))
                                        .text_sm()
                                        .text_color(palette.muted),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("trash-items")
                    .flex_1()
                    .min_h_0()
                    .px(px(24.))
                    .pb(px(12.))
                    .track_scroll(&self.ui.recovery_scroll)
                    .overflow_y_scroll()
                    .when(source == Source::Trash, |s| {
                        s.when(trash.is_empty(), |s| {
                            s.child(self.recovery_empty(!query.is_empty(), false))
                        })
                        .children(trash.into_iter().map(|(i, e)| {
                            let details = super::recovery::trash_details(
                                self.ui.trash_metadata.get(&e.stored),
                            );
                            self.recovery_file_row(
                                format!("recovery-trash-{i}"),
                                &e.original,
                                if e.directory { "folder" } else { "file" },
                                details,
                                FocusReveal::new(
                                    (ElementId::from(("restore-trash", i)), "focus"),
                                    &self.ui.recovery_scroll,
                                    Button::new(("restore-trash", i))
                                        .ghost()
                                        .icon(icon("undo"))
                                        .label("恢复")
                                        .tooltip("恢复到原位置；同名文件不会被覆盖")
                                        .accessibility_label(format!(
                                            "从回收站恢复：{}",
                                            e.original.display()
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.restore_deleted(i, cx)
                                        })),
                                ),
                            )
                        }))
                    })
                    .when(source == Source::Drafts, |s| {
                        s.when(self.recoveries_loading, |s| {
                            s.child(
                                label("recovery-drafts-loading", "正在读取草稿……")
                                    .text_color(palette.muted),
                            )
                        })
                        .when(drafts.is_empty() && !self.recoveries_loading, |s| {
                            s.child(self.recovery_empty(!query.is_empty(), true))
                        })
                        .children(drafts.into_iter().map(|(i, e)| {
                            let details = super::recovery_metadata::Metadata {
                                source: super::recovery_metadata::Source::Draft,
                                modified: Some(e.modified),
                                bytes: Some(e.bytes),
                            }
                            .label();
                            self.recovery_file_row(
                                format!("recovery-draft-{i}"),
                                &e.relative,
                                "file",
                                details,
                                FocusReveal::new(
                                    (ElementId::from(("restore-draft", i)), "focus"),
                                    &self.ui.recovery_scroll,
                                    Button::new(("restore-draft", i))
                                        .ghost()
                                        .label("查看草稿 →")
                                        .accessibility_label(format!(
                                            "比较、恢复或放弃草稿：{}",
                                            e.relative.display()
                                        ))
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            this.review_draft(i, w, cx)
                                        })),
                                ),
                            )
                        }))
                    })
                    .when(source == Source::History, |s| {
                        s.child(self.history_catalog_panel(cx))
                    })
                    .when(source == Source::Sync, |s| {
                        s.child(self.sync_recovery_panel(cx))
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(palette.border)
                    .bg(palette.surface)
                    .px(px(24.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        label(
                            "recovery-safety",
                            match source {
                                Source::Trash => "文件不会自动清理 · 恢复时不覆盖同名文件",
                                Source::Drafts => "草稿会保留到你恢复或放弃为止",
                                Source::History | Source::Sync => "恢复会创建副本，保留现有文件",
                            },
                        )
                        .text_sm()
                        .text_color(palette.muted),
                    )
                    .child(
                        Button::new("recovery-history")
                            .ghost()
                            .label("当前笔记历史 →")
                            .disabled(self.active.is_none())
                            .on_click(cx.listener(|this, _, w, cx| this.open_history(w, cx))),
                    ),
            )
            .into_any_element()
    }

    fn recovery_empty(&self, filtered: bool, draft: bool) -> AnyElement {
        let palette = crate::theme::palette(self.ui.prefs.light);
        let (title, description) = if filtered {
            ("没有找到匹配的记录", "试试更短的名称，或清空搜索查看全部。")
        } else if draft {
            ("没有待恢复的草稿", "未保存的内容会显示在这里。")
        } else {
            ("回收站是空的", "删除的文件会保留在这里，随时可以找回。")
        };
        div()
            .size_full()
            .min_h(px(180.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .size(px(56.))
                    .rounded(px(16.))
                    .bg(palette.surface)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.accent)
                    .child(
                        icon(if filtered {
                            "search"
                        } else if draft {
                            "file"
                        } else {
                            "trash"
                        })
                        .size(px(24.)),
                    ),
            )
            .child(label("recovery-empty-title", title).font_weight(FontWeight::SEMIBOLD))
            .child(
                label("recovery-empty-description", description)
                    .text_sm()
                    .text_color(palette.muted),
            )
            .into_any_element()
    }

    fn recovery_file_row(
        &self,
        id: String,
        path: &std::path::Path,
        symbol: &str,
        details: String,
        action: impl IntoElement,
    ) -> AnyElement {
        let palette = crate::theme::palette(self.ui.prefs.light);
        let full_path = path.to_string_lossy().to_string();
        div()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .flex()
            .items_center()
            .gap(px(12.))
            .py_4()
            .px_3()
            .border_b_1()
            .border_color(palette.border)
            .hover(|s| s.bg(palette.surface))
            .child(
                div()
                    .size(px(40.))
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .bg(palette.surface)
                    .text_color(palette.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(symbol).size(px(20.))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        label(
                            "file-name",
                            path.file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy()
                                .to_string(),
                        )
                        .font_weight(FontWeight::MEDIUM)
                        .truncate(),
                    )
                    .child(
                        label("file-path", full_path.clone())
                            .text_sm()
                            .text_color(palette.muted)
                            .truncate(),
                    )
                    .child(
                        label("file-details", details)
                            .text_sm()
                            .text_color(palette.muted),
                    ),
            )
            .child(action)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn filtering_restores_the_matching_file_and_source_tabs_hide_other_records(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!("inkstone-recovery-filter-{stamp}"));
        std::fs::create_dir_all(&temp).unwrap();
        let root = temp.join("vault");
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(root, temp.join("recovery")).unwrap();
        for name in ["甲.md", "乙.md"] {
            let path = PathBuf::from(name);
            vault.save(&path, None, name).unwrap();
            vault.trash_file(&path).unwrap();
        }
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.ui.trash = vault.trash_entries().unwrap();
                w.ui.trash_open = true;
                w.ui.recovery_filter
                    .update(cx, |input, cx| input.set_value("乙", window, cx));
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let selected = handle
            .update(&mut visual, |w, _, _| {
                w.ui.trash
                    .iter()
                    .position(|e| e.original == std::path::Path::new("乙.md"))
                    .unwrap()
            })
            .unwrap();
        assert!(
            visual
                .debug_bounds(if selected == 0 {
                    "recovery-trash-1"
                } else {
                    "recovery-trash-0"
                })
                .is_none()
        );
        let row = visual
            .debug_bounds(if selected == 0 {
                "recovery-trash-0"
            } else {
                "recovery-trash-1"
            })
            .unwrap();
        visual.simulate_click(
            point(row.right() - px(40.), row.center().y),
            Modifiers::default(),
        );
        visual.run_until_parked();
        assert_eq!(
            vault.read(&PathBuf::from("乙.md")).unwrap().as_deref(),
            Some("乙.md")
        );
        assert!(vault.read(&PathBuf::from("甲.md")).unwrap().is_none());
        handle
            .update(&mut visual, |w, _, cx| {
                w.ui.recovery_source = Source::Drafts;
                cx.notify();
            })
            .unwrap();
        visual.update(|w, cx| w.draw(cx).clear(cx));
        assert!(visual.debug_bounds("recovery-trash-0").is_none());
        std::fs::remove_dir_all(temp).unwrap();
    }
}
