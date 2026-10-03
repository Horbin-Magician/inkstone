//! Source-preserving cell contents with atomic Markdown table operations.
use crate::markdown_edit::Edit;
use markdown_parser::mdast::{AlignKind, Node};
use std::ops::Range;

#[derive(Clone, Copy)]
pub enum Operation {
    Insert,
    AddRow,
    RemoveRow,
    AddColumn,
    RemoveColumn,
    Left,
    Center,
    Right,
}
pub enum Navigation {
    Select(Range<usize>),
    Edit(Edit),
}
/// Small table snapshot for the visual editor. Cell strings retain inline Markdown.
#[derive(Clone)]
pub struct Grid {
    pub range: Range<usize>,
    pub rows: Vec<Vec<String>>,
    pub alignment: Vec<usize>,
}
impl Grid {
    pub fn at(text: &str, offset: usize) -> Option<Self> {
        let (table, _, _) = Table::at(text, offset)?;
        if table.rows.len() * table.alignment.len() > 500 {
            return None;
        }
        Some(Self {
            range: table.range,
            rows: table
                .rows
                .into_iter()
                .map(|r| r.into_iter().map(|c| c.text).collect())
                .collect(),
            alignment: table
                .alignment
                .into_iter()
                .map(|a| match a {
                    ":---" => 1,
                    ":---:" => 2,
                    "---:" => 3,
                    _ => 0,
                })
                .collect(),
        })
    }
    pub fn edit(&self, original: &str) -> Option<Edit> {
        let columns = self.alignment.len();
        if columns == 0
            || self.rows.is_empty()
            || self.rows.len().checked_mul(columns)? > 500
            || self.range.start > self.range.end
            || !original.is_char_boundary(self.range.start)
            || !original.is_char_boundary(self.range.end)
            || self
                .rows
                .iter()
                .any(|r| r.len() != columns || r.iter().any(|s| s.contains(['\r', '\n'])))
        {
            return None;
        }
        let rows = self
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| {
                        let mut text = String::new();
                        let mut escaped = false;
                        for ch in value.chars() {
                            if ch == '|' && !escaped {
                                text.push('\\');
                            }
                            text.push(ch);
                            escaped = ch == '\\' && !escaped;
                        }
                        Cell {
                            text,
                            range: 0..0,
                            present: true,
                        }
                    })
                    .collect()
            })
            .collect();
        let table = Table {
            range: self.range.clone(),
            rows,
            row_ranges: vec![],
            alignment: self
                .alignment
                .iter()
                .map(|a| match a {
                    1 => ":---",
                    2 => ":---:",
                    3 => "---:",
                    _ => "---",
                })
                .collect(),
        };
        Some(table.finish(original, 0, 0))
    }
}
struct Cell {
    text: String,
    range: Range<usize>,
    present: bool,
}
struct Table {
    range: Range<usize>,
    rows: Vec<Vec<Cell>>,
    row_ranges: Vec<Range<usize>>,
    alignment: Vec<&'static str>,
}
impl Table {
    fn at(text: &str, offset: usize) -> Option<(Self, usize, usize)> {
        let mut options = markdown_parser::ParseOptions::gfm();
        options.constructs.frontmatter = true;
        let root = markdown_parser::to_mdast(text, &options).ok()?;
        for node in root.children()? {
            let Node::Table(table) = node else {
                continue;
            };
            let p = table.position.as_ref()?;
            if offset < p.start.offset || offset > p.end.offset {
                continue;
            }
            let mut rows = vec![];
            let mut row_ranges = vec![];
            for row in &table.children {
                let rp = row.position()?;
                row_ranges.push(rp.start.offset..rp.end.offset);
                let mut cells = vec![];
                for cell in row.children()? {
                    let p = cell.position()?;
                    let children = cell.children()?;
                    let range =
                        if let (Some(first), Some(last)) = (children.first(), children.last()) {
                            first.position()?.start.offset..last.position()?.end.offset
                        } else {
                            p.start.offset..p.start.offset
                        };
                    cells.push(Cell {
                        present: true,
                        text: text[range.clone()].into(),
                        range,
                    });
                }
                rows.push(cells);
            }
            let cols = rows.iter().map(Vec::len).max()?.max(table.align.len());
            // A ragged table can have many columns but very short rows. Do not
            // expand a small source into an unbounded rectangular allocation.
            if cols == 0
                || rows
                    .len()
                    .checked_mul(cols)
                    .is_none_or(|cells| cells > 100_000)
            {
                return None;
            }
            for (row, range) in rows.iter_mut().zip(&row_ranges) {
                while row.len() < cols {
                    row.push(Cell {
                        present: false,
                        text: String::new(),
                        range: range.end..range.end,
                    });
                }
            }
            let mut alignment: Vec<_> = table
                .align
                .iter()
                .map(|a| match a {
                    AlignKind::Left => ":---",
                    AlignKind::Right => "---:",
                    AlignKind::Center => ":---:",
                    _ => "---",
                })
                .collect();
            alignment.resize(cols, "---");
            let row = row_ranges
                .iter()
                .position(|r| r.start <= offset && offset <= r.end)?;
            let column = rows[row]
                .iter()
                .rposition(|c| c.present && c.range.start <= offset)
                .unwrap_or(0);
            return Some((
                Self {
                    range: p.start.offset..p.end.offset,
                    rows,
                    row_ranges,
                    alignment,
                },
                row,
                column,
            ));
        }
        None
    }
    fn encode(&self, newline: &str) -> (String, Vec<Vec<Range<usize>>>) {
        let mut out = String::new();
        let mut ranges = vec![];
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                out.push_str(newline);
            }
            out.push_str("| ");
            let mut cells = vec![];
            for (c, cell) in row.iter().enumerate() {
                if c > 0 {
                    out.push_str(" | ");
                }
                let start = out.len();
                out.push_str(&cell.text);
                cells.push(start..out.len());
            }
            out.push_str(" |");
            ranges.push(cells);
            if i == 0 {
                out.push_str(newline);
                out.push_str("| ");
                out.push_str(&self.alignment.join(" | "));
                out.push_str(" |");
            }
        }
        (out, ranges)
    }
    fn finish(&self, text: &str, row: usize, col: usize) -> Edit {
        let newline = if text[self.range.clone()].contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let (replacement, ranges) = self.encode(newline);
        let target = &ranges[row.min(ranges.len() - 1)][col.min(self.alignment.len() - 1)];
        Edit {
            range: self.range.clone(),
            replacement,
            selection: self.range.start + target.start..self.range.start + target.end,
        }
    }
    fn blank_row(&self) -> Vec<Cell> {
        (0..self.alignment.len())
            .map(|_| Cell {
                present: true,
                text: String::new(),
                range: 0..0,
            })
            .collect()
    }
}

