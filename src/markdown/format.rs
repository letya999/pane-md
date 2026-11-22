use crate::{Bytes, newline};

use super::{Layout, Markdown, Region, Style, TableEdit, table_patch};

#[derive(Clone, Copy)]
pub enum InlineFormat {
    Bold,
    Italic,
    Strike,
    Code,
}

impl InlineFormat {
    fn enabled(self, style: Style) -> bool {
        match self {
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Strike => style.strike,
            Self::Code => style.code,
        }
    }
}

#[derive(Clone, Copy)]
pub enum BlockFormat {
    Heading(u8),
    Bullet,
    Numbered,
    Task,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    SelectText,
    Unsupported,
    AroundSpaces,
    ChooseParagraph,
    ChooseCell,
    TableSource,
    LinkSource,
    LinkOneLine,
    InvalidLink,
}

pub struct FormatPatch {
    pub range: Bytes,
    pub text: String,
    pub caret: Bytes,
    pub focus_region: Option<usize>,
}

impl FormatPatch {
    pub fn new(range: Bytes, text: impl Into<String>, caret: usize) -> Self {
        Self {
            range,
            text: text.into(),
            caret: caret..caret,
            focus_region: None,
        }
    }
}

impl Region {
    pub fn inline_format(
        &self,
        source: &str,
        selection: Bytes,
        format: InlineFormat,
    ) -> Result<FormatPatch, FormatError> {
        use FormatError::*;
        let span = self.source_selection(selection.clone()).ok_or(SelectText)?;
        let (range, _) = self.patch(source, selection, "").ok_or(Unsupported)?;
        if span != range || matches!(self.kind, "code" | "html") {
            return Err(Unsupported);
        }
        let raw = source.get(span.clone()).ok_or(Unsupported)?;
        if raw.trim() != raw {
            return Err(AroundSpaces);
        }
        let (primary, alternative) = match format {
            InlineFormat::Bold => ("**", "__"),
            InlineFormat::Italic => ("*", "_"),
            InlineFormat::Strike => ("~~", "~~"),
            InlineFormat::Code => ("`", "`"),
        };
        let run = self
            .runs
            .iter()
            .find(|run| {
                run.units
                    .iter()
                    .flatten()
                    .any(|unit| unit.start == span.start)
            })
            .ok_or(Unsupported)?;
        let styled = format.enabled(run.style);
        let check = |patch: FormatPatch| {
            let mut candidate = source.to_owned();
            candidate.replace_range(patch.range.clone(), &patch.text);
            let parsed = Markdown::parse(&candidate);
            let safe = parsed.regions.iter().any(|region| {
                if region.source.start > patch.caret.start
                    || region.source.end < patch.caret.end
                    || region.text != self.text
                {
                    return false;
                }
                let mut affected = region.runs.iter().filter(|run| {
                    run.units
                        .iter()
                        .flatten()
                        .any(|unit| patch.caret.start < unit.end && unit.start < patch.caret.end)
                });
                affected
                    .next()
                    .is_some_and(|run| format.enabled(run.style) != styled)
                    && affected.all(|run| format.enabled(run.style) != styled)
            });
            safe.then_some(patch).ok_or(Unsupported)
        };
        if styled {
            let delimiter = [primary, alternative]
                .into_iter()
                .find(|marker| {
                    source[..span.start].ends_with(marker) && source[span.end..].starts_with(marker)
                })
                .ok_or(Unsupported)?;
            let start = span.start - delimiter.len();
            let end = span.end + delimiter.len();
            let mut patch = FormatPatch::new(start..end, raw, start);
            patch.caret.end += raw.len();
            return check(patch);
        }
        if run.style.code || matches!(format, InlineFormat::Code) && raw.contains('`') {
            return Err(Unsupported);
        }
        // Underscores allow the selection to be wrapped next to an existing star delimiter.
        let marker = if matches!(format, InlineFormat::Bold | InlineFormat::Italic)
            && (source[..span.start].ends_with('*') || source[span.end..].starts_with('*'))
        {
            alternative
        } else {
            primary
        };
        let start = span.start + marker.len();
        let mut patch = FormatPatch::new(span, format!("{marker}{raw}{marker}"), start);
        patch.caret.end += raw.len();
        check(patch)
    }

