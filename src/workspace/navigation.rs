use super::*;
use inkstone::preferences::Navigation;

pub(super) struct PendingNavigation {
    path: PathBuf,
    text: Option<String>,
    source: usize,
    pane: Entity<EditorPane>,
    secondary: bool,
    generation: u64,
    history: Navigation,
}

impl Workspace {
    pub(super) fn open_current_note(
        &mut self,
        path: PathBuf,
        mut history: Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active else { return };
        let Some(pane) = self.current_pane() else {
            return;
        };
        if self.tabs[index].path == path {
            history.visit(path);
            pane.update(cx, |p, _| {
                p.navigation = history;
            });
            self.activate_tab(index, window, cx);
            self.apply_jump(window, cx);
            return;
        }
        if self.has_pending_input(self.tabs[index].id, window, cx) {
            self.status = "请完成当前编辑后再打开其他笔记。".into();
            self.pending_jump = None;
            cx.notify();
            return;
        }
        history.visit(path.clone());
        let mut pending = PendingNavigation {
            path: path.clone(),
            text: None,
            source: self.tabs[index].id,
            pane,
            secondary: self.views.secondary_focused,
            generation: self.navigation_generation,
            history,
        };
        if self.tabs.iter().any(|tab| tab.path == path) {
            self.pending_navigation = Some(pending);
            self.finish_pending_navigation(window, cx);
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let generation = self.generation;
        let task = cx
            .background_executor()
            .spawn(async move { vault.read(&path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.navigation_generation != pending.generation
                {
                    return;
                }
                match result {
                    Ok(Some(text)) => {
                        pending.text = Some(text);
                        this.pending_navigation = Some(pending);
                        this.finish_pending_navigation(window, cx);
                    }
                    Ok(None) => {
                        this.status = "文件已不存在，请刷新目录。".into();
                        this.pending_jump = None;
                        cx.notify();
                    }
                    Err(error) => {
                        this.status = error.to_string();
                        this.pending_jump = None;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn finish_pending_navigation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_navigation.take() else {
            return;
        };
        if pending.generation != self.navigation_generation
            || self.current_pane().is_none_or(|pane| pane != pending.pane)
            || self.views.secondary_focused != pending.secondary
        {
            return;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == pending.source) else {
            return;
        };
        if self.has_pending_input(pending.source, window, cx) {
            self.status = "请完成当前编辑后再打开其他笔记。".into();
            self.pending_jump = None;
            cx.notify();
            return;
        }
        if self.ui.inline_title.is_some() {
            self.commit_inline_title(false, window, cx);
            if self.ui.inline_title.is_some() {
                self.status = "请完成或取消标题修改后再打开其他笔记。".into();
                self.pending_jump = None;
                cx.notify();
                return;
            }
        }
        self.flush_document_views(window, cx);
        let save = self.tabs[index].save.clone();
        if save.conflict.get() || save.error.borrow().is_some() {
            self.status = "请先处理当前笔记的保存问题，再打开其他笔记。".into();
            self.pending_jump = None;
            cx.notify();
            return;
        }
        if save.dirty.get() || save.saving.get() || self.ui.file_operation {
            self.pending_navigation = Some(pending);
            self.save_pending(window, cx);
            return;
        }
        if let Some(target) = self.tabs.iter().find(|tab| tab.path == pending.path)
            && self.has_pending_input(target.id, window, cx)
        {
            self.status = "请完成目标笔记的编辑后再打开另一个视图。".into();
            self.pending_jump = None;
            cx.notify();
            return;
        }
        // Keep a backing tab when the other pane still displays this document.
        let keep_source = if pending.secondary {
            self.views.main == Some(pending.source)
        } else {
            self.views
                .split
                .as_ref()
                .is_some_and(|split| split.source == pending.source)
        };
        let pinned = self.current_view_pinned();
        let reading = pending.pane.read(cx).reading;
        let live = pending.pane.read(cx).live;
        let jump = self.pending_jump.take();
        self.add_tab(pending.path, pending.text, false, window, cx);
        let Some(new_index) = self.active else { return };
        let new_id = self.tabs[new_index].id;
        if new_id == pending.source {
            return;
        }
        if let Some(pane) = self.current_pane() {
            pane.update(cx, |p, cx| {
                p.navigation = pending.history;
                p.reading = reading;
                p.live = live;
                p.focus_view(window, cx);
                cx.notify();
            });
        }
        if pending.secondary {
            if let Some(split) = &mut self.views.split {
                split.pinned = pinned;
            }
        } else {
            self.tabs[new_index].pinned = pinned;
        }
        let new_tab = self.tabs.remove(new_index);
        if !keep_source {
            self.tabs.retain(|tab| tab.id != pending.source);
        }
        self.tabs.insert(index.min(self.tabs.len()), new_tab);
        self.active = self.tabs.iter().position(|tab| tab.id == new_id);
        self.pending_jump = jump;
        self.apply_jump(window, cx);
        self.ui.close_pending.remove(&pending.source);
        self.persist_workspace(cx);
        cx.notify();
    }

    pub(super) fn relocate_navigation(
        &mut self,
        old: &std::path::Path,
        new: Option<&std::path::Path>,
        folder: bool,
        cx: &mut Context<Self>,
    ) {
        let panes: Vec<_> = self
            .tabs
            .iter()
            .map(|tab| tab.pane.clone())
            .chain(self.views.split.iter().map(|split| split.pane.clone()))
            .collect();
        for pane in panes {
            pane.update(cx, |pane, _| {
                let history = &mut pane.navigation;
                let mut before = 0;
                let cursor = history.cursor;
                let mut position = 0;
                history.entries.retain_mut(|path| {
                    let affected = if folder {
                        path.starts_with(old)
                    } else {
                        path == old
                    };
                    let keep = !affected || new.is_some();
                    if affected && let Some(new) = new {
                        *path = if folder {
                            new.join(path.strip_prefix(old).unwrap())
                        } else {
                            new.to_owned()
                        };
                    }
                    if position < cursor && !keep {
                        before += 1;
                    }
                    position += 1;
                    keep
                });
                history.cursor = cursor
                    .saturating_sub(before)
                    .min(history.entries.len().saturating_sub(1));
            });
        }
        self.navigation_generation += 1;
        self.pending_navigation = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn invalid_inline_title_keeps_the_source_and_draft(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("b.md".into(), Some("B".into()), false, window, cx);
                w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                let pane = w.current_pane().unwrap();
                w.begin_inline_title(1, false, window, cx);
                let input = w.ui.inline_title.as_ref().unwrap().input.clone();
                input.update(cx, |s, cx| s.set_value("bad/name", window, cx));
                w.open_note("b.md".into(), window, cx);
                assert_eq!(w.current_pane().unwrap(), pane);
                assert_eq!(w.tabs[1].path, PathBuf::from("a.md"));
                assert_eq!(input.read(cx).value().as_ref(), "bad/name");
                assert!(w.ui.inline_title.is_some());
                assert!(w.pending_navigation.is_none());
            })
            .unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("inkstone-navigation-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for (path, text) in [("a.md", "A"), ("b.md", "B"), ("c.md", "C")] {
            std::fs::write(root.join(path), text).unwrap();
        }
        root
    }

    #[gpui::test]
    fn loading_does_not_follow_focus_changes_and_missing_files_keep_history(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let root = fixture("cancel");
        let handle = cx.add_window(Workspace::new);
        let source = handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                let source = w.current_pane().unwrap();
                w.add_tab("c.md".into(), Some("C".into()), false, window, cx);
                w.focus_primary(0, window, cx);
                w.open_note("b.md".into(), window, cx);
                w.focus_primary(1, window, cx);
                w.focus_primary(0, window, cx);
                source
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.current_pane().unwrap(), source);
                assert_eq!(w.tabs[0].path, PathBuf::from("a.md"));
                w.open_note("missing.md".into(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.current_pane().unwrap(), source);
                assert_eq!(
                    source.read(cx).navigation.entries,
                    vec![PathBuf::from("a.md")]
                );
                source.update(cx, |p, _| {
                    p.navigation.visit("folder/b.md".into());
                    p.navigation.visit("folder/c.md".into());
                    p.navigation.visit("d.md".into());
                });
                w.relocate_navigation(
                    std::path::Path::new("folder"),
                    Some(std::path::Path::new("moved")),
                    true,
                    cx,
                );
                assert_eq!(
                    source.read(cx).navigation.entries[1],
                    PathBuf::from("moved/b.md")
                );
                w.relocate_navigation(std::path::Path::new("moved"), None, true, cx);
                assert_eq!(
                    source.read(cx).navigation.entries,
                    vec![PathBuf::from("a.md"), "d.md".into()]
                );
                assert_eq!(source.read(cx).navigation.cursor, 1);
            })
            .unwrap();
    }

    #[gpui::test]
    fn replacement_keeps_slot_and_histories_are_independent(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = fixture("history");
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                w.add_tab("c.md".into(), Some("C".into()), false, window, cx);
                w.focus_primary(0, window, cx);
                w.open_note("b.md".into(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(w.active, Some(0));
                assert_eq!(w.tabs[0].path, PathBuf::from("b.md"));
                assert!(w.ui.closed.is_empty());
                assert_eq!(
                    w.tabs[0].pane.read(cx).navigation.entries,
                    vec![PathBuf::from("a.md"), "b.md".into()]
                );
                assert_eq!(
                    w.tabs[1].pane.read(cx).navigation.entries,
                    vec![PathBuf::from("c.md")]
                );
                w.execute_command(20, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs[0].path, PathBuf::from("a.md"));
                assert_eq!(w.tabs[0].pane.read(cx).navigation.cursor, 0);
                w.execute_command(21, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs[0].path, PathBuf::from("b.md"));
                w.split_active(true, window, cx);
                let main = w.tabs[0].pane.clone();
                w.open_note("c.md".into(), window, cx);
                assert_eq!(
                    w.current_pane().unwrap().read(cx).current_path,
                    PathBuf::from("c.md")
                );
                assert_eq!(main.read(cx).navigation.entries.len(), 2);
                assert_eq!(
                    w.current_pane().unwrap().read(cx).navigation.entries.len(),
                    3
                );
                w.execute_command(20, window, cx);
                assert!(w.views.secondary_focused);
                assert_eq!(
                    w.current_pane().unwrap().read(cx).current_path,
                    PathBuf::from("b.md")
                );
                assert_eq!(w.current_pane().unwrap().read(cx).navigation.cursor, 1);
                assert_eq!(main.read(cx).navigation.cursor, 1);
            })
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui::test]
    fn replacement_waits_for_save_and_retains_source_on_conflict(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root = fixture("save");
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&root, app_dir().join("recovery")).unwrap());
                w.add_tab("b.md".into(), Some("B".into()), false, window, cx);
                w.add_tab("a.md".into(), Some("A".into()), false, window, cx);
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("改动", window, cx));
                w.open_note("b.md".into(), window, cx);
                assert_eq!(w.tabs[1].path, PathBuf::from("a.md"));
                assert!(w.pending_navigation.is_some());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "改动");
        handle
            .update(cx, |w, window, cx| {
                assert_eq!(w.tabs[1].path, PathBuf::from("b.md"));
                assert!(w.pending_navigation.is_none());
                std::fs::write(root.join("b.md"), "外部改动").unwrap();
                w.current_pane()
                    .unwrap()
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("本地改动", window, cx));
                w.open_note("c.md".into(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, _, cx| {
                assert_eq!(w.tabs[1].path, PathBuf::from("b.md"));
                assert_eq!(
                    w.current_pane()
                        .unwrap()
                        .read(cx)
                        .editor
                        .read(cx)
                        .value()
                        .as_ref(),
                    "本地改动"
                );
                assert!(w.tabs[1].save.conflict.get());
                assert!(w.pending_navigation.is_none());
                assert_eq!(
                    w.current_pane().unwrap().read(cx).navigation.entries.len(),
                    2
                );
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("b.md")).unwrap(),
            "外部改动"
        );
    }
}