pub fn edit(text: &str, selection: Range<usize>, operation: Operation) -> Option<Edit> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    if matches!(operation, Operation::Insert) {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let before = &text[..selection.start];
        let after = &text[selection.end..];
        let prefix = if before.is_empty() || before.ends_with(&format!("{newline}{newline}")) {
            String::new()
        } else if before.ends_with('\n') {
            newline.into()
        } else {
            format!("{newline}{newline}")
        };
        let suffix = if after.is_empty() || after.starts_with(&format!("{newline}{newline}")) {
            String::new()
        } else if after.starts_with(['\r', '\n']) {
            newline.into()
        } else {
            format!("{newline}{newline}")
        };
        let mut escaped = String::new();
        let mut slashes = 0;
        for ch in text[selection.clone()].chars() {
            if ch == '|' && slashes % 2 == 0 {
                escaped.push('\\');
            }
            escaped.push(ch);
            slashes = if ch == '\\' { slashes + 1 } else { 0 };
        }
        let body = escaped.replace("\r\n", "<br>").replace('\n', "<br>");
        let replacement =
            format!("{prefix}|  |  |{newline}| --- | --- |{newline}| {body} |  |{suffix}");
        let at = selection.start + prefix.len() + 2;
        return Some(Edit {
            range: selection,
            replacement,
            selection: at..at,
        });
    }
    let (mut table, row, col) = Table::at(text, selection.start)?;
    let mut target = (row, col);
    match operation {
        Operation::AddRow => {
            table.rows.insert(row + 1, table.blank_row());
            target.0 = row + 1;
        }
        Operation::RemoveRow => {
            table.rows.remove(row);
            target.0 = row.min(table.rows.len().saturating_sub(1));
        }
        Operation::AddColumn => {
            table.alignment.insert(col + 1, "---");
            for row in &mut table.rows {
                row.insert(
                    col + 1,
                    Cell {
                        present: true,
                        text: String::new(),
                        range: 0..0,
                    },
                );
            }
            target.1 = col + 1;
        }
        Operation::RemoveColumn => {
            table.alignment.remove(col);
            for row in &mut table.rows {
                row.remove(col);
            }
            target.1 = col.min(table.alignment.len().saturating_sub(1));
        }
        Operation::Left => table.alignment[col] = ":---",
        Operation::Center => table.alignment[col] = ":---:",
        Operation::Right => table.alignment[col] = "---:",
        Operation::Insert => unreachable!(),
    }
    if table.rows.is_empty() || table.alignment.is_empty() {
        return Some(Edit {
            selection: table.range.start..table.range.start,
            range: table.range,
            replacement: String::new(),
        });
    }
    Some(table.finish(text, target.0, target.1))
}

