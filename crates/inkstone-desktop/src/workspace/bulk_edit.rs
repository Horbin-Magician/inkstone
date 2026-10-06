use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
};
use inkstone_core::batch::{self, Plan, Spec};
use std::collections::BTreeSet;

pub(super) struct Review {
    find: Entity<InputState>,
    replacement: Entity<InputState>,
    folder: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    tags: bool,
    regex: bool,
    case_sensitive: bool,
    descendants: bool,
    plan: Option<Plan>,
    selected: BTreeSet<usize>,
    loading: bool,
    message: String,
    preview: Entity<TextareaState>,
}
impl Review {
    fn spec(&self, cx: &App) -> Spec {
        Spec {
            find: self.find.read(cx).value().to_string(),
            replacement: self.replacement.read(cx).value().to_string(),
            folder: self.folder.read(cx).value().to_string(),
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            tags: self.tags,
            descendants: self.descendants,
        }
    }
}
impl Workspace {
    pub(super) fn open_bulk_edit(
        &mut self,
        tags: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vault.is_none() {
            return;
        }
        self.close_overlays(window, cx);
        let find = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if tags {
                "旧标签（不含 #）"
            } else {
                "查找内容"
            })
        });
        let replacement = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if tags {
                "新标签 / 合并到标签"
            } else {
                "替换为（可为空）"
            })
        });
        let folder =
            cx.new(|cx| InputState::new(window, cx).placeholder("范围：库内文件夹，留空为全库"));
        let subscriptions = [&find, &replacement, &folder]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event: &InputEvent, w, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.invalidate_bulk_preview(w, cx);
                    }
                })
            })
            .collect();
        let preview = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).rows(10);
            s.set_readonly(true, cx);
            s
        });
        self.ui.bulk_edit = Some(Review {
            find,
            replacement,
            folder,
            _subscriptions: subscriptions,
            tags,
            regex: false,
            case_sensitive: false,
            descendants: true,
            plan: None,
            selected: Default::default(),
            loading: false,
            message: String::new(),
            preview,
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        cx.notify();
    }
    fn invalidate_bulk_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        if let Some(r) = &mut self.ui.bulk_edit {
            r.plan = None;
            r.selected.clear();
            r.loading = false;
            r.message = "条件已改变，请重新预览。".into();
            r.preview.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }
    fn preview_bulk_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        if self.file_writes.operation_active()
            || self.file_writes.pending() > 0
            || self.tabs.iter().any(|t| {
                t.save.persistence.is_dirty()
                    || t.save.persistence.is_saving()
                    || t.save.persistence.has_conflict()
                    || self.has_pending_input(t.id, window, cx)
            })
        {
            if let Some(r) = &mut self.ui.bulk_edit {
                r.message = "请先保存笔记、完成组词并处理冲突，再生成批量修改预览。".into();
            }
            cx.notify();
            return;
        }
        let Some(r) = &mut self.ui.bulk_edit else {
            return;
        };
        let spec = r.spec(cx);
        r.loading = true;
        r.plan = None;
        r.selected.clear();
        r.message = "正在生成预览，尚未修改文件……".into();
        self.ui.recovery_refresh = self.ui.recovery_refresh.wrapping_add(1);
        let request = self.ui.recovery_refresh;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            Index::build(&vault)
                .map_err(|e| e.to_string())
                .and_then(|index| batch::plan(&index, &spec))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, w, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                let Some(r) = &mut this.ui.bulk_edit else {
                    return;
                };
                r.loading = false;
                match result {
                    Ok(plan) => {
                        r.message = format!(
                            "预览 {} 篇笔记、{} 处修改；跳过/警告 {} 项。{}",
                            plan.edits.len(),
                            plan.edits.iter().map(|e| e.count).sum::<usize>(),
                            plan.warnings.len(),
                            plan.warnings
                                .iter()
                                .take(10)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("；")
                        );
                        r.selected = (0..plan.edits.len()).collect();
                        r.plan = Some(plan);
                        this.select_bulk_preview(0, w, cx);
                    }
                    Err(error) => r.message = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn select_bulk_preview(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = &self.ui.bulk_edit else {
            return;
        };
        let Some(edit) = r.plan.as_ref().and_then(|p| p.edits.get(index)).cloned() else {
            return;
        };
        self.ui.bulk_preview_revision = self.ui.bulk_preview_revision.wrapping_add(1);
        let revision = self.ui.bulk_preview_revision;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let diff = similar::TextDiff::configure()
                .timeout(Duration::from_millis(100))
                .diff_lines(&edit.before, &edit.after);
            recovery::preview_text(
                &diff
                    .unified_diff()
                    .context_radius(2)
                    .header("修改前", "修改后")
                    .to_string(),
            )
            .replace("恢复副本仍包含完整正文", "实际修改仍使用完整正文")
        });
        cx.spawn_in(window, async move |this, cx| {
            let text = task.await;
            let _ = this.update_in(cx, |this, w, cx| {
                if this.ui.recovery_refresh != request || this.ui.bulk_preview_revision != revision
                {
                    return;
                }
                if let Some(r) = &this.ui.bulk_edit {
                    r.preview.update(cx, |s, cx| s.set_value(text, w, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn apply_bulk_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_writes.operation_active() || self.file_writes.pending() > 0 {
            return;
        }
        self.flush_document_views(window, cx);
        let Some(r) = &self.ui.bulk_edit else {
            return;
        };
        if r.loading {
            return;
        }
        let Some(plan) = &r.plan else {
            return;
        };
        let edits: Vec<_> = r
            .selected
            .iter()
            .filter_map(|i| plan.edits.get(*i).cloned())
            .collect();
        if edits.is_empty() {
            return;
        }
        let stale = edits.iter().any(|e| {
            self.tabs.iter().filter(|t| t.path == e.path).any(|t| {
                t.save.persistence.is_saving()
                    || t.save.persistence.has_conflict()
                    || self.has_pending_input(t.id, window, cx)
                    || t.save.editor.read(cx).value().as_ref() != e.before
                    || t.save.persistence.baseline().as_deref() != Some(e.before.as_str())
            })
        });
        if stale {
            self.invalidate_bulk_preview(window, cx);
            if let Some(r) = &mut self.ui.bulk_edit {
                r.message = "待修改笔记在预览后发生了编辑，请先保存并重新预览。".into();
            }
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let write_ticket = self.file_writes.begin_operation();
        let generation = self.generation;
        if let Some(r) = &mut self.ui.bulk_edit {
            r.loading = true;
            r.plan = None;
            r.selected.clear();
            r.message = "正在应用已审阅修改，每篇笔记保存前会记录版本……".into();
        }
        let task = cx
            .background_executor()
            .spawn(async move { batch::apply(&vault, edits) });
        cx.spawn_in(window,async move|this,cx|{let result=task.await;let _=this.update_in(cx,|this,w,cx|{
            this.file_writes.finish(write_ticket);if this.generation!=generation{return;}
            this.flush_document_views(w,cx);
            let mut changed=vec![];let mut conflicts=0;
            for edit in &result.written {
                if let Some(tab)=this.tabs.iter().find(|t|t.path==edit.path){
                    let document=tab.save.clone();let pending=this.has_pending_input(tab.id,w,cx);
                    if !document.persistence.apply_reviewed_edit(&edit.before, &edit.after, document.editor.read(cx).value().as_ref(), pending){
                        conflicts+=1;
                    }else{document.editor.update(cx,|s,cx|{let range=s.selected_range();s.replace_all(&edit.after,w,cx);s.set_selected_range(range,cx);});changed.push(document.editor.clone());}
                }
            }
            for editor in changed{this.document_changed(editor,w,cx);}
            let message=format!("已修改 {} 篇笔记；失败 {} 篇；{} 篇处理中产生的新编辑已保留为冲突。可在各笔记版本历史查看修改前内容。{}",result.written.len(),result.errors.len(),conflicts,result.errors.join("；"));
            this.notifications.publish(message.clone());if let Some(r)=&mut this.ui.bulk_edit{r.loading=false;r.message=message;}
            for edit in &result.written { this.changed_paths.insert(edit.path.clone()); }
            if !result.written.is_empty() { this.schedule_auto_sync(true); }
            this.refresh_requested=true;cx.notify();
        });}).detach();
        cx.notify();
    }
    pub(super) fn bulk_edit_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = &self.ui.bulk_edit else {
            return div().into_any_element();
        };
        let count = r.plan.as_ref().map_or(0, |p| p.edits.len());
        div().id("bulk-edit-panel").flex().flex_col().gap_2().min_h_0().overflow_y_scroll()
            .child(if r.tags{"标签重命名 / 合并：同时处理正文标签和 YAML 标签属性，忽略代码；标签匹配不区分大小写。"}else{"全库文本替换：包含正文、代码和属性。先预览差异，可逐篇取消选择，再应用。正则替换支持 $1 等捕获组。"})
            .child(Input::new(&r.find)).child(Input::new(&r.replacement)).child(Input::new(&r.folder))
            .child(div().flex().gap_2()
                .when(!r.tags,|s|s.child(Button::new("bulk-case").label("区分大小写").when(r.case_sensitive,|b|b.primary()).on_click(cx.listener(|this,_,w,cx|{if let Some(r)=&mut this.ui.bulk_edit{r.case_sensitive = !r.case_sensitive;}this.invalidate_bulk_preview(w,cx);})))
                    .child(Button::new("bulk-regex").label("正则表达式").when(r.regex,|b|b.primary()).on_click(cx.listener(|this,_,w,cx|{if let Some(r)=&mut this.ui.bulk_edit{r.regex = !r.regex;}this.invalidate_bulk_preview(w,cx); }))))
                .when(r.tags,|s|s.child(Button::new("bulk-descendants").label("包含子标签").when(r.descendants,|b|b.primary()).on_click(cx.listener(|this,_,w,cx|{if let Some(r)=&mut this.ui.bulk_edit{r.descendants = !r.descendants;}this.invalidate_bulk_preview(w,cx);}))))
                .child(Button::new("bulk-preview").label("生成预览").disabled(r.loading||self.file_writes.operation_active()).on_click(cx.listener(|this,_,w,cx|this.preview_bulk_edit(w,cx))))
                .child(Button::new("bulk-apply").primary().label(format!("应用选中 {} 篇",r.selected.len())).disabled(r.loading||r.selected.is_empty()||self.file_writes.operation_active()).on_click(cx.listener(|this,_,w,cx|this.apply_bulk_edit(w,cx)))))
            .child(r.message.clone())
            .child(uniform_list("bulk-files",count,cx.processor(|this,range:std::ops::Range<usize>,_,cx|{
                let Some(r)=&this.ui.bulk_edit else{return vec![];};let Some(p)=&r.plan else{return vec![];};range.filter_map(|i|{let edit=p.edits.get(i)?;Some(div().flex().gap_1().child(Button::new(("bulk-check",i)).label(if r.selected.contains(&i){"☑"}else{"☐"}).accessibility_label(format!("选择 {}",edit.path.display())).on_click(cx.listener(move|this,_,_,cx|{if let Some(r)=&mut this.ui.bulk_edit && !r.selected.remove(&i){r.selected.insert(i);}cx.notify();})))
                    .child(Button::new(("bulk-file",i)).ghost().label(format!("{} · {} 处",edit.path.display(),edit.count)).on_click(cx.listener(move|this,_,w,cx|this.select_bulk_preview(i,w,cx)))).into_any_element())}).collect()
            })).h(px(120.)).w_full())
            .child(Textarea::new(&r.preview).readonly(true).h(px(230.)))
            .child(div().text_sm().child("应用前会核对所有磁盘基线；执行过程中仍可能出现部分成功，会逐项报告。关闭预览后，已开始的写入继续完成。"))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn batch_preview_selection_and_inflight_edit_preservation(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "inkstone-bulk-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a.md", "b.md"] {
            std::fs::write(root.join(name), "old").unwrap();
        }
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| w.load_vault(root.clone(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.open_note("a.md".into(), window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                w.open_bulk_edit(false, window, cx);
                let r = w.ui.bulk_edit.as_ref().unwrap();
                r.find.update(cx, |s, cx| s.set_value("old", window, cx));
                r.replacement
                    .update(cx, |s, cx| s.set_value("new 😀", window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| w.preview_bulk_edit(window, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                let r = w.ui.bulk_edit.as_mut().unwrap();
                assert_eq!(r.plan.as_ref().unwrap().edits.len(), 2);
                assert!(r.preview.read(cx).value().contains("new 😀"));
                r.selected.remove(&1);
                w.apply_bulk_edit(window, cx);
                w.tabs[0].save.editor.update(cx, |s, cx| {
                    s.replace_all("typed while applying", window, cx)
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(root.join("a.md")).unwrap(),
            "new 😀"
        );
        assert_eq!(std::fs::read_to_string(root.join("b.md")).unwrap(), "old");
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(
                    w.tabs[0].save.editor.read(cx).value().as_ref(),
                    "typed while applying"
                );
                assert!(w.tabs[0].save.persistence.has_conflict());
                assert_eq!(w.file_writes.pending(), 0);
                assert!(
                    w.ui.bulk_edit
                        .as_ref()
                        .unwrap()
                        .message
                        .contains("已保留为冲突")
                );
                w.preview_bulk_edit(window, cx);
                assert!(
                    w.ui.bulk_edit
                        .as_ref()
                        .unwrap()
                        .message
                        .contains("处理冲突")
                );
                w.watcher = None;
                for h in w
                    .vault
                    .as_ref()
                    .unwrap()
                    .history(PathBuf::from("a.md").as_path())
                    .unwrap()
                {
                    std::fs::remove_file(h.journal).unwrap();
                }
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
