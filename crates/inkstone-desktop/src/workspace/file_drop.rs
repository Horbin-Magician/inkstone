//! System file drops have distinct destinations from clipboard attachments.
use super::*;

impl Workspace {
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
