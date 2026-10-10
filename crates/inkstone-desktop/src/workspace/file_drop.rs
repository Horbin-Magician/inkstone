//! System file drops have distinct destinations from clipboard attachments.
use super::*;

struct DroppedNote {
    vault: Vault,
    path: PathBuf,
    text: String,
    current: bool,
}

fn read_dropped_note(
    source: &std::path::Path,
    current: Option<&Vault>,
) -> Result<DroppedNote, String> {
    let source = std::fs::canonicalize(source).map_err(|e| e.to_string())?;
    if !source.is_file()
        || source
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("md"))
    {
        return Err("阅读区域仅支持打开 Markdown 文件".into());
    }
    let (vault, path, current) = if let Some((vault, path)) = current.and_then(|vault| {
        source
            .strip_prefix(&vault.root)
            .ok()
            .map(|path| (vault.clone(), path.to_path_buf()))
    }) {
        (vault, path, true)
    } else {
        let vault = Vault::open(
            source.parent().ok_or("文件没有父目录")?,
            app_dir().join("recovery"),
        )
        .map_err(|e| e.to_string())?;
        let path = PathBuf::from(source.file_name().ok_or("文件名无效")?);
        (vault, path, false)
    };
    let text = vault
        .read(&path)
        .map_err(|e| e.to_string())?
        .ok_or("文件已不存在")?;
    Ok(DroppedNote {
        vault,
        path,
        text,
        current,
    })
}

impl Workspace {
    fn install_standalone_note(
        &mut self,
        note: DroppedNote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.startup_pending = false;
        self.standalone = true;
        self.vault = Some(note.vault);
        self.ui.prefs.left_open = false;
        self.ui.prefs.right_open = false;
        Arc::make_mut(&mut self.index).update(note.path.clone(), note.text.clone());
        self.files = self.index.note_paths();
        self.add_tab(note.path, Some(note.text), false, window, cx);
        cx.notify();
    }

