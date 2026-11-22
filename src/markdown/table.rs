use crate::{Bytes, newline};

use super::{Layout, Markdown, Region, split_row};

pub(super) fn row_prefix_len(line: &str, quote_depth: usize) -> usize {
    let mut body = line.trim_start_matches([' ', '\t']);
    for _ in 0..quote_depth {
        let Some(tail) = body.strip_prefix('>') else {
            break;
        };
        body = tail.trim_start_matches([' ', '\t']);
    }
    line.len() - body.len()
}

impl Region {
    pub(super) fn materialize_cell(&self, source: &str, raw: &str) -> (String, usize) {
        if self.missing_cells == 0 || raw.is_empty() {
            return (raw.to_owned(), raw.len());
        }
        // Reuse a closing pipe, when present, after inserting the missing cells.
        let closes = source.as_bytes().get(self.source.start) == Some(&b'|');
        let prefix = if closes { "|" } else { " |" };
        let mut text = format!("{prefix}{} {raw}", "   |".repeat(self.missing_cells - 1));
        let caret = text.len();
        if closes {
            text.push(' ');
        }
        (text, caret)
    }
}

impl Markdown {
    pub(super) fn table_at(&self, index: usize) -> Option<&Layout> {
        let region = self.regions.get(index)?;
        let (row, cell) = region.row.zip(region.cell)?;
        self.layout.iter().find(|layout| {
            matches!(layout, Layout::Table { rows, .. }
                if rows.get(row).and_then(|cells| cells.get(cell)) == Some(&index))
        })
    }

    /// Walks cells within a table, including cells that are implicit in the source.
    pub fn table_neighbor(&self, index: usize, backwards: bool) -> Option<usize> {
        let Layout::Table { rows, .. } = self.table_at(index)? else {
            return None;
        };
        let region = &self.regions[index];
        let (row, col) = region.row.zip(region.cell)?;
        if backwards {
            col.checked_sub(1)
                .and_then(|col| rows[row].get(col))
                .or_else(|| rows.get(row.checked_sub(1)?)?.last())
        } else {
            rows[row]
                .get(col + 1)
                .or_else(|| rows.get(row + 1)?.first())
        }
        .copied()
    }

    /// Combines edits to empty cells that share a source insertion point.
    pub fn fill_empty_cells(
        &self,
        source: &str,
        edits: &[(usize, &str)],
    ) -> Option<(Bytes, String)> {
        let (first, _) = edits.first()?;
        let first = self.regions.get(*first)?;
        let at = first.source.start;
        let closes = source.as_bytes().get(at) == Some(&b'|');
        let mut replacement = String::new();
        let mut previous = None;
        for &(index, text) in edits {
            let region = self.regions.get(index)?;
            let missing = region.missing_cells;
            if region.cell.is_none()
                || region.row != first.row
                || region.owner != first.owner
                || region.source != (at..at)
                || !region.text.is_empty()
                || previous.is_some_and(|last| last >= missing)
                || text.chars().any(|ch| ch.is_control() && ch != '\t')
            {
                return None;
            }
            if missing != 0 {
                if previous.unwrap_or(0) > 0 || !closes {
                    replacement.push(' ');
                }
                for _ in previous.unwrap_or(0) + 1..missing {
                    replacement.push_str("|   ");
                }
                replacement.push_str("| ");
            }
            replacement.push_str(&super::paste::literal_line(text));
            previous = Some(missing);
        }
        if previous? == 0 {
            return None;
        }
        if closes {
            replacement.push(' ');
        }
        Some((at..at, replacement))
    }
}

#[derive(Clone, Copy)]
pub enum TableEdit {
    AddRow,
    DeleteRow,
    AddColumn,
    DeleteColumn,
}

