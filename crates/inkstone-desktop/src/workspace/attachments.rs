use super::accessible_text::label;
use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use inkstone_core::vault::attachments::{self, Attachment, Inventory, Kind};

mod cleanup;

pub(super) struct Manager {
    inventory: Inventory,
    query: Entity<InputState>,
    _subscription: Subscription,
    kind: Option<Kind>,
    large: bool,
    unused: bool,
    selected: Option<PathBuf>,
    loading: bool,
    message: String,
}
impl Manager {
    fn unused_images(&self, cx: &App) -> Vec<Attachment> {
        self.filtered(cx)
            .into_iter()
            .map(|i| &self.inventory.entries[i])
            .filter(|e| e.kind == Kind::Image && e.references.is_empty())
            .cloned()
            .collect()
    }

    fn filtered(&self, cx: &App) -> Vec<usize> {
        let query = self.query.read(cx).value().to_lowercase();
        self.inventory
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                self.kind.is_none_or(|k| e.kind == k)
                    && (!self.large || e.bytes >= 1024 * 1024)
                    && (!self.unused || e.references.is_empty())
                    && (query.is_empty()
                        || e.path.to_string_lossy().to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect()
    }
}
impl Workspace {
    pub(super) fn open_attachment_manager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault.is_none() {
            return;
        }
        self.close_overlays(window, cx);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索附件路径…"));
        let subscription = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.ui.attachment_manager = Some(Manager {
            inventory: Default::default(),
            query,
            _subscription: subscription,
            kind: None,
            large: false,
            unused: false,
            selected: None,
            loading: false,
            message: String::new(),
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        self.refresh_attachments(window, cx);
    }
    fn refresh_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Cleanup may finish after the panel closes; avoid collecting unused drafts.
        if self.ui.attachment_manager.is_none() {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        let drafts = self.file_draft_snapshots(cx);
        let Some(manager) = &mut self.ui.attachment_manager else {
            return;
        };
        manager.loading = true;
        manager.message.clear();
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { attachments::scan(&vault, &document::owned_drafts(drafts)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                if let Some(m) = &mut this.ui.attachment_manager {
                    m.loading = false;
                    match result {
                        Ok(inventory) => {
                            m.inventory = inventory;
                            if m.inventory.unreadable > 0 {
                                m.message = format!(
                                    "{} 篇笔记无法读取，已禁用清理。",
                                    m.inventory.unreadable
                                );
                            }
                        }
                        Err(error) => {
                            m.message = error.to_string();
                            m.inventory.unreadable = 1;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn request_attachment_cleanup(
        &mut self,
        entry: Attachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.file_writes.can_start_exclusive_operation() {
            return;
        }
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let detail = format!(
            "{}\n大小：{:.1} KiB\n\n扫描未找到直接引用，但其他文件、脚本或外部应用仍可能使用它。移入回收站后可通过“文件恢复”恢复。",
            entry.path.display(),
            entry.bytes as f64 / 1024.
        );
        let prompt = window.prompt(
            PromptLevel::Warning,
            "将疑似未引用附件移入回收站？",
            Some(&detail),
            &["取消", "移入回收站"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if prompt.await != Ok(1) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation == generation
                    && this.ui.recovery_refresh == request
                    && this.ui.attachment_manager.is_some()
                {
                    this.cleanup_attachment(entry, window, cx);
                }
            });
        })
        .detach();
    }
    fn attachment_cleanup_drafts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<std::collections::BTreeMap<PathBuf, SharedString>> {
        if !self.file_writes.can_start_exclusive_operation() {
            self.notifications
                .publish("请等待当前文件操作完成后再清理附件。".into());
            cx.notify();
            return None;
        }
        if self
            .tabs
            .iter()
            .any(|tab| self.has_pending_input(tab.id, window, cx))
        {
            self.notifications
                .publish("请完成输入法组词或脚注编辑后再清理附件。".into());
            cx.notify();
            return None;
        }
        self.flush_document_views(window, cx);
        // Unnamed notes have no stable base directory for relative references.
        if self.tabs.iter().any(|tab| {
            tab.image.is_none()
                && tab.path.as_os_str().is_empty()
                && !tab.save.text_snapshot(cx).trim().is_empty()
        }) {
            self.notifications
                .publish("请先保存未命名笔记，再清理附件，以便检查其中的引用。".into());
            cx.notify();
            return None;
        }
        Some(self.file_draft_snapshots(cx))
    }
    fn cleanup_attachment(
        &mut self,
        entry: Attachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(drafts) = self.attachment_cleanup_drafts(window, cx) else {
            return;
        };
        let Some(write_ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        if let Some(m) = &mut self.ui.attachment_manager {
            m.loading = true;
        }
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            attachments::trash_unused(&vault, &entry, &document::owned_drafts(drafts))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(write_ticket);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(_) => {
                        this.notifications
                            .publish("附件已移入回收站，可从“文件恢复”恢复。".into());
                        this.schedule_auto_sync(true);
                        this.refresh_attachments(window, cx);
                    }
                    Err(error) => {
                        this.notifications.publish(format!("附件未清理：{error}"));
                        if this.ui.recovery_refresh == request
                            && let Some(m) = &mut this.ui.attachment_manager
                        {
                            m.loading = false;
                            m.message = this.notifications.text().to_owned();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn attachments_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(m) = &self.ui.attachment_manager else {
            return div().into_any_element();
        };
        let filtered = m.filtered(cx);
        let colors = crate::theme::palette(self.ui.prefs.light);
        let total = m.inventory.entries.len();
        let count = filtered.len();
        let (images, bytes) = filtered
            .iter()
            .map(|&i| &m.inventory.entries[i])
            .filter(|entry| entry.kind == Kind::Image && entry.references.is_empty())
            .fold((0usize, 0u64), |(n, bytes), e| (n + 1, bytes + e.bytes));
        let selected = filtered
            .iter()
            .map(|&i| &m.inventory.entries[i])
            .find(|e| Some(&e.path) == m.selected.as_ref());
        let kinds = [
            (None, "全部"),
            (Some(Kind::Image), "图片"),
            (Some(Kind::Pdf), "PDF"),
            (Some(Kind::Media), "音视频"),
            (Some(Kind::Other), "其他"),
        ];
        let types =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(kinds.into_iter().map(|(kind, name)| {
                    Button::new(format!("attachment-kind-{name}"))
                        .label(name)
                        .compact()
                        .ghost()
                        .when(m.kind == kind, |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(manager) = &mut this.ui.attachment_manager {
                                manager.kind = kind;
                            }
                            cx.notify();
                        }))
                }));
        let filters = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(
                Button::new("attachment-large")
                    .label("至少 1 MiB")
                    .compact()
                    .when(m.large, |b| b.primary())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(m) = &mut this.ui.attachment_manager {
                            m.large = !m.large;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("attachment-unused")
                    .label("疑似未引用")
                    .compact()
                    .when(m.unused, |b| b.primary())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(m) = &mut this.ui.attachment_manager {
                            m.unused = !m.unused;
                        }
                        cx.notify();
                    })),
            );
        let empty_title = if total == 0 {
            "笔记库中还没有附件"
        } else if m.kind == Some(Kind::Image) && m.unused {
            "当前筛选下没有待清理图片"
        } else {
            "没有匹配的附件"
        };
        div().id("attachments-panel").flex().flex_col().gap_3().min_h_0().overflow_y_scroll()
            .child(div().flex().gap_2().items_center()
                .child(div().flex_1().min_w_0().child(Input::new(&m.query)))
                .child(Button::new("attachment-refresh").label("刷新").compact().ghost().disabled(m.loading)
                    .on_click(cx.listener(|this, _, w, cx| this.refresh_attachments(w, cx)))))
            .child(div().flex().flex_wrap().justify_between().gap_2().child(types).child(filters))
            .when(m.loading, |s| s.child(label("attachment-loading", "正在扫描或处理附件……").text_color(colors.muted)))
            .when(!m.message.is_empty(), |s| s.child(label("attachment-message", m.message.clone())))
            .when(!m.loading && m.inventory.unreadable == 0 && count == 0, |s| {
                s.child(div().debug_selector(|| "attachment-empty".into()).flex().flex_col().items_center()
                    .py_5().px_3().gap_1().rounded_md().bg(colors.surface)
                    .child(label("attachment-empty-title", empty_title).font_weight(FontWeight::SEMIBOLD))
                    .when(total > 0, |s| s.child(label("attachment-empty-detail",
                        format!("库中共 {total} 个附件，可调整筛选条件查看。")).text_sm().text_color(colors.muted))))
            })
            .when(count > 0, |s| {
                let summary = format!("{count} / {total} 个附件 · 按大小排序");
                s.child(div().flex().items_center().flex_wrap().justify_between().gap_2()
                    .child(label("attachment-summary", summary).text_color(colors.muted).text_sm())
                    .when(images > 0, |s| s.child(Button::new("image-cleanup-review").compact()
                        .label(format!("清理 {images} 张图片…")).tooltip(format!("合计 {:.1} KiB，点击核对清单", bytes as f64 / 1024.))
                        .disabled(m.loading || m.inventory.unreadable > 0 || !self.file_writes.can_start_exclusive_operation())
                        .on_click(cx.listener(|this, _, w, cx| this.request_unused_image_cleanup(w, cx))))))
                .child(uniform_list("attachment-list", count, cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let Some(m) = &this.ui.attachment_manager else { return vec![]; };
                    range.filter_map(|i| {
                        let entry = m.inventory.entries.get(*filtered.get(i)?)?;
                        let path = entry.path.clone();
                        let metadata = format!("{} · {:.1} KiB · {} 篇引用", entry.kind.label(), entry.bytes as f64 / 1024., entry.references.len());
                        Some(Button::new(("attachment", i)).ghost().w_full().h(px(36.))
                            .when(m.selected.as_ref() == Some(&path), |b| b.primary())
                            .accessibility_label(format!("{}，{metadata}", path.display()))
                            .tooltip(path.display().to_string())
                            .child(div().w_full().min_w_0().flex().items_center().justify_between().gap_2()
                                .child(div().flex_1().min_w_0().truncate().child(path.display().to_string()))
                                .child(div().flex_shrink_0().text_sm().child(metadata)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(m) = &mut this.ui.attachment_manager { m.selected = Some(path.clone()); }
                                cx.notify();
                            })).into_any_element())
                    }).collect()
                })).h(px(count.min(6) as f32 * 36.)).flex_shrink_0().w_full())
            })
            .when_some(selected,|s,e| {
                let path=e.path.clone();let reveal=path.clone();let open=path.clone();let cleanup=e.clone();
                s.child(label("attachment-selected-path", path.to_string_lossy().to_string())).child(div().flex().flex_wrap().gap_2()
                    .child(Button::new("attachment-preview").label("打开 / 预览").on_click(cx.listener(move|this,_,w,cx|{if let Some(v)=&this.vault{let path=v.root.join(&open);this.open_attachment(path,w,cx);}})))
                    .child(Button::new("attachment-reveal").label("显示文件位置").on_click(cx.listener(move|this,_,_,cx|{if let Some(v)=&this.vault{cx.reveal_path(&v.root.join(&reveal));}})))
                    .child(Button::new("attachment-trash").label("移入回收站").disabled(m.loading||m.inventory.unreadable>0||!e.references.is_empty()||self.file_writes.pending()>0).on_click(cx.listener(move|this,_,w,cx|this.request_attachment_cleanup(cleanup.clone(),w,cx)))))
                    .when(!e.references.is_empty(), |s| s.child(label("attachment-reference-heading", "引用来源")))
                    .child(div().id("attachment-references").max_h(px(140.)).overflow_y_scroll().children(e.references.iter().cloned().enumerate().map(|(i,path)|Button::new(("attachment-source",i)).ghost().label(path.to_string_lossy().to_string()).on_click(cx.listener(move|this,_,w,cx|{this.close_overlays(w,cx);this.open_note(path.clone(),w,cx);})))) )
            })
            .child(div().debug_selector(|| "attachment-footer".into()).border_t_1().border_color(colors.border).pt_2().child(
                label("attachment-cleanup-description", "检查笔记及未保存编辑中的双链、Markdown 和 HTML 引用。脚本和库外引用不在检查范围内。已清理的文件可从回收站恢复。")
                    .text_sm().text_color(colors.muted)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn empty_image_filter_keeps_footer_next_to_result(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("book.pdf"), "book").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.path().into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_unused_image_cleanup(window, cx))
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for width in [1000., 400.] {
            visual.simulate_resize(size(px(width), px(760.)));
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let result = visual.debug_bounds("attachment-empty").unwrap();
            let footer = visual.debug_bounds("attachment-footer").unwrap();
            let gap = footer.top() - result.bottom();
            assert!((px(0.)..=px(24.)).contains(&gap), "gap: {gap:?}");
            assert!(footer.right() <= px(width) && footer.bottom() <= px(760.));
        }
    }

    #[gpui::test]
    fn manager_filters_preserves_referenced_drafts_and_restores_cleanup(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-manager-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("vault")).unwrap();
        std::fs::write(root.join("vault/a.md"), "note").unwrap();
        std::fs::write(root.join("vault/image.png"), [0, 1, 255]).unwrap();
        std::fs::write(root.join("vault/book.pdf"), vec![0; 1024 * 1024]).unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.load_vault(root.join("vault"), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note("a.md".into(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("a.md".into(), None, false, window, cx);
                assert!(std::rc::Rc::ptr_eq(&w.tabs[0].save, &w.tabs[1].save));
                w.new_blank(window, cx);
                assert!(w.tabs.iter().any(|tab| tab.path.as_os_str().is_empty()));
                w.open_attachment_manager(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let image = handle
            .update(cx, |w, window, cx| {
                let m = w.ui.attachment_manager.as_mut().unwrap();
                assert_eq!(m.filtered(cx).len(), 2);
                m.query
                    .update(cx, |query, cx| query.set_value("IMAGE", window, cx));
                assert_eq!(m.filtered(cx).len(), 1);
                m.query
                    .update(cx, |query, cx| query.set_value("missing", window, cx));
                assert!(m.filtered(cx).is_empty());
                m.query
                    .update(cx, |query, cx| query.set_value("", window, cx));
                assert_eq!(m.filtered(cx).len(), 2);
                m.large = true;
                assert_eq!(m.filtered(cx).len(), 1);
                m.large = false;
                m.kind = Some(Kind::Image);
                assert_eq!(m.filtered(cx).len(), 1);
                let image = m
                    .inventory
                    .entries
                    .iter()
                    .find(|e| e.kind == Kind::Image)
                    .unwrap()
                    .clone();
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("![[image.png]]", window, cx));
                w.cleanup_attachment(image.clone(), window, cx);
                // A newer inspection owns the panel before cleanup finishes.
                w.ui.recovery_refresh = w.ui.recovery_refresh.wrapping_add(1);
                let manager = w.ui.attachment_manager.as_mut().unwrap();
                manager.message = "new inspection".into();
                manager.loading = true;
                image
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("vault/image.png").exists());
        handle
            .update(cx, |w, window, cx| {
                assert!(w.notifications.text().contains("已被笔记引用"));
                let manager = w.ui.attachment_manager.as_ref().unwrap();
                assert_eq!(manager.message, "new inspection");
                assert!(manager.loading);
                w.cleanup_attachment(image.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let manager = w.ui.attachment_manager.as_ref().unwrap();
                assert!(!manager.loading);
                assert!(manager.message.contains("已被笔记引用"));
                w.tabs[0]
                    .save
                    .editor
                    .update(cx, |s, cx| s.replace_all("note", window, cx));
                w.cleanup_attachment(image, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!root.join("vault/image.png").exists());
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.file_writes.pending(), 0);
                let v = w.vault.as_ref().unwrap();
                v.restore_trash(&v.trash_entries().unwrap()[0]).unwrap();
                w.refresh_attachments(window, cx);
                w.close_overlays(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read(root.join("vault/image.png")).unwrap(),
            [0, 1, 255]
        );
        handle
            .update(cx, |w, _, _| {
                assert!(w.ui.attachment_manager.is_none());
                w.watcher = None;
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