pub fn navigate(text: &str, selection: Range<usize>, backwards: bool) -> Option<Navigation> {
    if selection.start > selection.end
        || !text.is_char_boundary(selection.start)
        || !text.is_char_boundary(selection.end)
    {
        return None;
    }
    let (mut table, row, col) = Table::at(text, selection.start)?;
    if selection.end > table.row_ranges[row].end {
        return None;
    }
    let cols = table.alignment.len();
    let index = row * cols + col;
    if backwards && index == 0 {
        return Some(Navigation::Select(table.rows[0][0].range.clone()));
    }
    let next = if backwards { index - 1 } else { index + 1 };
    if next >= table.rows.len() * cols {
        table.rows.push(table.blank_row());
        return Some(Navigation::Edit(table.finish(
            text,
            table.rows.len() - 1,
            0,
        )));
    }
    let target = &table.rows[next / cols][next % cols];
    if !target.present {
        return Some(Navigation::Edit(table.finish(
            text,
            next / cols,
            next % cols,
        )));
    }
    Some(Navigation::Select(target.range.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_grid_preserves_markdown_crlf_and_escapes_pipes() {
        let text = "before\r\n\r\n| 姓名 | 说明 |\r\n| --- | :---: |\r\n| 中文 | **bold** a\\|b |\r\n\r\nafter";
        let mut grid = Grid::at(text, text.find("中文").unwrap()).unwrap();
        grid.rows[1][0] = "👩‍💻 a|b".into();
        grid.alignment[0] = 3;
        let edit = grid.edit(text).unwrap();
        let result = format!(
            "{}{}{}",
            &text[..edit.range.start],
            edit.replacement,
            &text[edit.range.end..]
        );
        assert!(result.contains("👩‍💻 a\\|b | **bold** a\\|b"));
        assert!(result.contains("| ---: | :---: |\r\n"));
        assert!(result.starts_with("before\r\n\r\n"));
        assert!(result.ends_with("\r\n\r\nafter"));
        assert_eq!(
            Grid::at(&result, result.find("👩").unwrap())
                .unwrap()
                .rows
                .len(),
            2
        );
        grid.rows[0][0] = "a\nb".into();
        assert!(grid.edit(text).is_none());
    }
    #[test]
    fn oversized_ragged_table_does_not_expand_into_a_huge_grid() {
        let header = format!(
            "|{}|\n|{}|\n",
            vec![" h "; 600].join("|"),
            vec![" --- "; 600].join("|")
        );
        let text = format!("{header}{}", "| x |\n".repeat(200));
        assert!(edit(&text, 2..2, Operation::AddColumn).is_none());
    }
    #[test]
    fn table_operations_keep_markdown_cells_and_line_endings() {
        let text = "before\r\n\r\n| 姓名 | 说明 |\r\n| --- | :---: |\r\n| 中文 | **bold** a\\|b |\r\n\r\nafter";
        let cursor = text.find("中文").unwrap();
        let e = edit(text, cursor..cursor, Operation::AddColumn).unwrap();
        let mut out = text.to_string();
        out.replace_range(e.range, &e.replacement);
        assert!(out.contains("| 中文 |  | **bold** a\\|b |"));
        assert!(out.starts_with("before\r\n\r\n"));
        assert!(out.ends_with("\r\n\r\nafter"));
        let e = edit(text, cursor..cursor, Operation::Center).unwrap();
        assert!(e.replacement.contains("| :---: | :---: |"));
        let e = edit(text, cursor..cursor, Operation::RemoveRow).unwrap();
        assert!(!e.replacement.contains("中文"));
        assert!(e.replacement.contains("姓名"));
    }
    #[test]
    fn table_tab_moves_cells_and_appends_one_row() {
        let text = "| a | b |\n| --- | --- |\n| c | d |";
        let start = text.find('a').unwrap();
        let Some(Navigation::Select(next)) = navigate(text, start..start, false) else {
            panic!()
        };
        assert_eq!(&text[next], "b");
        let end = text.rfind('d').unwrap();
        let Some(Navigation::Edit(e)) = navigate(text, end..end, false) else {
            panic!()
        };
        assert!(e.replacement.ends_with("\n|  |  |"));
        assert_eq!(
            &e.replacement[e.selection.start - e.range.start..e.selection.end - e.range.start],
            ""
        );
        assert!(
            edit(
                "```\n| a | b |\n| --- | --- |\n```",
                8..8,
                Operation::AddRow
            )
            .is_none()
        );
        let edit = edit("before\nafter", 7..7, Operation::Insert).unwrap();
        let mut inserted = "before\nafter".to_string();
        inserted.replace_range(edit.range, &edit.replacement);
        assert!(Table::at(&inserted, edit.selection.start).is_some());
        let ragged = "| a | b |\n| --- | --- |\n| c";
        let at = ragged.len();
        let Some(Navigation::Edit(edit)) = navigate(ragged, at..at, false) else {
            panic!()
        };
        assert!(edit.replacement.ends_with("| c |  |"));
    }
}
