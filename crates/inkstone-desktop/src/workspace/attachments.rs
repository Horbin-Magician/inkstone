use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use inkstone_core::vault::attachments::{self, Attachment, Inventory, Kind};
use std::collections::BTreeMap;

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
                    && e.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }
}
impl Workspace {
    fn attachment_drafts(&self, cx: &App) -> BTreeMap<PathBuf, String> {
        self.tabs
            .iter()
            .map(|t| (t.path.clone(), t.save.editor.read(cx).value().to_string()))
            .collect()
    }
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
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        let drafts = self.attachment_drafts(cx);
        let Some(manager) = &mut self.ui.attachment_manager else {
            return;
        };
        manager.loading = true;
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { attachments::scan(&vault, &drafts) });
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
                            m.message = if m.inventory.unreadable > 0 {
                                format!("{} 篇笔记无法读取，已禁用清理。", m.inventory.unreadable)
                            } else {
                                String::new()
                            };
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
        if self.ui.file_operation || self.ui.pending_file_writes > 0 {
            return;
        }
        let generation = self.generation;
        let detail = format!(
            "{}\n大小：{:.1} KiB\n\n扫描未找到直接引用，但其他文件、脚本或外部应用仍可能使用它。移入回收站后可通过“查看回收站”恢复。",
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
                if this.generation == generation {
                    this.cleanup_attachment(entry, window, cx);
                }
            });
        })
        .detach();
    }
    fn cleanup_attachment(
        &mut self,
        entry: Attachment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ui.file_operation || self.ui.pending_file_writes > 0 {
            return;
        }
        if self
            .tabs
            .iter()
            .any(|t| self.has_pending_input(t.id, window, cx))
        {
            self.status = "请完成输入法组词或脚注编辑后再清理附件。".into();
            cx.notify();
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        let drafts = self.attachment_drafts(cx);
        self.ui.pending_file_writes += 1;
        if let Some(m) = &mut self.ui.attachment_manager {
            m.loading = true;
        }
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { attachments::trash_unused(&vault, &entry, &drafts) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.ui.pending_file_writes = this.ui.pending_file_writes.saturating_sub(1);
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(_) => {
                        this.status = "附件已移入回收站，可从“查看回收站”恢复。".into();
                        this.schedule_auto_sync(true);
                        this.refresh_attachments(window, cx);
                    }
                    Err(error) => {
                        this.status = format!("附件未清理：{error}");
                        if let Some(m) = &mut this.ui.attachment_manager {
                            m.loading = false;
                            m.message = this.status.clone();
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
        let selected = m
            .selected
            .as_ref()
            .and_then(|p| m.inventory.entries.iter().find(|e| &e.path == p))
            .cloned();
        div().id("attachments-panel").flex().flex_col().gap_2().min_h_0().overflow_y_scroll()
            .child(Input::new(&m.query))
            .child(div().flex().gap_1().children([(None,"全部"),(Some(Kind::Image),"图片"),(Some(Kind::Pdf),"PDF"),(Some(Kind::Media),"音视频"),(Some(Kind::Other),"其他")].into_iter().map(|(kind,label)|Button::new(format!("attachment-kind-{label}")).label(label).when(m.kind==kind,|b|b.primary()).on_click(cx.listener(move|this,_,_,cx|{if let Some(m)=&mut this.ui.attachment_manager{m.kind=kind;}cx.notify();})))))
            .child(div().flex().gap_2()
                .child(Button::new("attachment-large").label("至少 1 MiB").when(m.large,|b|b.primary()).on_click(cx.listener(|this,_,_,cx|{if let Some(m)=&mut this.ui.attachment_manager{m.large = !m.large;}cx.notify();})))
                .child(Button::new("attachment-unused").label("疑似未引用").when(m.unused,|b|b.primary()).on_click(cx.listener(|this,_,_,cx|{if let Some(m)=&mut this.ui.attachment_manager{m.unused = !m.unused;}cx.notify();})))
                .child(Button::new("attachment-refresh").label("刷新").disabled(m.loading).on_click(cx.listener(|this,_,w,cx|this.refresh_attachments(w,cx)))))
            .child(format!("显示 {} / {} 个附件 · 按大小排序 · 引用包含打开笔记的编辑快照",filtered.len(),m.inventory.entries.len()))
            .when(m.loading,|s|s.child("正在扫描或处理……"))
            .when(!m.message.is_empty(),|s|s.child(m.message.clone()))
            .child(uniform_list("attachment-list",filtered.len(),cx.processor(|this,range:std::ops::Range<usize>,_,cx| {
                let Some(m)=&this.ui.attachment_manager else{return vec![];};let filtered=m.filtered(cx);
                range.filter_map(|i|{let entry=m.inventory.entries.get(*filtered.get(i)?)?;let path=entry.path.clone();Some(Button::new(("attachment",i)).ghost().when(m.selected.as_ref()==Some(&path),|b|b.primary()).label(format!("{} · {:.1} KiB · {} 篇引用 · {}",entry.kind.label(),entry.bytes as f64/1024.,entry.references.len(),path.display())).on_click(cx.listener(move|this,_,_,cx|{if let Some(m)=&mut this.ui.attachment_manager{m.selected=Some(path.clone());}cx.notify();})).into_any_element())}).collect()
            })).h(px(200.)).w_full())
            .when_some(selected,|s,e| {
                let path=e.path.clone();let reveal=path.clone();let open=path.clone();let cleanup=e.clone();
                s.child(path.to_string_lossy().to_string()).child(div().flex().gap_2()
                    .child(Button::new("attachment-preview").label("系统打开 / 预览").on_click(cx.listener(move|this,_,_,cx|{if let Some(v)=&this.vault{cx.open_with_system(&v.root.join(&open));}})))
                    .child(Button::new("attachment-reveal").label("显示文件位置").on_click(cx.listener(move|this,_,_,cx|{if let Some(v)=&this.vault{cx.reveal_path(&v.root.join(&reveal));}})))
                    .child(Button::new("attachment-trash").label("移入回收站").disabled(m.loading||m.inventory.unreadable>0||!e.references.is_empty()||self.ui.pending_file_writes>0).on_click(cx.listener(move|this,_,w,cx|this.request_attachment_cleanup(cleanup.clone(),w,cx)))))
                    .child("引用来源（点击打开）：")
                    .child(div().id("attachment-references").max_h(px(140.)).overflow_y_scroll().children(e.references.into_iter().enumerate().map(|(i,path)|Button::new(("attachment-source",i)).ghost().label(path.to_string_lossy().to_string()).on_click(cx.listener(move|this,_,w,cx|{this.close_overlays(w,cx);this.open_note(path.clone(),w,cx);})))) )
            })
            .child(div().text_sm().child("疑似未引用不等于无用。仅检查笔记中的双链、Markdown 与 HTML 直接引用；不分析脚本、其他附件内容或库外引用。清理前会重新扫描，文件仍可从回收站恢复。"))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
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
            .update(cx, |w, window, cx| w.open_attachment_manager(window, cx))
            .unwrap();
        cx.run_until_parked();
        let image = handle
            .update(cx, |w, window, cx| {
                let m = w.ui.attachment_manager.as_mut().unwrap();
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
                image
            })
            .unwrap();
        cx.run_until_parked();
        assert!(root.join("vault/image.png").exists());
        handle
            .update(cx, |w, window, cx| {
                assert!(w.status.contains("已被笔记引用"));
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
                assert_eq!(w.ui.pending_file_writes, 0);
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
