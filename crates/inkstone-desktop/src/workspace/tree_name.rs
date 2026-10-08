//! In-place file-tree naming. Pending rows never touch the vault.
use super::ui::NameMode;
use super::*;

pub(super) struct TreeName {
    pub row: PathBuf,
    pub parent: PathBuf,
    pub creating: bool,
    pub folder: bool,
    pub note_id: Option<usize>,
}

impl Workspace {
    pub(super) fn begin_tree_name(
        &mut self,
        mode: NameMode,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vault.is_none() || self.file_writes.operation_active() {
            return;
        }
        self.cancel_tree_name(cx);
        let creating = matches!(mode, NameMode::New | NameMode::Folder);
        let folder = matches!(mode, NameMode::Folder | NameMode::RenameFolder);
        let parent = if creating {
            path.clone()
        } else {
            path.parent()
                .unwrap_or(std::path::Path::new(""))
                .to_path_buf()
        };
        let row = if creating {
            // A reserved, UI-only path keeps the placeholder distinct from real files.
            let mut suffix = 0;
            loop {
                let row = parent.join(format!(".inkstone-pending-name-{suffix}"));
                if !self.vault.as_ref().unwrap().root.join(&row).exists() {
                    break row;
                }
                suffix += 1;
            }
        } else {
            path.clone()
        };
        let value = if creating {
            String::new()
        } else if folder {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        };
        self.ui.tree_name = Some(TreeName {
            row: row.clone(),
            parent: parent.clone(),
            creating,
            folder,
            note_id: self
                .tabs
                .iter()
                .find(|tab| tab.path == path)
                .map(|tab| tab.id),
        });
        self.ui.name_mode = Some(mode);
        self.ui.folder_target = (mode == NameMode::RenameFolder).then_some(path);
        self.command_open = false;
        self.ui.left_mode = 0;
        self.ui.prefs.left_open = true;
        self.ui.focus_mode = false;
        for ancestor in parent.ancestors().filter(|p| !p.as_os_str().is_empty()) {
            if !self.ui.prefs.expanded_folders.iter().any(|p| p == ancestor) {
                self.ui.prefs.expanded_folders.push(ancestor.to_path_buf());
            }
        }
        self.rebuild_sorted_tree(cx);
        self.tree.update(cx, |tree, cx| {
            let id: SharedString = row.to_string_lossy().to_string().into();
            tree.reveal_item(&id, ScrollStrategy::Center, cx);
        });
        self.name.update(cx, |input, cx| {
            input.set_placeholder(
                if folder {
                    "未命名文件夹"
                } else {
                    "未命名"
                },
                window,
                cx,
            );
            input.set_value(value, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    pub(super) fn cancel_tree_name(&mut self, cx: &mut Context<Self>) {
        self.ui.name_mode = None;
        self.ui.folder_target = None;
        if self.ui.tree_name.take().is_some() {
            self.rebuild_sorted_tree(cx);
        }
        cx.notify();
    }

    pub(super) fn prepare_tree_name(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.name.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return false;
        }
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return false;
        }
        if name == "."
            || name == ".."
            || name.ends_with('.')
            || name.chars().any(char::is_control)
            || (cfg!(windows)
                && matches!(
                    name.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                ))
            || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        {
            self.notifications
                .publish("名称不能包含路径或文件名保留字符。".into());
            cx.notify();
            return false;
        }
        let edit = self.ui.tree_name.as_ref().unwrap();
        let filename = if !edit.folder && !name.to_lowercase().ends_with(".md") {
            format!("{name}.md")
        } else {
            name
        };
        let destination = edit.parent.join(filename);
        if !edit.creating && destination == edit.row {
            self.cancel_tree_name(cx);
            return false;
        }
        if self
            .vault
            .as_ref()
            .is_some_and(|vault| vault.root.join(&destination).exists())
            || self.tabs.iter().any(|tab| tab.path == destination)
        {
            self.notifications
                .publish("同名文件或文件夹已存在，请换一个名称。".into());
            cx.notify();
            return false;
        }
        if !edit.creating
            && self.tabs.iter().any(|tab| {
                tab.save.persistence.is_dirty()
                    || tab.save.persistence.is_saving()
                    || tab.save.persistence.has_conflict()
            })
        {
            self.notifications
                .publish("请先保存笔记并处理冲突，然后重新确认名称。".into());
            self.save_all(window, cx);
            return false;
        }
        let note_id = edit.note_id;
        if !edit.creating && !self.file_writes.can_start_exclusive_operation() {
            self.notifications
                .publish("正在保存文件，请稍后重新确认名称。".into());
            cx.notify();
            return false;
        }
        if !edit.creating {
            let ids: Vec<_> = self.tabs.iter().map(|tab| tab.id).collect();
            if ids
                .into_iter()
                .any(|id| self.has_pending_input(id, window, cx))
            {
                return false;
            }
        }
        let rename_note = self.ui.name_mode == Some(NameMode::Rename);
        self.ui.tree_name = None;
        self.ui.tree_active = Some(destination.clone());
        self.rebuild_sorted_tree(cx);
        self.name.update(cx, |input, cx| {
            input.set_value(destination.to_string_lossy().to_string(), window, cx)
        });
        if rename_note {
            self.ui.name_mode = None;
            if let Some(id) = note_id {
                self.manage_named_note(
                    id,
                    false,
                    destination.to_string_lossy().to_string(),
                    window,
                    cx,
                );
            }
            return false;
        }
        true
    }
}
