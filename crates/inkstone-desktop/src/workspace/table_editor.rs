use super::*;
use gpui_component::button::{Button, ButtonVariants};
use inkstone_core::tables::Grid;

pub(super) struct Review {
    tab: usize,
    original: String,
    grid: Grid,
    cells: Vec<Vec<Entity<InputState>>>,
    message: String,
}

impl Review {
    fn composing(&self, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
        self.cells
            .iter()
            .flatten()
            .any(|cell| cell.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()))
    }
    fn collect(&mut self, cx: &App) {
        self.grid.rows = self
            .cells
            .iter()
            .map(|row| row.iter().map(|s| s.read(cx).value().to_string()).collect())
            .collect();
    }
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Workspace>) {
        self.cells = self
            .grid
            .rows
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, text)| {
                        cx.new(|cx| {
                            let mut input = InputState::new(window, cx).placeholder(format!(
                                "第 {} 行，第 {} 列",
                                r + 1,
                                c + 1
                            ));
                            input.set_value(text.clone(), window, cx);
                            input
                        })
                    })
                    .collect()
            })
            .collect();
    }
}
impl Workspace {
    pub(super) fn open_table_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_document_views(window, cx);
        let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) else {
            return;
        };
        if self.has_pending_input(tab.id, window, cx) {
            return;
        }
        let Some(pane) = self.current_pane() else {
            return;
        };
        let editor = pane.read(cx).editor.clone();
        let original = editor.read(cx).value().to_string();
        let offset = editor.read(cx).selected_range().start;
        let Some(grid) = Grid::at(&original, offset) else {
            self.notifications.publish(
                "请先将光标放入独立 Markdown 表格；可视化编辑支持最多 500 个单元格。".into(),
            );
            return;
        };
        let mut review = Review {
            tab: tab.id,
            original,
            grid,
            cells: vec![],
            message: String::new(),
        };
        review.rebuild(window, cx);
        self.close_overlays(window, cx);
        self.ui.table_editor = Some(review);
        self.ui.trash_open = true;
        window.focus(&self.ui.modal_focus, cx);
        cx.notify();
    }
    fn resize_table_grid(&mut self, operation: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &mut self.ui.table_editor else {
            return;
        };
        if review.composing(window, cx) {
            return;
        }
        review.collect(cx);
        let columns = review.grid.alignment.len();
        let rows = review.grid.rows.len();
        match operation {
            0 if (rows + 1) * columns <= 500 => review.grid.rows.push(vec![String::new(); columns]),
            1 if rows > 1 => {
                review.grid.rows.pop();
            }
            2 if rows * (columns + 1) <= 500 => {
                for row in &mut review.grid.rows {
                    row.push(String::new());
                }
                review.grid.alignment.push(0);
            }
            3 if columns > 1 => {
                for row in &mut review.grid.rows {
                    row.pop();
                }
                review.grid.alignment.pop();
            }
            _ => {
                review.message = "至少保留一行一列，最多 500 个单元格。".into();
                cx.notify();
                return;
            }
        }
        review.rebuild(window, cx);
        review.message.clear();
        cx.notify();
    }
    fn apply_table_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_document_views(window, cx);
        let Some(review) = &mut self.ui.table_editor else {
            return;
        };
        if review.composing(window, cx) {
            return;
        }
        review.collect(cx);
        let id = review.tab;
        let original = review.original.clone();
        let edit = review.grid.edit(&original);
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        if tab.save.editor.read(cx).value().as_ref() != original
            || self.has_pending_input(id, window, cx)
        {
            self.ui.table_editor.as_mut().unwrap().message =
                "笔记已变化或仍在组词，请关闭面板并重新打开表格，避免覆盖新内容。".into();
            cx.notify();
            return;
        }
        let Some(edit) = edit else {
            self.ui.table_editor.as_mut().unwrap().message =
                "单元格不能包含换行；请使用行内内容。".into();
            cx.notify();
            return;
        };
        let editor = tab.save.editor.clone();
        self.close_overlays(window, cx);
        editor.update(cx, |s, cx| {
            s.apply_source_edit(edit.range, &edit.replacement, edit.selection, window, cx);
            s.focus(window, cx);
        });
        self.document_changed(editor, window, cx);
        self.notifications
            .publish("表格已更新，可撤销本次修改。".into());
        cx.notify();
    }
    pub(super) fn table_editor_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(review) = &self.ui.table_editor else {
            return div().into_any_element();
        };
        div().flex().flex_col().min_h_0().gap_3()
            .child("首行为表头；单元格支持行内 Markdown，竖线自动转义。应用后可一次撤销；关闭面板放弃本次表格修改。")
            .child(div().flex().flex_wrap().gap_2().children([
                (0, "添加末行"), (1, "删除末行"), (2, "添加末列"), (3, "删除末列")
            ].into_iter().map(|(op, title)| Button::new(("table-grid-size", op)).label(title)
                .on_click(cx.listener(move |this, _, w, cx| this.resize_table_grid(op, w, cx))))))
            .child(div().id("visual-table-grid").overflow_scroll().min_h_0().max_h(px(420.)).flex().flex_col().gap_2()
                .child(div().flex().gap_2().children(review.grid.alignment.iter().enumerate().map(|(c, align)| {
                    let label = ["默认对齐", "左对齐", "居中", "右对齐"][*align];
                    Button::new(("table-grid-alignment", c)).w(px(180.)).flex_shrink_0().label(format!("第 {} 列 · {label}", c + 1))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(r) = &mut this.ui.table_editor { r.grid.alignment[c] = (r.grid.alignment[c] + 1) % 4; }
                            cx.notify();
                        }))
                })))
                .children(review.cells.iter().map(|row| div().flex().flex_shrink_0().gap_2().children(row.iter().map(|cell| Input::new(cell).w(px(180.)).flex_shrink_0())))))
            .child(review.message.clone())
            .child(Button::new("apply-table-grid").primary().label("应用表格修改")
                .on_click(cx.listener(|this, _, w, cx| this.apply_table_grid(w, cx))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui::test]
    fn visual_table_apply_is_undoable_and_rejects_changed_source(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.add_window(Workspace::new);
        let text = "| A | B |\n| --- | --- |\n| 中文 | value |";
        handle
            .update(cx, |w, window, cx| {
                w.add_tab("table.md".into(), Some(text.into()), false, window, cx);
                let editor = w.tabs[0].save.editor.clone();
                w.open_table_editor(window, cx);
                w.ui.table_editor.as_ref().unwrap().cells[1][0]
                    .update(cx, |s, cx| s.set_value("new|值", window, cx));
                w.resize_table_grid(0, window, cx);
                w.apply_table_grid(window, cx);
                assert!(editor.read(cx).value().contains("new\\|值"));
                assert!(w.ui.table_editor.is_none());
                editor.update(cx, |s, cx| s.undo(&gpui_component::input::Undo, window, cx));
                assert_eq!(editor.read(cx).value().as_ref(), text);
                w.open_table_editor(window, cx);
                editor.update(cx, |s, cx| s.set_value("changed", window, cx));
                w.apply_table_grid(window, cx);
                assert_eq!(editor.read(cx).value().as_ref(), "changed");
                assert!(
                    w.ui.table_editor
                        .as_ref()
                        .unwrap()
                        .message
                        .contains("笔记已变化")
                );
            })
            .unwrap();
    }
}
