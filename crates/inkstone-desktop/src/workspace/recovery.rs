use super::focus_reveal::FocusReveal;
use super::*;
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
    entries: Vec<HistoryEntry>,
    selected: Option<usize>,
    record: Option<Recovery>,
    baseline: bool,
    difference: bool,
    loading: bool,
    message: String,
    preview: Entity<TextareaState>,
    draft_entry: Option<RecoveryEntry>,
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
    let time = metadata
        .and_then(|m| m.modified)
        .map(|t| {
            let time: chrono::DateTime<chrono::Local> = t.into();
            time.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_else(|| "时间未知".into());
    let size = metadata
        .and_then(|m| m.bytes)
        .map(|n| format!("{:.1} KiB", n as f64 / 1024.))
        .unwrap_or_else(|| "大小未知".into());
    format!("回收站 · {time} · {size}")
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
        self.ui
            .history
            .as_ref()
            .is_some_and(|b| b.draft_entry.is_some())
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
            path: entry.record.relative.clone(),
            current: "".into(),
            entries: vec![],
            selected: None,
            record: None,
            baseline: false,
            difference: true,
            loading: true,
            message: String::new(),
            preview,
            draft_entry: Some(entry.clone()),
            confirm_discard: false,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            vault.validate_draft(&entry)?;
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
                        browser.entries = vec![HistoryEntry {
                            journal: entry.journal,
                            modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                            bytes: metadata.len(),
                            saved: false,
                        }];
                        browser.selected = Some(0);
                        browser.record = Some(entry.record);
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
        self.ui.pending_file_writes += 1;
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
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
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
        let Some(vault) = self.vault.clone() else {
            return;
        };
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
        let current = pane.read(cx).editor.read(cx).value();
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
            current,
            entries: vec![],
            selected: None,
            record: None,
            baseline: false,
            difference: false,
            loading: true,
            message: String::new(),
            preview,
            draft_entry: None,
            confirm_discard: false,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx
            .background_executor()
            .spawn(async move { vault.history(&path) });
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
                        browser.record = Some(record);
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
        let Some(record) = &browser.record else {
            return;
        };
        let text = if browser.baseline {
            record.baseline.as_ref().unwrap_or(&record.draft)
        } else {
            &record.draft
        }
        .clone();
        let current = browser.current.clone();
        let difference = browser.difference;
        let draft_review = browser.draft_entry.is_some();
        browser.loading = true;
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
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
                preview_text(&text)
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
        if self
            .ui
            .history
            .as_ref()
            .is_some_and(|b| b.draft_entry.is_some())
        {
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

    pub(super) fn history_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(browser) = &self.ui.history else {
            return div().into_any_element();
        };
        let bytes: u64 = browser.entries.iter().map(|e| e.bytes).sum();
        div()
            .id("history-content")
            .track_scroll(&browser.scroll)
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .min_h_0()
            .child(
                div()
                    .truncate()
                    .child(browser.path.to_string_lossy().to_string()),
            )
            .when_some(browser.record.as_ref(), |s, record| {
                s.child(
                    div()
                        .text_sm()
                        .child(format!("记录原路径：{}", record.relative.display())),
                )
            })
            .child(div().text_sm().child(if browser.draft_entry.is_some() {
                format!(
                    "未保存恢复记录 · {:.1} KiB · 恢复或放弃前保留",
                    bytes as f64 / 1024.
                )
            } else {
                format!(
                    "{} 条记录 · {:.1} MiB · {}",
                    browser.entries.len(),
                    bytes as f64 / 1048576.,
                    retention_label(self.ui.prefs.history)
                )
            }))
            .when_some(browser.selected.and_then(|i| browser.entries.get(i)), |s, entry| {
                s.child(div().text_sm().whitespace_normal().child(format!(
                    "所选记录 {} · {} · 记录文件 {} 字节（{:.1} KiB）",
                    browser.selected.unwrap_or_default() + 1,
                    if entry.saved { "保存历史" } else { "未保存恢复记录" },
                    entry.bytes,
                    entry.bytes as f64 / 1024.
                )))
            })
            .child(div().text_sm().whitespace_normal().child(
                "容量按记录文件长度统计，包含正文、元数据及可能的保存前正文，不等于当前笔记大小或磁盘实际分配空间。"
            ))
            .when(browser.loading, |s| s.child("正在读取版本……"))
            .when(!browser.message.is_empty(), |s| {
                s.child(browser.message.clone())
            })
            .when(browser.entries.is_empty() && !browser.loading, |s| {
                s.child("这篇笔记尚无可用历史。修改并保存后会记录版本。")
            })
            .when(!browser.entries.is_empty(), |panel| {
                let pages = browser.entries.len().div_ceil(VERSIONS_PER_PAGE);
                panel.child(div().track_focus(&browser.pager_focus).flex().flex_wrap().gap_2()
                    .child(format!("历史：第 {} / {pages} 页，每页最多 {VERSIONS_PER_PAGE} 条", browser.page + 1))
                    .children([false, true].into_iter().map(|next| {
                        let id = ("history-page", usize::from(next));
                        div().debug_selector(move || format!("history-page-{next}"))
                            .child(FocusReveal::new((ElementId::from(id), "focus"), &browser.scroll,
                                Button::new(id).label(if next { "历史下一页" } else { "历史上一页" })
                                    .disabled(if next { browser.page + 1 >= pages } else { browser.page == 0 })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let Some(browser) = &mut this.ui.history else { return; };
                                        let last = browser.entries.len().saturating_sub(1) / VERSIONS_PER_PAGE;
                                        browser.page = if next { browser.page.saturating_add(1).min(last) } else { browser.page.saturating_sub(1) };
                                        // Paging does not select/read another version or cancel its preview.
                                        window.focus(&browser.pager_focus, cx);
                                        cx.notify();
                                    }))))
                    })))
                    .children(browser.entries.iter().enumerate()
                        .skip(browser.page * VERSIONS_PER_PAGE).take(VERSIONS_PER_PAGE)
                        .map(|(i, entry)| {
                            let date: chrono::DateTime<chrono::Local> = entry.modified.into();
                            div().debug_selector(move || format!("history-version-{i}"))
                                .child(FocusReveal::new((ElementId::from(("history-version", i)), "focus"), &browser.scroll,
                                    Button::new(("history-version", i)).ghost()
                                        .when(browser.selected == Some(i), |b| b.primary())
                                        .selected(browser.selected == Some(i))
                                        .label(format!("{} · {} · {}", i + 1, date.format("%Y-%m-%d %H:%M:%S%.3f"), if entry.saved { "已保存" } else { "恢复草稿" }))
                                        .on_click(cx.listener(move |this, _, window, cx| this.select_history(i, window, cx)))))
                        }))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(FocusReveal::new("history-before-focus", &browser.scroll, Button::new("history-before")
                            .label("保存前内容")
                            .when(browser.baseline, |b| b.primary())
                            .selected(browser.baseline)
                            .disabled(browser.record.as_ref().is_none_or(|r| r.baseline.is_none()))
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(browser) = &mut this.ui.history {
                                    browser.baseline = true;
                                }
                                this.update_history_preview(w, cx);
                            }))),)
                    .child(FocusReveal::new("history-draft-focus", &browser.scroll, Button::new("history-draft")
                            .label("记录内容")
                            .when(!browser.baseline, |b| b.primary())
                            .selected(!browser.baseline)
                            .disabled(browser.record.is_none())
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(browser) = &mut this.ui.history {
                                    browser.baseline = false;
                                }
                                this.update_history_preview(w, cx);
                            }))),)
                    .child(FocusReveal::new("history-diff-focus", &browser.scroll, Button::new("history-diff")
                            .label(if browser.draft_entry.is_some() {
                                "与磁盘正文比较"
                            } else {
                                "与当前正文比较"
                            })
                            .when(browser.difference, |b| b.primary())
                            .selected(browser.difference)
                            .disabled(browser.record.is_none())
                            .on_click(cx.listener(|this, _, w, cx| {
                                if let Some(browser) = &mut this.ui.history {
                                    browser.difference = !browser.difference;
                                }
                                this.update_history_preview(w, cx);
                            }))),),
            )
            .child(Textarea::new(&browser.preview).readonly(true).h(px(240.)))
            .child(div().text_sm().child(if browser.draft_entry.is_some() {
                "差异以打开比较时的磁盘正文为准；恢复草稿会新建副本，成功后清理所选草稿。"
            } else {
                "差异以打开历史时的正文为准；恢复会新建笔记并保留原文件。"
            }))
            .child(FocusReveal::new("history-restore-focus", &browser.scroll, Button::new("history-restore")
                    .primary()
                    .label(if browser.draft_entry.is_some() {
                        "恢复草稿为副本"
                    } else {
                        "恢复所选内容为新笔记"
                    })
                    .disabled(browser.record.is_none() || browser.loading)
                    .on_click(cx.listener(|this, _, w, cx| this.restore_history(w, cx)))),)
            .when(browser.draft_entry.is_some(), |s| {
                s.child(FocusReveal::new("draft-discard-focus", &browser.scroll, Button::new("draft-discard")
                        .label(if browser.confirm_discard {
                            "确认放弃这一条草稿"
                        } else {
                            "放弃恢复"
                        })
                        .disabled(browser.loading || browser.record.is_none())
                        .on_click(
                            cx.listener(|this, _, w, cx| this.finish_draft_review(true, w, cx)),
                        )),)
            })
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
                        assert_eq!(w.ui.pending_file_writes, 0);
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
                assert_eq!(w.ui.pending_file_writes, 0);
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
                w.recoveries = vault.recoveries().unwrap();
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
