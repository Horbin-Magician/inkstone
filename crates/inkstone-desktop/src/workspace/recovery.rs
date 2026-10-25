use super::focus_reveal::FocusReveal;
use super::*;
use crate::workspace::accessible_text::label as history_text;
use gpui_component::{
    Disableable, Selectable,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
};
use inkstone_core::vault::{HistoryEntry, Recovery};

const VERSIONS_PER_PAGE: usize = 5;

pub(super) struct Browser {
    page: usize,
    scroll: ScrollHandle,
    pager_focus: FocusHandle,
    path: PathBuf,
    current: SharedString,
    current_available: bool,
    entries: Vec<HistoryEntry>,
    selected: Option<usize>,
    record: Option<Arc<Recovery>>,
    baseline: bool,
    difference: bool,
    loading: bool,
    message: String,
    preview: Entity<TextareaState>,
    draft_entry: Option<RecoveryEntry>,
    draft_review: bool,
    confirm_discard: bool,
}

pub(super) fn preview_text(text: &str) -> String {
    const LIMIT: usize = 128 * 1024;
    if text.len() <= LIMIT {
        return text.into();
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n……预览已截断，恢复副本仍包含完整正文。", &text[..end])
}

pub(super) fn trash_details(metadata: Option<&inkstone_core::vault::TrashMetadata>) -> String {
    super::recovery_metadata::Metadata {
        source: super::recovery_metadata::Source::Trash,
        modified: metadata.and_then(|m| m.modified),
        bytes: metadata.and_then(|m| m.bytes),
    }
    .label()
}

fn retention_label(policy: inkstone_core::vault::Retention) -> String {
    let age = if policy.days == 0 {
        "不限保留天数".into()
    } else {
        format!("保留 {} 天", policy.days)
    };
    let capacity = if policy.max_mib == 0 {
        "不限容量".into()
    } else {
        format!("每库 {} MiB 软上限", policy.max_mib)
    };
    format!("成功记录：{age} · {capacity}；每篇至少保留最新成功记录。未保存草稿不自动清理。")
}

impl Workspace {
    pub(super) fn show_recovery_hub(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.bulk_edit = None;
        self.ui.link_health = None;
        self.ui.attachment_manager = None;
        self.ui.history = None;
        self.ui.conflict_review = None;
        self.ui.trash_open = true;
        self.ui.recovery_scroll.set_offset(point(px(0.), px(0.)));
        window.focus(&self.ui.modal_focus, cx);
        // Refresh advances the request identity, invalidating any old preview.
        self.refresh_trash(cx);
        cx.notify();
    }

    pub(super) fn refresh_trash_metadata(&mut self, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.ui.trash_metadata.clear();
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx
            .background_executor()
            .spawn(async move { vault.trash_inventory() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                if let Ok(records) = result {
                    this.ui.trash_metadata = records
                        .into_iter()
                        .map(|r| (r.entry.stored, r.metadata))
                        .collect();
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn reviewing_draft(&self) -> bool {
        self.ui.history.as_ref().is_some_and(|b| b.draft_review)
    }
    pub(super) fn review_draft(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.recoveries.get(index).cloned() else {
            return;
        };
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.close_overlays(window, cx);
        let preview = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).rows(12);
            state.set_readonly(true, cx);
            state
        });
        self.ui.history = Some(Browser {
            page: 0,
            scroll: ScrollHandle::new(),
            pager_focus: cx.focus_handle(),
            path: entry.relative.clone(),
            current: "".into(),
            current_available: true,
            entries: vec![],
            selected: None,
            record: None,
            baseline: false,
            difference: true,
            loading: true,
            message: String::new(),
            preview,
            draft_entry: None,
            draft_review: true,
            confirm_discard: false,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let entry = vault.read_draft(&entry)?;
            let current = vault.read(&entry.record.relative)?;
            let metadata = std::fs::metadata(&entry.journal)?;
            Ok::<_, VaultError>((entry, current, metadata))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(browser) = &mut this.ui.history else {
                    return;
                };
                browser.loading = false;
                match result {
                    Ok((entry, current, metadata)) => {
                        browser.message = if current.is_none() {
                            "原文件已不存在；恢复仍会创建独立副本。".into()
                        } else {
                            String::new()
                        };
                        browser.current = current.unwrap_or_default().into();
                        browser.draft_entry = Some(entry.clone());
                        browser.entries = vec![HistoryEntry {
                            journal: entry.journal,
                            modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                            bytes: metadata.len(),
                            saved: false,
                        }];
                        browser.selected = Some(0);
                        browser.record = Some(Arc::new(entry.record));
                        this.update_history_preview(window, cx);
                    }
                    Err(error) => {
                        browser.message = format!("无法读取草稿，请重新打开恢复入口：{error}")
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn finish_draft_review(
        &mut self,
        discard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(browser) = &mut self.ui.history else {
            return;
        };
        if browser.loading || browser.record.is_none() {
            return;
        }
        let Some(entry) = browser.draft_entry.clone() else {
            return;
        };
        if discard && !browser.confirm_discard {
            browser.confirm_discard = true;
            browser.message = "再次点击确认放弃将删除这一条草稿，原文件与其他记录保留。".into();
            cx.notify();
            return;
        }
        // A recovery always restores the recorded draft, regardless of preview mode.
        browser.loading = true;
        let write_ticket = self.file_writes.begin();
        let reserved = self
            .tabs
            .iter()
            .map(|tab| tab.path.clone())
            .collect::<Vec<_>>();
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let journal = entry.journal.clone();
        let task = cx.background_executor().spawn(async move {
            vault.validate_draft(&entry)?;
            if discard {
                vault.discard_draft(&entry)?;
                Ok::<_, VaultError>((None, None))
            } else {
                let copy =
                    vault.duplicate_note(&entry.record.relative, &entry.record.draft, &reserved)?;
                let cleanup = vault
                    .discard_draft(&entry)
                    .err()
                    .map(|error| error.to_string());
                Ok((Some(copy), cleanup))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(write_ticket);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((copy, cleanup)) => {
                        if cleanup.is_none() {
                            this.recoveries.retain(|entry| entry.journal != journal);
                        }
                        if this.ui.recovery_refresh == request {
                            this.close_overlays(window, cx);
                        }
                        if let Some((path, receipt)) = copy {
                            this.note_indexed_change(path.clone(), receipt.text.clone(), cx);
                            if !this.files.contains(&path) {
                                this.files.push(path.clone());
                                this.files.sort();
                            }
                            this.add_tab(path, Some(receipt.text), false, window, cx);
                            this.schedule_auto_sync(true);
                            this.notifications.publish(cleanup.map_or_else(
                                || "草稿已恢复为独立副本，原文件保留，所选草稿已清理。".into(),
                                |error| format!("副本已保存，原草稿清理失败：{error}"),
                            ));
                        } else {
                            this.notifications
                                .publish("已放弃所选草稿，原文件与其他记录保留。".into());
                        }
                    }
                    Err(error) => {
                        this.notifications
                            .publish(format!("草稿处理失败，恢复记录保留：{error}"));
                        if this.ui.recovery_refresh == request
                            && let Some(browser) = &mut this.ui.history
                        {
                            browser.loading = false;
                            browser.message = this.notifications.text().to_owned();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.current_pane() else {
            return;
        };
        let path = pane.read(cx).current_path.clone();
        if path.as_os_str().is_empty() {
            return;
        }
        if pane.read(cx).editor.read(cx).is_composing() {
            self.notifications
                .publish("请完成输入法组词后查看历史。".into());
            cx.notify();
            return;
        }
        self.open_history_path(path, window, cx);
    }

    pub(super) fn open_history_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let pane = self
            .current_pane()
            .filter(|p| p.read(cx).current_path == path)
            .or_else(|| {
                self.tabs
                    .iter()
                    .find(|tab| tab.path == path)
                    .map(|tab| tab.pane.clone())
            });
        if pane
            .as_ref()
            .is_some_and(|p| p.read(cx).editor.read(cx).is_composing())
        {
            self.notifications
                .publish("请完成输入法组词后查看历史。".into());
            cx.notify();
            return;
        }
        let current = pane.map(|p| p.read(cx).editor.read(cx).value());
        self.close_overlays(window, cx);
        let preview = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).rows(12);
            state.set_readonly(true, cx);
            state
        });
        self.ui.history = Some(Browser {
            page: 0,
            scroll: ScrollHandle::new(),
            pager_focus: cx.focus_handle(),
            path: path.clone(),
            current: current.clone().unwrap_or_default(),
            current_available: current.is_some(),
            entries: vec![],
            selected: None,
            record: None,
            baseline: false,
            difference: false,
            loading: true,
            message: String::new(),
            preview,
            draft_entry: None,
            draft_review: false,
            confirm_discard: false,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let current =
                current.or_else(|| vault.read(&path).ok().flatten().map(SharedString::from));
            (vault.history(&path), current)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (result, current) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(browser) = &mut this.ui.history else {
                    return;
                };
                browser.current_available = current.is_some();
                browser.current = current.unwrap_or_default();
                browser.loading = false;
                match result {
                    Ok(entries) => browser.entries = entries,
                    Err(error) => browser.message = error.to_string(),
                }
                if !browser.entries.is_empty() {
                    this.select_history(0, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_history(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(browser) = &mut self.ui.history else {
            return;
        };
        let Some(entry) = browser.entries.get(index).cloned() else {
            return;
        };
        browser.selected = Some(index);
        browser.page = index / VERSIONS_PER_PAGE;
        browser.record = None;
        browser.loading = true;
        browser.message.clear();
        browser
            .preview
            .update(cx, |s, cx| s.set_value("", window, cx));
        let path = browser.path.clone();
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { vault.read_history(&path, &entry.journal) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(browser) = &mut this.ui.history else {
                    return;
                };
                browser.loading = false;
                match result {
                    Ok(record) => {
                        if record.baseline.is_none() {
                            browser.baseline = false;
                        }
                        browser.record = Some(Arc::new(record));
                        this.update_history_preview(window, cx);
                    }
                    Err(error) => {
                        browser.message =
                            format!("无法读取该版本：{error}。可重新打开历史刷新列表。")
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn update_history_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(browser) = &mut self.ui.history else {
            return;
        };
        let Some(record) = browser.record.clone() else {
            return;
        };
        let baseline = browser.baseline;
        let current = browser.current.clone();
        let difference = browser.difference;
        let draft_review = browser.draft_review;
        browser.loading = true;
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            let text = if baseline {
                record.baseline.as_ref().unwrap_or(&record.draft)
            } else {
                &record.draft
            };
            if difference {
                if text.as_str() == current.as_ref() {
                    return if draft_review {
                        "与打开比较时的磁盘正文一致。"
                    } else {
                        "与打开历史时的当前正文一致。"
                    }
                    .to_string();
                }
                let diff = similar::TextDiff::configure()
                    .timeout(Duration::from_millis(100))
                    .diff_lines(text.as_str(), current.as_ref());
                preview_text(
                    &diff
                        .unified_diff()
                        .context_radius(3)
                        .header(
                            if draft_review {
                                "所选草稿"
                            } else {
                                "所选历史版本"
                            },
                            if draft_review {
                                "打开比较时的磁盘正文"
                            } else {
                                "打开历史时的当前正文"
                            },
                        )
                        .to_string(),
                )
            } else {
                preview_text(text)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let text = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(browser) = &mut this.ui.history else {
                    return;
                };
                browser.loading = false;
                browser
                    .preview
                    .update(cx, |s, cx| s.set_value(text, window, cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn restore_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.history.as_ref().is_some_and(|b| b.draft_review) {
            self.finish_draft_review(false, window, cx);
            return;
        }
        let Some(browser) = &self.ui.history else {
            return;
        };
        if browser.loading {
            return;
        }
        let Some(record) = &browser.record else {
            return;
        };
        let text = if browser.baseline {
            record.baseline.as_ref().unwrap_or(&record.draft)
        } else {
            &record.draft
        }
        .clone();
        let path = browser.path.clone();
        self.close_overlays(window, cx);
        self.restore_text_as_copy(&path, text, window, cx);
    }

    pub(super) fn history_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        use super::ui::{icon, tool};
        let Some(browser) = &self.ui.history else {
            return div().into_any_element();
        };
        let palette = crate::theme::palette(self.ui.prefs.light);
        let bytes: u64 = browser.entries.iter().map(|e| e.bytes).sum();
        let pages = browser.entries.len().div_ceil(VERSIONS_PER_PAGE);
        let selected = browser.selected.and_then(|i| browser.entries.get(i));
        let note_name = browser
            .path
            .file_name()
            .unwrap_or(browser.path.as_os_str())
            .to_string_lossy()
            .to_string();
        let metadata = selected
            .map(|entry| {
                super::recovery_metadata::Metadata {
                    source: if entry.saved {
                        super::recovery_metadata::Source::History
                    } else {
                        super::recovery_metadata::Source::Draft
                    },
                    modified: Some(entry.modified),
                    bytes: Some(entry.bytes),
                }
                .label()
            })
            .unwrap_or_else(|| "选择左侧记录，查看此版本的内容".into());
        let policy = if browser.draft_review {
            "未保存草稿会保留到恢复或放弃为止。".to_owned()
        } else {
            retention_label(self.ui.prefs.history)
        };
        let help = format!(
            "{policy}\n\n共 {} 条记录，{:.1} KiB。容量按记录文件长度统计，包含正文、元数据及可能的保存前正文，不等于当前笔记大小或磁盘实际分配空间。",
            browser.entries.len(),
            bytes as f64 / 1024.
        );
        div().debug_selector(|| "history-shell".into()).flex().flex_col().flex_1().min_h_0().overflow_hidden()
            .child(div().flex_shrink_0().px(px(24.)).py(px(16.)).flex().items_center().gap_3()
                .border_b_1().border_color(palette.border)
                .child(div().size(px(40.)).flex_shrink_0().rounded(px(12.)).bg(palette.surface)
                    .flex().items_center().justify_center().text_color(palette.accent).child(icon("history").size(px(22.))))
                .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                    .child(history_text("history-note-name", note_name).font_weight(FontWeight::SEMIBOLD).truncate())
                    .child(history_text("history-owner-path", format!("{} · {}", if browser.draft_review { "草稿恢复" } else { "版本历史" }, browser.path.display()))
                        .text_sm().text_color(palette.muted).truncate()))
                .child(Button::new("history-back-to-recovery").ghost().compact().label("文件恢复中心")
                    .on_click(cx.listener(|this, _, w, cx| this.show_recovery_hub(w, cx))))
                .child(tool("close-history", "x", "关闭 Esc").on_click(cx.listener(|this, _, w, cx| this.close_overlays(w, cx)))))
            .child(div().debug_selector(|| "history-columns".into()).flex().items_stretch().flex_1().min_h_0()
                .child(div().w(px(if window.viewport_size().width < px(1000.) { 224. } else { 264. }))
                    .flex_shrink_0().flex().flex_col().bg(palette.surface).border_r_1().border_color(palette.border)
                    .child(div().flex_shrink_0().px(px(16.)).py(px(12.)).flex().items_center().justify_between()
                        .child(history_text("history-list-title", format!("版本记录 · {}", browser.entries.len())).text_sm().font_weight(FontWeight::MEDIUM))
                        .child(tool("history-policy", "info", help)))
                    .child(div().id("history-content").flex_1().min_h_0().track_scroll(&browser.scroll).overflow_y_scroll()
                        .px(px(10.)).pb(px(12.)).flex().flex_col().gap_2()
                        .when(browser.entries.is_empty(), |s| s.child(history_text("history-empty", if browser.loading { "正在读取版本……" } else { "暂无历史版本\n修改并保存后，版本会显示在这里。" }).p_3().text_sm().text_color(palette.muted)))
                        .children(browser.entries.iter().enumerate().skip(browser.page * VERSIONS_PER_PAGE).take(VERSIONS_PER_PAGE).map(|(i, entry)| {
                            let date: chrono::DateTime<chrono::Local> = entry.modified.into();
                            let active = browser.selected == Some(i);
                            div().debug_selector(move || format!("history-version-{i}"))
                                .child(FocusReveal::new((ElementId::from(("history-version", i)), "focus"), &browser.scroll,
                                    Button::new(("history-version", i)).ghost().w_full().h_auto().p(px(12.))
                                        .rounded(px(8.)).border_l_2().border_color(if active { palette.accent } else { palette.surface })
                                        .when(active, |s| s.bg(palette.selected)).selected(active)
                                        .accessibility_label(format!("版本 {} · {} · {}", i + 1, date.format("%Y-%m-%d %H:%M:%S%.3f"), if entry.saved { "已保存" } else { "未保存草稿" }))
                                        .child(div().w_full().min_w_0().flex().flex_col().gap_2()
                                            .child(div().flex().items_center().justify_between().gap_2()
                                                .child(div().font_weight(FontWeight::SEMIBOLD).child(date.format("%H:%M:%S").to_string()))
                                                .child(div().text_sm().text_color(palette.muted).child(format!("#{:02}", i + 1))))
                                            .child(div().flex().flex_wrap().items_center().justify_between().gap_1().text_size(px(14.)).text_color(palette.muted)
                                                .child(date.format("%Y-%m-%d").to_string())
                                                .child(div().when(!entry.saved, |s| s.text_color(palette.accent)).child(if entry.saved { "已保存" } else { "草稿" }))))
                                        .on_click(cx.listener(move |this, _, w, cx| this.select_history(i, w, cx)))))
                        }))
                        .when(pages > 1, |s| s.child(div().track_focus(&browser.pager_focus).flex().flex_wrap().items_center().justify_between().gap_1().pt_2()
                            .children([false, true].into_iter().map(|next| {
                                let id = ("history-page", usize::from(next));
                                div().debug_selector(move || format!("history-page-{next}"))
                                    .child(FocusReveal::new((ElementId::from(id), "focus"), &browser.scroll,
                                        Button::new(id).ghost().compact().label(if next { "下一页 →" } else { "← 上一页" })
                                            .accessibility_label(if next { "历史下一页" } else { "历史上一页" })
                                            .disabled(if next { browser.page + 1 >= pages } else { browser.page == 0 })
                                            .on_click(cx.listener(move |this, _, w, cx| {
                                                let Some(browser) = &mut this.ui.history else { return; };
                                                let last = browser.entries.len().saturating_sub(1) / VERSIONS_PER_PAGE;
                                                browser.page = if next { browser.page.saturating_add(1).min(last) } else { browser.page.saturating_sub(1) };
                                                browser.scroll.set_offset(point(px(0.), px(0.)));
                                                w.focus(&browser.pager_focus, cx);
                                                cx.notify();
                                            }))))
                            }))
                            .child(history_text("history-page-status", format!("{} / {pages}", browser.page + 1)).w_full().text_center().text_sm().text_color(palette.muted))))))
                .child(div().debug_selector(|| "history-detail".into()).flex_1().h_full().min_w_0().min_h_0().flex().flex_col()
                    .child(div().flex_shrink_0().px(px(20.)).py(px(12.)).flex().flex_wrap().items_center().justify_between().gap_2()
                        .child(div().flex().items_center().gap_1().p_1().rounded(px(8.)).bg(palette.surface)
                            .children([false, true].into_iter().map(|baseline| {
                                Button::new(if baseline { "history-before" } else { "history-draft" }).ghost().compact()
                                    .label(if baseline { "保存前" } else { "记录内容" })
                                    .selected(browser.baseline == baseline)
                                    .when(browser.baseline == baseline, |s| s.bg(palette.selected).text_color(palette.accent))
                                    .disabled(browser.record.is_none() || browser.loading || (baseline && browser.record.as_ref().is_none_or(|r| r.baseline.is_none())))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        if let Some(browser) = &mut this.ui.history { browser.baseline = baseline; }
                                        this.update_history_preview(w, cx);
                                    }))
                            })))
                        .child(Button::new("history-diff").ghost().compact().icon(icon("split-horizontal"))
                            .label(if browser.draft_review { "对比磁盘正文" } else { "对比当前正文" })
                            .selected(browser.difference).when(browser.difference, |s| s.bg(palette.selected).text_color(palette.accent))
                            .tooltip("差异以打开比较时的正文为准")
                            .disabled(browser.record.is_none() || !browser.current_available || browser.loading)
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(browser) = &mut this.ui.history { browser.difference = !browser.difference; }
                                this.update_history_preview(w, cx);
                            }))))
                    .child(div().flex_shrink_0().px(px(24.)).pb(px(12.)).flex().flex_col().gap_1()
                        .child(history_text("history-selected-metadata", if browser.difference { format!("− 所选版本　 + {}", if browser.draft_review { "打开比较时的磁盘正文" } else { "打开历史时的当前正文" }) } else { metadata }).text_sm().text_color(palette.muted).truncate())
                        .when_some(browser.record.as_ref().filter(|r| r.relative != browser.path), |s, r| s.child(history_text("history-original-path", format!("记录原路径：{}", r.relative.display())).text_sm().text_color(palette.muted).truncate())))
                    .child(div().id("history-preview-pane").debug_selector(|| "history-preview-pane".into()).flex_1().min_h_0().min_w_0().mx(px(20.)).mb(px(16.))
                        .border_1().border_color(palette.border).rounded(px(10.)).overflow_hidden().flex().flex_col()
                        .when(browser.record.is_some() && !browser.loading, |s| s.child(Textarea::new(&browser.preview)
                            .readonly(true).appearance(false).bordered(false).h(relative(1.)).flex_1().min_h_0()
                            .aria_label(if browser.difference { "版本差异预览" } else { "版本内容预览" })))
                        .when(browser.record.is_none() || browser.loading, |s| s.child(div().flex_1().flex().flex_col().items_center().justify_center().gap_3().p_4().text_color(palette.muted)
                            .child(icon("history").size(px(28.)))
                            .child(history_text("history-preview-placeholder", if browser.loading { "正在读取版本……" } else { "选择一个版本，内容将在这里显示" }).text_sm()))))
                    .when(!browser.message.is_empty(), |s| s.child(history_text("history-message", browser.message.clone()).flex_shrink_0().px(px(24.)).pb_2().text_sm()))
                    .when(!browser.current_available && !browser.loading, |s| s.child(history_text("history-missing-current", "原路径没有当前正文，仍可恢复为副本。").flex_shrink_0().px(px(24.)).pb_2().text_sm().text_color(palette.muted)))
                    .child(div().debug_selector(|| "history-restore-footer".into()).flex_shrink_0().border_t_1().border_color(palette.border)
                        .px(px(20.)).py(px(12.)).flex().flex_wrap().items_center().justify_between().gap_2()
                        .child(history_text("history-restore-help", if browser.draft_review { "恢复为副本后，清理所选草稿" } else if browser.baseline { "恢复保存前内容，保留原笔记" } else { "创建独立副本，保留原笔记" }).text_sm().text_color(palette.muted))
                        .child(div().flex().items_center().gap_2()
                            .when(browser.draft_review, |s| s.child(Button::new("draft-discard").ghost().compact()
                                .label(if browser.confirm_discard { "确认放弃草稿" } else { "放弃草稿" })
                                .disabled(browser.loading || browser.record.is_none())
                                .on_click(cx.listener(|this, _, w, cx| this.finish_draft_review(true, w, cx)))))
                            .child(Button::new("history-restore").primary().icon(icon("undo"))
                                .label(if browser.draft_review { "恢复草稿为副本" } else { "恢复为新笔记" })
                                .disabled(browser.record.is_none() || browser.loading)
                                .on_click(cx.listener(|this, _, w, cx| this.restore_history(w, cx))))))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn unknown_trash_size_is_not_reported_as_empty() {
        assert!(trash_details(None).contains("大小未知"));
        let empty = inkstone_core::vault::TrashMetadata {
            modified: None,
            bytes: Some(0),
        };
        assert!(trash_details(Some(&empty)).contains("0.0 KiB"));
        assert!(trash_details(Some(&empty)).contains("时间未知"));
    }

    #[test]
    fn retention_text_matches_configured_limits_including_unlimited() {
        use inkstone_core::vault::Retention;
        let default = retention_label(Retention::default());
        assert!(default.contains("30 天") && default.contains("128 MiB"));
        let custom = retention_label(Retention {
            days: 90,
            max_mib: 512,
        });
        assert!(custom.contains("90 天") && custom.contains("512 MiB"));
        assert!(!custom.contains("30 天") && !custom.contains("128 MiB"));
        let unlimited = retention_label(Retention {
            days: 0,
            max_mib: 0,
        });
        assert!(unlimited.contains("不限保留天数") && unlimited.contains("不限容量"));
        assert!(unlimited.contains("未保存草稿不自动清理"));
    }

    #[test]
    fn preview_limit_preserves_unicode_boundaries() {
        let text = "中文😀".repeat(20_000);
        let preview = preview_text(&text);
        let shown = preview.split("\n\n……").next().unwrap();
        assert!(text.starts_with(shown));
        assert!(shown.len() <= 128 * 1024);
        assert!(preview.contains("恢复副本仍包含完整正文"));
        assert_eq!(preview_text("a\r\n😀"), "a\r\n😀");
    }

    #[gpui::test]
    fn recovery_dialog_traps_tabs_with_application_root(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        visual.update(|window, cx| {
            workspace.update(cx, |w, cx| w.show_recovery_hub(window, cx));
        });
        visual.run_until_parked();
        for key in ["shift-tab", "tab"] {
            for step in 0..12 {
                visual.update(|window, cx| window.draw(cx).clear(cx));
                let keystroke = Keystroke::parse(key).unwrap();
                visual.simulate_event(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                });
                visual.simulate_event(KeyUpEvent { keystroke });
                visual.update(|window, cx| {
                    assert!(
                        workspace
                            .read(cx)
                            .ui
                            .modal_focus
                            .contains_focused(window, cx),
                        "application root escaped dialog on {key} {step}"
                    );
                });
            }
        }
    }

    #[gpui::test]
    fn history_mouse_selection_keeps_reverse_tab_inside_dialog(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-history-mouse-focus-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let path = PathBuf::from("note.md");
        vault.save(&path, None, "saved").unwrap();
        vault.journal(&path, Some("saved"), "draft").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(path.clone(), Some("saved".into()), false, window, cx);
                w.open_history(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1200.), px(820.)));
        visual.update(|w, cx| w.draw(cx).clear(cx));
        let row = visual.debug_bounds("history-version-1").unwrap();
        visual.simulate_click(row.center(), Modifiers::default());
        visual.run_until_parked();
        // Native pointer/accessibility transitions can leave focus at the workspace
        // root. Either Tab direction must re-enter the dialog, not its background.
        for key in ["tab", "shift-tab"] {
            handle
                .update(&mut visual, |w, window, cx| {
                    window.focus(&w.ui.workspace_focus, cx);
                })
                .unwrap();
            for step in 0..12 {
                let keystroke = Keystroke::parse(key).unwrap();
                visual.simulate_event(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                });
                visual.simulate_event(KeyUpEvent { keystroke });
                visual.update(|w, cx| w.draw(cx).clear(cx));
                handle
                    .update(&mut visual, |w, window, cx| {
                        assert_eq!(w.ui.history.as_ref().unwrap().selected, Some(1));
                        assert!(
                            w.ui.modal_focus.contains_focused(window, cx),
                            "focus escaped after mouse selection / {key} {step}"
                        );
                    })
                    .unwrap();
            }
        }
        assert_eq!(vault.read(&path).unwrap().as_deref(), Some("saved"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn recovery_hub_return_invalidates_preview_and_preserves_dirty_document(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-recovery-return-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let path = PathBuf::from("note.md");
        vault.save(&path, None, "disk").unwrap();
        let journal = vault.journal(&path, Some("disk"), "recoverable").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(path.clone(), Some("disk".into()), false, window, cx);
                w.tabs[0]
                    .save
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("unsaved", window, cx));
                w.flush_document_views(window, cx);
            })
            .unwrap();
        for draft in [false, true] {
            for wait_for_preview in [false, true] {
                handle
                    .update(cx, |w, window, cx| {
                        if draft {
                            w.recoveries = vault.recovery_summaries().unwrap();
                            let index = w
                                .recoveries
                                .iter()
                                .position(|e| e.journal == journal)
                                .unwrap();
                            w.review_draft(index, window, cx);
                        } else {
                            w.open_history_path(path.clone(), window, cx);
                        }
                    })
                    .unwrap();
                if wait_for_preview {
                    cx.run_until_parked();
                }
                handle
                    .update(cx, |w, window, cx| {
                        let previous_request = w.ui.recovery_refresh;
                        w.show_recovery_hub(window, cx);
                        assert_ne!(w.ui.recovery_refresh, previous_request);
                        assert!(w.ui.modal_focus.is_focused(window));
                    })
                    .unwrap();
                cx.run_until_parked();
                handle
                    .update(cx, |w, _, cx| {
                        assert!(w.ui.history.is_none());
                        assert!(w.ui.trash_open);
                        assert!(w.recoveries.iter().any(|e| e.journal == journal));
                        assert_eq!(w.tabs[0].save.editor.read(cx).value().as_ref(), "unsaved");
                        assert!(w.tabs[0].save.persistence.is_dirty());
                        assert_eq!(w.file_writes.pending(), 0);
                    })
                    .unwrap();
                assert_eq!(vault.read(&path).unwrap().as_deref(), Some("disk"));
                assert!(journal.exists());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn catalog_history_compares_an_inactive_open_notes_unsaved_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-inactive-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let path = PathBuf::from("note.md");
        vault.save(&path, None, "disk text").unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(path.clone(), Some("disk text".into()), false, window, cx);
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| {
                        s.set_value("unsaved text", window, cx);
                    });
                w.flush_document_views(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("other.md".into(), Some("other".into()), false, window, cx);
                w.open_history_path(path.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert!(browser.current_available);
                assert_eq!(browser.current.as_ref(), "unsaved text");
                assert_eq!(w.tabs[w.active.unwrap()].path, PathBuf::from("other.md"));
                assert_eq!(
                    w.tabs[0].save.editor.read(cx).value().as_ref(),
                    "unsaved text"
                );
                assert!(w.tabs[0].save.persistence.is_dirty());
                assert_eq!(w.file_writes.pending(), 0);
            })
            .unwrap();
        assert_eq!(vault.read(&path).unwrap().as_deref(), Some("disk text"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn catalog_history_restores_missing_path_without_saving_other_edits(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-missing-history-ui-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = PathBuf::from("旧名字.md");
        let moved = PathBuf::from("外部重命名.md");
        let other = PathBuf::from("其他未保存.md");
        vault.save(&old, None, "历史正文 👩‍💻").unwrap();
        std::fs::rename(vault.root.join(&old), vault.root.join(&moved)).unwrap();
        vault.save(&other, None, "磁盘正文").unwrap();
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(other.clone(), Some("磁盘正文".into()), false, window, cx);
                let editor = w.current_pane().unwrap().read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_value("保留未保存编辑", window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.open_history_path(old.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert!(!browser.current_available);
                assert!(!browser.loading);
                assert_eq!(browser.preview.read(cx).value().as_ref(), "历史正文 👩‍💻");
                w.restore_history(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let copy = handle
            .update(cx, |w, _, cx| {
                let original = w.tabs.iter().find(|t| t.path == other).unwrap();
                assert!(original.save.persistence.is_dirty());
                assert_eq!(
                    original.pane.read(cx).editor.read(cx).value().as_ref(),
                    "保留未保存编辑"
                );
                assert_eq!(w.file_writes.pending(), 0);
                w.tabs[w.active.unwrap()].path.clone()
            })
            .unwrap();
        assert_ne!(copy, old);
        assert_eq!(vault.read(&copy).unwrap().as_deref(), Some("历史正文 👩‍💻"));
        assert_eq!(vault.read(&other).unwrap().as_deref(), Some("磁盘正文"));
        assert_eq!(vault.read(&moved).unwrap().as_deref(), Some("历史正文 👩‍💻"));
        assert_eq!(vault.read(&old).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn history_pages_reveal_versions_and_load_the_selected_record(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-history-pages-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let path = PathBuf::from("历史.md");
        let mut current = None;
        for i in 0..12 {
            let text = format!("版本 {i} 👩‍💻 e\u{301}");
            vault.save(&path, current.as_deref(), &text).unwrap();
            current = Some(text);
        }
        let handle = cx.add_window(Workspace::new);
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.add_tab(path.clone(), current.clone(), false, window, cx);
                w.open_history(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let expected = handle
            .update(cx, |w, _, _| {
                let browser = w.ui.history.as_ref().unwrap();
                assert_eq!(browser.entries.len(), 12);
                assert!(!browser.loading);
                vault
                    .read_history(&path, &browser.entries[11].journal)
                    .unwrap()
                    .draft
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(800.), px(500.)));
        fn key(v: &mut VisualTestContext, key: &str) {
            let keystroke = Keystroke::parse(key).unwrap();
            v.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            v.simulate_event(KeyUpEvent { keystroke });
            for _ in 0..2 {
                v.update(|w, cx| w.draw(cx).clear(cx));
            }
        }
        let selectors = [
            "history-version-0",
            "history-version-1",
            "history-version-2",
            "history-version-3",
            "history-version-4",
            "history-version-5",
            "history-version-6",
            "history-version-7",
            "history-version-8",
            "history-version-9",
            "history-version-10",
            "history-version-11",
        ];
        for rem in [16., 24.] {
            visual.update(|w, _| w.set_rem_size(px(rem)));
            handle
                .update(&mut visual, |w, window, cx| {
                    w.ui.history.as_mut().unwrap().page = 0;
                    window.focus(&w.ui.modal_focus, cx);
                })
                .unwrap();
            for draft_review in [true, false] {
                handle
                    .update(&mut visual, |w, _, _| {
                        let browser = w.ui.history.as_mut().unwrap();
                        browser.draft_review = draft_review;
                        browser.confirm_discard = draft_review;
                    })
                    .unwrap();
                visual.update(|w, cx| w.draw(cx).clear(cx));
                let preview = visual.debug_bounds("history-preview-pane").unwrap();
                let footer = visual.debug_bounds("history-restore-footer").unwrap();
                assert!(
                    preview.size.height >= px(80.),
                    "rem={rem}: preview too small: {preview:?}"
                );
                assert!(
                    preview.bottom() <= footer.top(),
                    "preview must not overlap restore actions"
                );
                assert!(
                    footer.bottom() <= px(500.),
                    "restore actions must stay visible: {footer:?}"
                );
            }
            for (step, page) in [0, 1, 2, 1, 0].into_iter().enumerate() {
                let navigation = if step < 2 { "tab" } else { "shift-tab" };
                let mut reached = std::collections::BTreeSet::new();
                for _ in 0..30 {
                    key(&mut visual, navigation);
                    if let Some(target) = visual.debug_bounds("focus-revealed-control") {
                        handle
                            .update(&mut visual, |w, _, _| {
                                let viewport = w.ui.history.as_ref().unwrap().scroll.bounds();
                                assert!(
                                    target.top() >= viewport.top()
                                        && target.bottom() <= viewport.bottom(),
                                    "rem={rem}: {target:?} outside {viewport:?}"
                                );
                            })
                            .unwrap();
                        for (i, selector) in selectors.iter().enumerate() {
                            if let Some(row) = visual.debug_bounds(selector)
                                && target.top() >= row.top()
                                && target.bottom() <= row.bottom()
                            {
                                reached.insert(i);
                            }
                        }
                    }
                }
                let start = page * VERSIONS_PER_PAGE;
                let end = (start + VERSIONS_PER_PAGE).min(12);
                assert_eq!(reached, (start..end).collect());
                for (i, selector) in selectors.iter().enumerate() {
                    assert_eq!(
                        visual.debug_bounds(selector).is_some(),
                        (start..end).contains(&i)
                    );
                }
                // Walk forward to the last page, then backward to the first.
                handle
                    .update(&mut visual, |w, _, _| {
                        assert_eq!(w.ui.history.as_mut().unwrap().page, page);
                        assert_eq!(w.ui.history.as_ref().unwrap().selected, Some(0));
                        assert!(w.ui.history.as_ref().unwrap().record.is_some());
                        assert_eq!(w.file_writes.pending(), 0);
                        assert!(w.ui.history.as_ref().unwrap().message.is_empty());
                    })
                    .unwrap();
                if step == 4 {
                    break;
                }
                let forward = step < 2;
                let selector = if forward {
                    "history-page-true"
                } else {
                    "history-page-false"
                };
                let mut found = false;
                for _ in 0..30 {
                    key(&mut visual, navigation);
                    if let (Some(target), Some(button)) = (
                        visual.debug_bounds("focus-revealed-control"),
                        visual.debug_bounds(selector),
                    ) && target.top() >= button.top()
                        && target.bottom() <= button.bottom()
                        && target.left() >= button.left()
                        && target.right() <= button.right()
                    {
                        found = true;
                        break;
                    }
                }
                assert!(found);
                key(&mut visual, "enter");
                handle
                    .update(&mut visual, |w, window, cx| {
                        assert!(
                            w.ui.history
                                .as_ref()
                                .unwrap()
                                .pager_focus
                                .contains_focused(window, cx)
                        );
                    })
                    .unwrap();
            }
        }
        // Navigate to the final page, then activate its last real version.
        handle
            .update(&mut visual, |w, window, cx| {
                let browser = w.ui.history.as_mut().unwrap();
                browser.page = 2;
                window.focus(&browser.pager_focus, cx);
            })
            .unwrap();
        let mut found = false;
        for _ in 0..30 {
            key(&mut visual, "tab");
            if let (Some(target), Some(row)) = (
                visual.debug_bounds("focus-revealed-control"),
                visual.debug_bounds("history-version-11"),
            ) && target.top() >= row.top()
                && target.bottom() <= row.bottom()
            {
                found = true;
                break;
            }
        }
        assert!(found);
        key(&mut visual, "enter");
        visual.run_until_parked();
        handle
            .update(&mut visual, |w, _, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert_eq!(browser.selected, Some(11));
                assert_eq!(browser.page, 2);
                assert!(!browser.loading);
                assert_eq!(browser.record.as_ref().unwrap().draft, expected);
                assert_eq!(browser.preview.read(cx).value().as_ref(), expected);
                assert_eq!(w.file_writes.pending(), 0);
            })
            .unwrap();
        assert_eq!(vault.read(&path).unwrap(), current);
        assert_eq!(
            std::fs::read_dir(&vault.root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn history_preview_diff_restore_and_closed_requests_preserve_original(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-history-ui-{stamp}"));
        std::fs::create_dir(&root).unwrap();
        let vault = Vault::open(&root, app_dir().join("recovery")).unwrap();
        let path = PathBuf::from("原件.md");
        let old = "# 原件\r\n旧版本 👩‍💻 e\u{301}\r\n";
        let current = "# 原件\r\n当前版本\r\n";
        let first = vault.save(&path, None, old).unwrap();
        let second = vault.save(&path, Some(old), current).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note(path.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.execute_command(99, window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert_eq!(browser.entries.len(), 2);
                assert!(!browser.loading);
                assert!(!browser.preview.read(cx).is_editable());
                let i = browser
                    .entries
                    .iter()
                    .position(|e| e.journal == second.recovery)
                    .unwrap();
                w.select_history(i, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_mut().unwrap();
                assert_eq!(browser.preview.read(cx).value().as_ref(), current);
                browser.baseline = true;
                browser.difference = true;
                w.update_history_preview(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                let diff = browser.preview.read(cx).value();
                assert!(
                    diff.contains("-旧版本") && diff.contains("+当前版本"),
                    "{diff}"
                );
                w.restore_history(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let restored = handle
            .update(cx, |w, window, cx| {
                assert!(w.ui.history.is_none());
                let tab = &w.tabs[w.active.unwrap()];
                let restored = tab.path.clone();
                assert_ne!(restored, path);
                assert_eq!(tab.pane.read(cx).editor.read(cx).value().as_ref(), old);
                w.execute_command(99, window, cx);
                w.close_overlays(window, cx);
                restored
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.ui.history.is_none());
                w.watcher = None;
            })
            .unwrap();
        assert_eq!(vault.read(&restored).unwrap().as_deref(), Some(old));
        assert_eq!(vault.read(&path).unwrap().as_deref(), Some(current));
        assert!(first.recovery.exists() && second.recovery.exists());
        for note in [&path, &restored] {
            for entry in vault.history(note).unwrap() {
                std::fs::remove_file(entry.journal).unwrap();
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn renamed_history_restores_a_copy_at_current_path(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-renamed-history-ui-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        let old = PathBuf::from("旧名.md");
        let current = PathBuf::from("新名.md");
        let first = vault.save(&old, None, "旧版本😀").unwrap();
        vault.save(&old, Some("旧版本😀"), "当前内容").unwrap();
        vault.rename_note(&old, &current, "当前内容").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(vault.root.clone(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.open_note(current.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.execute_command(99, window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                let selected = browser
                    .entries
                    .iter()
                    .position(|e| e.journal == first.recovery)
                    .unwrap();
                w.select_history(selected, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert_eq!(browser.path, current);
                assert_eq!(browser.record.as_ref().unwrap().relative, old);
                w.restore_history(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let restored = handle
            .update(cx, |w, _, _| {
                w.watcher = None;
                w.tabs[w.active.unwrap()].path.clone()
            })
            .unwrap();
        assert_ne!(restored, current);
        assert_ne!(restored, old);
        assert_eq!(vault.read(&restored).unwrap().as_deref(), Some("旧版本😀"));
        assert_eq!(vault.read(&current).unwrap().as_deref(), Some("当前内容"));
        assert!(vault.read(&old).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[gpui::test]
    fn changed_draft_summary_cannot_enable_recovery(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-stale-draft-summary-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("note.md"), "original").unwrap();
        let journal = vault
            .journal(
                std::path::Path::new("note.md"),
                Some("original"),
                "listed draft",
            )
            .unwrap();
        let summaries = vault.recovery_summaries().unwrap();
        std::fs::write(&journal, "changed after listing").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(vault.clone());
                w.recoveries = summaries;
                w.review_draft(0, window, cx);
                assert!(w.reviewing_draft());
                assert!(w.ui.history.as_ref().unwrap().draft_entry.is_none());
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                assert!(!browser.loading);
                assert!(browser.record.is_none() && browser.draft_entry.is_none());
                assert!(browser.message.contains("无法读取草稿"));
                w.finish_draft_review(false, window, cx);
                w.finish_draft_review(true, window, cx);
                assert!(w.tabs.is_empty());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(&journal).unwrap(),
            "changed after listing"
        );
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "original"
        );
        assert_eq!(std::fs::read_dir(&vault.root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn draft_review_compares_disk_and_discard_requires_confirmation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inkstone-review-draft-{stamp}"));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        let vault = Vault::open(root.join("vault"), root.join("recovery")).unwrap();
        std::fs::write(vault.root.join("note.md"), "disk text").unwrap();
        let older = vault
            .journal(std::path::Path::new("note.md"), Some("base"), "older")
            .unwrap();
        let latest = vault
            .journal(std::path::Path::new("note.md"), Some("base"), "draft text")
            .unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.recoveries = vault.recovery_summaries().unwrap();
                w.vault = Some(vault.clone());
                let index = w
                    .recoveries
                    .iter()
                    .position(|e| e.journal == latest)
                    .unwrap();
                w.review_draft(index, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let browser = w.ui.history.as_ref().unwrap();
                let preview = browser.preview.read(cx).value();
                assert!(preview.contains("disk text") && preview.contains("draft text"));
                w.finish_draft_review(true, window, cx);
                assert!(latest.exists());
                assert!(w.ui.history.as_ref().unwrap().confirm_discard);
                w.finish_draft_review(true, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!latest.exists());
        assert!(older.exists());
        assert_eq!(
            std::fs::read_to_string(vault.root.join("note.md")).unwrap(),
            "disk text"
        );
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.recoveries.len(), 1);
                w.review_draft(0, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        // A modified record after review must not be recovered or discarded.
        std::fs::write(&older, "corrupt").unwrap();
        handle
            .update(cx, |w, window, cx| w.finish_draft_review(false, window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, _| {
                assert!(w.notifications.text().contains("草稿处理失败"));
                assert!(w.tabs.is_empty());
            })
            .unwrap();
        assert!(older.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
