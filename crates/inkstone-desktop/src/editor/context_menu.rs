//! Article commands stay on the pane that owns the right-click selection.
use super::*;
use gpui_component::{Icon, input, native_menu::NativeMenu};
use inkstone_core::{
    markdown_edit::{self, BlockFormat},
    tables::{self, Operation},
};

#[derive(Action, Clone, Debug, PartialEq)]
#[action(namespace = article, no_json)]
pub(super) enum Command {
    Block(BlockFormat),
    Inline(&'static str),
    Link(bool),
    Table(Operation),
    Task,
    Footnote,
}

fn items(entries: &[(&'static str, Command)]) -> NativeMenu {
    entries
        .iter()
        .fold(NativeMenu::new(), |menu, (label, command)| {
            menu.menu(*label, Box::new(command.clone()))
        })
}

pub(super) fn build(state: &EditorState) -> NativeMenu {
    let capabilities = state.context_menu_capabilities();
    let editable = capabilities.is_editable();
    let copyable = capabilities.is_copyable();
    let mut menu = NativeMenu::new()
        .menu_with_disabled("撤销", !editable, Box::new(input::Undo))
        .menu_with_disabled("重做", !editable, Box::new(input::Redo))
        .separator()
        .menu_with_disabled("剪切", !(editable && copyable), Box::new(input::Cut))
        .menu_with_disabled("复制", !copyable, Box::new(input::Copy))
        .menu_with_disabled("粘贴", !editable, Box::new(input::Paste))
        .menu("全选", Box::new(input::SelectAll));
    if editable && !state.has_multiple_selections() {
        use Command::*;
        let headings = items(&[
            ("正文", Block(BlockFormat::Heading(0))),
            ("一级标题", Block(BlockFormat::Heading(1))),
            ("二级标题", Block(BlockFormat::Heading(2))),
            ("三级标题", Block(BlockFormat::Heading(3))),
            ("四级标题", Block(BlockFormat::Heading(4))),
            ("五级标题", Block(BlockFormat::Heading(5))),
            ("六级标题", Block(BlockFormat::Heading(6))),
        ]);
        menu = menu
            .separator()
            .submenu("标题与正文", headings)
            .submenu(
                "列表",
                items(&[
                    ("无序列表", Block(BlockFormat::Bullet)),
                    ("有序列表", Block(BlockFormat::Numbered)),
                    ("任务列表 / 切换完成状态", Task),
                ]),
            )
            .submenu(
                "文本格式",
                items(&[
                    ("粗体", Inline("**")),
                    ("斜体", Inline("*")),
                    ("删除线", Inline("~~")),
                    ("高亮", Inline("==")),
                    ("行内代码", Inline("`")),
                    ("行内公式", Inline("$")),
                ]),
            )
            .separator()
            .menu_with_icon(
                "插入表格",
                Icon::default().path("icons/table-2.svg"),
                Box::new(Table(Operation::Insert)),
            )
            .menu_with_icon(
                "代码块",
                Icon::default().path("icons/code.svg"),
                Box::new(Block(BlockFormat::Code)),
            )
            .menu_with_icon(
                "公式块",
                Icon::default().path("icons/sigma.svg"),
                Box::new(Block(BlockFormat::Math)),
            )
            .submenu(
                "更多插入",
                items(&[
                    ("引用", Block(BlockFormat::Quote)),
                    ("提示块", Block(BlockFormat::Callout)),
                    ("分隔线", Block(BlockFormat::Rule)),
                    ("网页链接", Link(false)),
                    ("内部链接", Link(true)),
                    ("脚注", Footnote),
                ]),
            );
        // Only show structural table tools when they can act on this location.
        if tables::Grid::at(&state.value(), state.selected_range().start).is_some() {
            menu = menu.submenu(
                "编辑表格",
                items(&[
                    ("在下方插入行", Table(Operation::AddRow)),
                    ("在右侧插入列", Table(Operation::AddColumn)),
                ])
                .separator()
                .submenu(
                    "当前列对齐",
                    items(&[
                        ("左对齐", Table(Operation::Left)),
                        ("居中", Table(Operation::Center)),
                        ("右对齐", Table(Operation::Right)),
                    ]),
                )
                .separator()
                .menu("删除当前行", Box::new(Table(Operation::RemoveRow)))
                .menu("删除当前列", Box::new(Table(Operation::RemoveColumn))),
            );
        }
    }
    menu.separator().submenu(
        "Agent 助手",
        NativeMenu::new()
            .menu("发送给 Agent", Box::new(crate::workspace::AgentAsk))
            .menu("润色", Box::new(crate::workspace::AgentPolish))
            .menu("摘要", Box::new(crate::workspace::AgentSummary)),
    )
}

impl EditorPane {
    pub(super) fn context_command(
        &mut self,
        command: &Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.reading || self.footnote_edit.is_some() {
            return;
        }
        let applied = self.editor.update(cx, |state, cx| {
            let text = state.value();
            let selection = state.selected_range();
            let edit = match command {
                Command::Block(format) => markdown_edit::block_format(&text, selection, *format),
                Command::Inline(marker) => {
                    markdown_edit::inline_format(&text, selection, marker, marker)
                }
                Command::Link(wiki) => markdown_edit::insert_link(&text, selection, *wiki),
                Command::Table(operation) => tables::edit(&text, selection, *operation),
                Command::Footnote => markdown_edit::insert_footnote(&text, selection),
                Command::Task => markdown::toggle_task_lines(&text, selection.clone()).map(
                    |(range, replacement)| {
                        let end = range.start + replacement.len();
                        let selected = if selection.is_empty() {
                            let at = selection
                                .start
                                .saturating_add_signed(
                                    replacement.len() as isize - range.len() as isize,
                                )
                                .min(end);
                            at..at
                        } else {
                            range.start..end
                        };
                        markdown_edit::Edit {
                            range,
                            replacement,
                            selection: selected,
                        }
                    },
                ),
            };
            let applied = edit.is_some_and(|edit| {
                state.apply_source_edit(edit.range, &edit.replacement, edit.selection, window, cx)
            });
            state.focus(window, cx);
            if applied && matches!(command, Command::Link(true)) {
                state.request_completions(window, cx);
            }
            applied
        });
        if applied && matches!(command, Command::Footnote) {
            self.open_new_footnote(window, cx);
        }
        cx.notify();
    }
}