    pub(super) fn open_external_files(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_is_open() || self.ui.tree_name.is_some() || self.loading {
            return;
        }
        let vault = self.vault.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            paths
                .iter()
                .map(|path| {
                    read_dropped_note(path, vault.as_ref())
                        .map_err(|e| format!("{}：{e}", path.display()))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let notes = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                for result in notes {
                    match result {
                        Ok(note) if note.current => {
                            if let Some(index) = this.existing_note_target(&note.path) {
                                this.activate_tab(index, window, cx);
                            } else {
                                this.views.secondary_focused = false;
                                this.add_tab(note.path, Some(note.text), false, window, cx);
                            }
                        }
                        Ok(note) if this.vault.is_none() && this.tabs.is_empty() => {
                            this.install_standalone_note(note, window, cx);
                        }
                        Ok(note) => {
                            let result = cx.open_window(
                                WindowOptions {
                                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                        point(px(100.), px(80.)),
                                        size(px(1000.), px(760.)),
                                    ))),
                                    window_min_size: Some(size(px(800.), px(500.))),
                                    titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                                    ..gpui_component::TitleBar::window_options()
                                },
                                |window, cx| {
                                    let view = cx.new(|cx| {
                                        let mut workspace = Workspace::new(window, cx);
                                        workspace.install_standalone_note(note, window, cx);
                                        workspace
                                    });
                                    #[cfg(target_os = "macos")]
                                    crate::app_menu::init(&view, window, cx);
                                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                                },
                            );
                            if let Err(error) = result {
                                this.notifications
                                    .publish(format!("无法打开文件窗口：{error}"));
                            }
                        }
                        Err(error) => this.notifications.publish(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn import_tree_files(
        &mut self,
        paths: Vec<PathBuf>,
        folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_is_open() || self.ui.tree_name.is_some() || self.loading {
            return;
        }
        let Some(vault) = self.vault.clone() else {
            return;
        };
        let Some(ticket) = self.file_writes.try_begin_exclusive_operation() else {
            return;
        };
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    vault
                        .import_attachment_to(&folder, &path)
                        .map_err(|error| format!("{}：{error}", path.display()))
                })
                .collect::<Vec<_>>()
        });
        cx.spawn_in(window, async move |this, cx| {
            let results = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_writes.finish(ticket);
                if this.generation != generation {
                    return;
                }
                let count = results.iter().filter(|result| result.is_ok()).count();
                let errors = results
                    .into_iter()
                    .filter_map(Result::err)
                    .collect::<Vec<_>>();
                this.notifications.publish(if errors.is_empty() {
                    format!("已导入 {count} 个文件")
                } else {
                    format!("已导入 {count} 个文件；{}", errors.join("；"))
                });
                this.structure_changed = true;
                this.schedule_auto_sync(true);
                this.refresh(window, cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn external_reading_drop_keeps_current_vault_and_unsaved_text(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-drop-window-{}", std::process::id()));
        let target = root.join("vault");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(root.join("outside.md"), "outside").unwrap();
        std::fs::write(target.join("inside.md"), "inside").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&target, root.join("recovery")).unwrap());
                w.add_tab("inside.md".into(), Some("inside".into()), false, window, cx);
                w.tabs[0]
                    .pane
                    .read(cx)
                    .editor
                    .clone()
                    .update(cx, |s, cx| s.set_value("unsaved", window, cx));
                w.open_external_files(vec![root.join("outside.md")], window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.windows().len(), 2));
        handle
            .update(cx, |w, _, cx| {
                assert!(!w.standalone);
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), "unsaved");
                assert_eq!(
                    w.vault.as_ref().unwrap().root,
                    std::fs::canonicalize(&target).unwrap()
                );
            })
            .unwrap();
        assert!(!target.join("outside.md").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("outside.md")).unwrap(),
            "outside"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn external_reading_drop_saves_original_without_workspace_metadata(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-external-open-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("original.md");
        std::fs::write(&source, "original").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.open_external_files(vec![source.clone()], window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |w, window, cx| {
                assert!(w.standalone);
                assert!(!w.startup_pending);
                assert_eq!(w.tabs.len(), 1);
                assert_eq!(
                    w.vault.as_ref().unwrap().root,
                    std::fs::canonicalize(&root).unwrap()
                );
                let editor = w.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_value("edited original", window, cx));
                w.save_all(window, cx);
                w.persist_workspace(cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "edited original");
        assert!(!root.join(".inkstone-workspace.json").exists());
        handle
            .update(cx, |w, window, cx| {
                assert!(w.request_window_close(window, cx));
                // A later external edit must still trigger the normal save conflict.
                std::fs::write(&source, "outside change").unwrap();
                let editor = w.tabs[0].pane.read(cx).editor.clone();
                editor.update(cx, |s, cx| s.set_value("local change", window, cx));
                w.save_all(window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "outside change");
        handle
            .update(cx, |w, _, _| {
                assert!(w.tabs[0].save.persistence.has_conflict())
            })
            .unwrap();
        assert!(read_dropped_note(&root, None).is_err());
        std::fs::write(root.join("binary.png"), [0, 1, 2]).unwrap();
        assert!(read_dropped_note(&root.join("binary.png"), None).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn external_drop_routes_folder_root_and_reading_area(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-drop-routing-{}", std::process::id()));
        let source = root.join("source.md");
        let target = root.join("vault");
        std::fs::create_dir_all(target.join("folder")).unwrap();
        std::fs::write(&source, "dropped").unwrap();
        std::fs::write(target.join("existing.md"), "unchanged").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                let vault = Vault::open(&target, root.join("recovery")).unwrap();
                w.index = Arc::new(Index::build(&vault).unwrap());
                w.files = w.index.note_paths();
                w.ui.folders = vec!["folder".into()];
                w.vault = Some(vault);
                w.ui.prefs.left_open = true;
                w.ui.left_mode = 0;
                w.add_tab(
                    "existing.md".into(),
                    Some("unchanged".into()),
                    false,
                    window,
                    cx,
                );
                w.rebuild_sorted_tree(cx);
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.simulate_resize(size(px(1100.), px(800.)));
        for (selector, path) in [
            ("tree-name-0", source.clone()),
            ("tree-root-drop", source),
            ("file-open-drop", target.join("folder/source.md")),
        ] {
            visual.update(|window, cx| window.draw(cx).clear(cx));
            let position = visual.debug_bounds(selector).unwrap().center();
            visual.update(|window, cx| {
                window.dispatch_event(
                    PlatformInput::FileDrop(FileDropEvent::Entered {
                        position,
                        paths: ExternalPaths(vec![path].into()),
                    }),
                    cx,
                );
                window.draw(cx).clear(cx);
                window.dispatch_event(
                    PlatformInput::FileDrop(FileDropEvent::Submit { position }),
                    cx,
                );
            });
            visual.run_until_parked();
        }
        assert_eq!(
            std::fs::read_to_string(target.join("folder/source.md")).unwrap(),
            "dropped"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("source.md")).unwrap(),
            "dropped"
        );
        handle
            .update(&mut visual, |w, _, cx| {
                assert_eq!(w.tabs.len(), 2);
                assert_eq!(
                    w.tabs[w.active.unwrap()].path,
                    PathBuf::from("folder/source.md")
                );
                assert_eq!(w.tabs[0].pane.read(cx).editor.read(cx).value(), "unchanged");
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui::test]
    fn external_tree_drop_copies_files_without_an_open_note(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let root =
            std::env::temp_dir().join(format!("inkstone-external-tree-{}", std::process::id()));
        let source = root.join("source");
        let target = root.join("vault");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(target.join("folder")).unwrap();
        std::fs::write(source.join("note.md"), "imported").unwrap();
        std::fs::write(target.join("folder/note.md"), "existing").unwrap();
        let handle = cx.add_window(Workspace::new);
        handle
            .update(cx, |w, window, cx| {
                w.vault = Some(Vault::open(&target, root.join("recovery")).unwrap());
                w.import_tree_files(
                    vec![source.join("note.md"), source.join("missing.md")],
                    "folder".into(),
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(target.join("folder/note.md")).unwrap(),
            "existing"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("folder/note 1.md")).unwrap(),
            "imported"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("note.md")).unwrap(),
            "imported"
        );
        handle
            .update(cx, |w, _, _| {
                assert!(w.tabs.is_empty());
                assert_eq!(w.file_writes.pending(), 0);
                assert!(w.files.contains(&PathBuf::from("folder/note 1.md")));
            })
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