    pub fn block_format(
        &self,
        source: &str,
        format: BlockFormat,
    ) -> Result<FormatPatch, FormatError> {
        use FormatError::ChooseParagraph;
        if let Some(item) = &self.list {
            let (range, text) = match format {
                BlockFormat::Task => item.create_task_patch(source).ok_or(ChooseParagraph)?,
                BlockFormat::Bullet | BlockFormat::Numbered => {
                    let ordered = item.marker.len() > 1;
                    if ordered == matches!(format, BlockFormat::Numbered) {
                        return Err(ChooseParagraph);
                    }
                    (item.marker.clone(), if ordered { "-" } else { "1." })
                }
                BlockFormat::Heading(_) => return Err(ChooseParagraph),
            };
            let caret = self.source.start.max(range.end) - range.len() + text.len();
            return Ok(FormatPatch::new(range, text, caret));
        }
        if self.indent != 0 || self.quote || !matches!(self.kind, "paragraph" | "heading") {
            return Err(ChooseParagraph);
        }
        let raw = source.get(self.source.clone()).ok_or(ChooseParagraph)?;
        let content = if self.level > 0 {
            heading_content(raw).ok_or(ChooseParagraph)?
        } else if raw.contains(['\n', '\r']) {
            return Err(ChooseParagraph);
        } else {
            raw
        };
        let marker = match format {
            BlockFormat::Heading(0) => String::new(),
            BlockFormat::Heading(level @ 1..=6) if self.level != level => {
                format!("{} ", "#".repeat(usize::from(level)))
            }
            BlockFormat::Heading(1..=6) => String::new(),
            BlockFormat::Heading(_) => return Err(ChooseParagraph),
            BlockFormat::Bullet => "- ".into(),
            BlockFormat::Numbered => "1. ".into(),
            BlockFormat::Task => "- [ ] ".into(),
        };
        let caret = self.source.start + marker.len();
        Ok(FormatPatch::new(
            self.source.clone(),
            marker + content,
            caret,
        ))
    }
}

fn heading_content(raw: &str) -> Option<&str> {
    let raw = raw.trim_start_matches([' ', '\t']);
    if raw.starts_with('#') {
        let body = raw.trim_start_matches('#').trim();
        let after_hashes = body.trim_end_matches('#');
        return Some(if after_hashes.ends_with([' ', '\t']) {
            after_hashes.trim_end()
        } else {
            body
        });
    }
    let (line, underline) = raw.split_once(['\r', '\n'])?;
    let underline = underline.trim();
    ((!underline.is_empty() && underline.bytes().all(|b| b == b'=' || b == b'-'))
        && !underline.contains(['\r', '\n']))
    .then_some(line)
}

impl Markdown {
    pub fn edit_table(
        &self,
        source: &str,
        index: usize,
        edit: TableEdit,
    ) -> Result<FormatPatch, FormatError> {
        let region = self.regions.get(index).ok_or(FormatError::ChooseCell)?;
        let (row, cell) = region.row.zip(region.cell).ok_or(FormatError::ChooseCell)?;
        let Some(Layout::Table {
            rows,
            alignment,
            source: bytes,
        }) = self.table_at(index)
        else {
            return Err(FormatError::ChooseCell);
        };
        let text = table_patch(
            source,
            bytes.clone(),
            rows.len(),
            alignment.len(),
            row,
            cell,
            edit,
        )
        .ok_or(FormatError::TableSource)?;
        let (height, width) = (rows.len(), alignment.len());
        let (height, width, target_row, target_column) = match edit {
            TableEdit::AddRow => (height + 1, width, row + 1, 0),
            TableEdit::DeleteRow => (height - 1, width, row.min(height - 2), cell),
            TableEdit::AddColumn => (height, width + 1, row, cell + 1),
            TableEdit::DeleteColumn => (height, width - 1, row, cell.min(width - 2)),
        };
        let mut candidate = source.to_owned();
        candidate.replace_range(bytes.clone(), &text);
        let first_line_end = bytes.start + text.find(['\r', '\n']).unwrap_or(text.len());
        let parsed = Markdown::parse(&candidate);
        let focus = parsed
            .layout
            .iter()
            .find_map(|block| match block {
                Layout::Table {
                    rows,
                    alignment,
                    source,
                } if (bytes.start..first_line_end).contains(&source.start)
                    && rows.len() == height
                    && alignment.len() == width =>
                {
                    rows.get(target_row)?.get(target_column).copied()
                }
                _ => None,
            })
            .ok_or(FormatError::TableSource)?;
        let mut patch = FormatPatch::new(bytes.clone(), text, parsed.regions[focus].source.start);
        patch.focus_region = Some(focus);
        Ok(patch)
    }

    pub fn insert_table(&self, source: &str, index: usize) -> Option<FormatPatch> {
        let region = self.regions.get(index)?;
        let owner = self.owners.get(region.owner)?;
        let at = owner.source.end;
        let ending = newline(source);
        let prefix = if at == 0 || region.source.is_empty() {
            String::new()
        } else {
            ending.repeat(2)
        };
        let suffix = if at == source.len() {
            String::new()
        } else {
            ending.to_owned()
        };
        let caret = at + prefix.len() + 2;
        let table = ["|   |   |", "| --- | --- |", "|   |   |"].join(ending);
        Some(FormatPatch::new(at..at, prefix + &table + &suffix, caret))
    }
}
