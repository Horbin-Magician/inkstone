use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
};
use inkstone_core::link_audit::Issue;
pub(super) struct Review {
    issues: Vec<Issue>,
    loading: bool,
    message: String,
}
impl Workspace {
    pub(super) fn open_link_health(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        self.flush_document_views(window, cx);
        let drafts: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| !t.save.editor.read(cx).is_composing())
            .map(|t| (t.path.clone(), t.save.editor.read(cx).value()))
            .collect();
        self.close_overlays(window, cx);
        self.ui.link_health = Some(Review {
            issues: vec![],
            loading: true,
            message: String::new(),
        });
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        let generation = self.generation;
        let request = self.ui.recovery_refresh;
        let task = cx.background_executor().spawn(async move {
            let mut index = Index::build(&vault)?;
            for (path, text) in drafts {
                index.update(path, text.to_string());
            }
            Ok::<_, VaultError>(inkstone_core::link_audit::scan(&vault.root, &index))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.ui.recovery_refresh != request {
                    return;
                }
                if let Some(review) = &mut this.ui.link_health {
                    review.loading = false;
                    match result {
                        Ok(issues) => review.issues = issues,
                        Err(error) => review.message = error.to_string(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn link_health_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = &self.ui.link_health else {
            return div().into_any_element();
        };
        div().flex().flex_col().gap_2().min_h_0()
            .child("检查本地笔记、附件、标题与块引用，包含打开正文快照；不访问外部网址。点击问题可打开来源笔记，修改后请重新检查。")
            .child(format!("{} 个待检查问题",r.issues.len()))
            .when(r.loading,|s|s.child("正在检查……"))
            .when(!r.message.is_empty(),|s|s.child(r.message.clone()))
            .when(!r.loading&&r.message.is_empty()&&r.issues.is_empty(),|s|s.child("本次扫描未发现失效的直接引用。"))
            .child(Button::new("link-health-refresh").label("重新检查").disabled(r.loading).on_click(cx.listener(|this,_,w,cx|this.open_link_health(w,cx))))
            .child(uniform_list("link-health-list",r.issues.len(),cx.processor(|this,range:std::ops::Range<usize>,_,cx|{
                let Some(r)=&this.ui.link_health else{return vec![];};range.filter_map(|i|{let issue=r.issues.get(i)?;let path=issue.from.clone();Some(Button::new(("link-issue",i)).ghost().label(format!("{} · {} → {}",issue.reason,path.display(),issue.target)).on_click(cx.listener(move|this,_,w,cx|{this.close_overlays(w,cx);this.open_note(path.clone(),w,cx);})).into_any_element())}).collect()
            })).h(px(320.)).w_full()).into_any_element()
    }
}