pub fn table_patch(
    source: &str,
    span: Bytes,
    rows: usize,
    columns: usize,
    row: usize,
    cell: usize,
    edit: TableEdit,
) -> Option<String> {
    let raw = source.get(span)?;
    let ending = newline(if raw.contains(['\r', '\n']) {
        raw
    } else {
        source
    });
    let mut lines = Vec::new();
    let mut rest = raw;
    loop {
        if let Some(index) = rest.find(['\r', '\n']) {
            let (body, tail) = rest.split_at(index);
            let count = if tail.starts_with("\r\n") { 2 } else { 1 };
            lines.push((body.to_owned(), &tail[..count]));
            rest = &tail[count..];
        } else {
            lines.push((rest.to_owned(), ""));
            break;
        }
    }
    if lines.len() != rows + 1 || row >= rows || cell >= columns {
        return None;
    }
    let header = &lines[0].0;
    let divider = &lines[1].0;
    let divider_prefix = row_prefix_len(divider, usize::MAX);
    let quote_depth = divider[..divider_prefix]
        .bytes()
        .filter(|&b| b == b'>')
        .count();
    let alignment = split_row(divider, divider_prefix..divider.len());
    if split_row(header, 0..header.len()).len() != columns
        || alignment.len() != columns
        || !alignment.iter().all(|cell| {
            let marks = divider[cell.clone()].trim().trim_matches(':');
            !marks.is_empty() && marks.bytes().all(|byte| byte == b'-')
        })
    {
        return None;
    }
    let line = if row == 0 { 0 } else { row + 1 };
    match edit {
        TableEdit::AddRow => {
            let at = line.max(1);
            let raw_line = &lines[at].0;
            let prefix = &raw_line[..row_prefix_len(raw_line, quote_depth)];
            let mut new = format!("{prefix}|");
            for _ in 0..columns {
                new.push_str("   |");
            }
            let eol = lines[at].1;
            lines[at].1 = if eol.is_empty() { ending } else { eol };
            lines.insert(at + 1, (new, eol));
        }
        TableEdit::DeleteRow => {
            if row == 0 {
                if rows < 2 {
                    return None;
                }
                let body = lines.remove(2).0;
                let mut header = body[row_prefix_len(&body, quote_depth)..].to_owned();
                let cells = split_row(&header, 0..header.len());
                let missing = columns.checked_sub(cells.len())?;
                if missing != 0 {
                    let at = header.trim_end().len();
                    let prefix = if cells.last()?.end == at { " |" } else { "" };
                    header.insert_str(at, &format!("{prefix}{}", "   |".repeat(missing)));
                }
                lines[0].0 = header;
            } else if line + 1 == lines.len() {
                lines.pop();
                lines.last_mut()?.1 = "";
            } else {
                lines.remove(line);
            }
        }
        TableEdit::AddColumn | TableEdit::DeleteColumn => {
            if columns <= 1 && matches!(edit, TableEdit::DeleteColumn) {
                return None;
            }
            for (i, (raw, _)) in lines.iter_mut().enumerate() {
                let start = row_prefix_len(raw, if i == 0 { 0 } else { quote_depth });
                let cells = split_row(raw, start..raw.len());
                let Some(target) = cells.get(cell) else {
                    continue;
                };
                if matches!(edit, TableEdit::AddColumn) {
                    let content = if i == 1 { " --- " } else { "   " };
                    let trailing = if target.end == raw.trim_end().len() {
                        "|"
                    } else {
                        ""
                    };
                    raw.insert_str(target.end, &format!("|{content}{trailing}"));
                } else {
                    if cells.len() == 1 {
                        // The other columns are implicit. Keep the row itself in the table.
                        raw.replace_range(start..raw.trim_end().len(), "|   |");
                        continue;
                    }
                    let remove = if raw.as_bytes().get(target.end) == Some(&b'|') {
                        target.start..target.end + 1
                    } else {
                        target.start.checked_sub(1)?..target.end
                    };
                    raw.replace_range(remove, "");
                    let content = raw[start..].trim();
                    if columns == 2 && !content.starts_with('|') && !content.ends_with('|') {
                        raw.insert(raw.trim_end().len(), '|');
                    }
                }
            }
        }
    }
    Some(lines.into_iter().map(|(body, eol)| body + eol).collect())
}
