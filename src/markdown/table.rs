use crate::{Bytes, markdown::split_row, newline};

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
    let line = if row == 0 { 0 } else { row + 1 };
    // shortcut: complex or ragged table structures use source editing until missing cells can be materialized safely.
    match edit {
        TableEdit::AddRow => {
            let raw_line = &lines[line].0;
            let indent = &raw_line[..raw_line.len() - raw_line.trim_start().len()];
            let mut new = format!("{indent}|");
            for _ in 0..columns {
                new.push_str("   |");
            }
            let at = line.max(1);
            let eol = lines[at].1;
            lines[at].1 = if eol.is_empty() { ending } else { eol };
            lines.insert(at + 1, (new, eol));
        }
        TableEdit::DeleteRow => {
            if row == 0 {
                if rows < 2 {
                    return None;
                }
                lines[0].0 = lines.remove(2).0;
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
                let cells = split_row(raw, 0..raw.len());
                if cells.len() != columns {
                    return None;
                }
                let target = &cells[cell];
                if matches!(edit, TableEdit::AddColumn) {
                    let content = if i == 1 { " --- " } else { "   " };
                    let trailing = if target.end == raw.trim_end().len() {
                        "|"
                    } else {
                        ""
                    };
                    raw.insert_str(target.end, &format!("|{content}{trailing}"));
                } else {
                    let remove = if raw.as_bytes().get(target.end) == Some(&b'|') {
                        target.start..target.end + 1
                    } else {
                        target.start.checked_sub(1)?..target.end
                    };
                    raw.replace_range(remove, "");
                    if columns == 2 && !raw.trim().starts_with('|') && !raw.trim().ends_with('|') {
                        raw.push('|');
                    }
                }
            }
        }
    }
    Some(lines.into_iter().map(|(body, eol)| body + eol).collect())
}
