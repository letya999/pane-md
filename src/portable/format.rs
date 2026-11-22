use std::ops::Range;

use pane_md::{
    Bytes,
    markdown::{BlockFormat, FormatPatch, InlineFormat, Markdown, TableEdit},
    newline,
};
use serde::{Deserialize, Serialize};

use super::reviews::Part;
use crate::{
    Result,
    document::Document,
    i18n::{Label as L, format_error, tr},
};

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Command {
    Bold,
    Italic,
    Strike,
    Code,
    Heading { level: u8 },
    Bullet,
    Numbered,
    Task,
    ToggleTask,
    InsertTable,
    InsertRow,
    DeleteRow,
    InsertColumn,
    DeleteColumn,
    Link { target: String },
    Break { soft: bool },
}

pub fn projection(doc: &Document) -> Markdown {
    let mut parsed = Markdown::parse(&doc.text);
    if doc.mode == 2 {
        let lf = newline(&doc.text);
        let blank = parsed.regions.is_empty() && doc.text.trim().is_empty();
        let trailing = doc
            .text
            .strip_suffix(lf)
            .is_some_and(|text| text.ends_with(['\r', '\n']));
        if blank || trailing {
            parsed.push_empty_paragraph(doc.text.len());
        }
    }
    parsed
}

pub fn apply(doc: &mut Document, command: Command, part: Part) -> Result<CaretTarget> {
    if doc.mode != 2 {
        return Err(tr(L::UnsupportedEdit).into());
    }
    let parsed = projection(doc);
    let (index, local) = part.checked(&parsed).ok_or(tr(L::ChooseTextBlock))?;
    let region = &parsed.regions[index];
    if region.mermaid {
        return Err(tr(L::MermaidSourceHint).into());
    }
    let source = &doc.text;
    let patch = match command {
        Command::Bold | Command::Italic | Command::Strike | Command::Code => {
            let style = match command {
                Command::Bold => InlineFormat::Bold,
                Command::Italic => InlineFormat::Italic,
                Command::Strike => InlineFormat::Strike,
                _ => InlineFormat::Code,
            };
            region
                .inline_format(source, local, style)
                .map_err(format_error)?
        }
        Command::Heading { .. } | Command::Bullet | Command::Numbered | Command::Task => {
            let block = match command {
                Command::Heading { level } => BlockFormat::Heading(level),
                Command::Bullet => BlockFormat::Bullet,
                Command::Numbered => BlockFormat::Numbered,
                _ => BlockFormat::Task,
            };
            region.block_format(source, block).map_err(format_error)?
        }
        Command::ToggleTask => {
            let (range, text) = region
                .list
                .as_ref()
                .and_then(|item| item.task_patch(source))
                .ok_or(tr(L::ChooseTextBlock))?;
            FormatPatch::new(range, text, region.source.start)
        }
        Command::InsertTable => parsed
            .insert_table(source, index)
            .ok_or(tr(L::ChooseTextBlock))?,
        Command::InsertRow | Command::DeleteRow | Command::InsertColumn | Command::DeleteColumn => {
            let edit = match command {
                Command::InsertRow => TableEdit::AddRow,
                Command::DeleteRow => TableEdit::DeleteRow,
                Command::InsertColumn => TableEdit::AddColumn,
                _ => TableEdit::DeleteColumn,
            };
            parsed
                .edit_table(source, index, edit)
                .map_err(format_error)?
        }
        Command::Link { target } => parsed
            .change_link(source, index, local, &target)
            .map_err(format_error)?,
        Command::Break { soft } => {
            let end = region.text.encode_utf16().count();
            if !soft && region.list.is_some() {
                if !local.is_empty() {
                    return Err(tr(L::EditSourceHint).into());
                }
                let caret = if local.is_empty() && local.start == end {
                    region.source.end
                } else {
                    let (range, _) = region
                        .patch(source, local.clone(), "")
                        .ok_or(tr(L::EditSourceHint))?;
                    range.start
                };
                let (range, text, caret) = region
                    .continue_list(source, caret)
                    .ok_or(tr(L::EditSourceHint))?;
                FormatPatch::new(range, text, caret)
            } else if local.is_empty()
                && local.start == end
                && !soft
                && !region.quote
                && matches!(region.kind, "heading" | "paragraph")
            {
                let point = region.source.end;
                let text = newline(source).repeat(2);
                let caret = point + text.len();
                FormatPatch::new(point..point, text, caret)
            } else if soft || region.kind == "paragraph" && !region.quote && region.runs.len() == 1
            {
                let (range, text) = region
                    .patch(source, local, if soft { "\u{000b}" } else { "\n" })
                    .ok_or(tr(L::EditSourceHint))?;
                let caret = range.start + text.len();
                FormatPatch::new(range, text, caret)
            } else {
                return Err(tr(L::EditSourceHint).into());
            }
        }
    };
    doc.patch(patch.range, &patch.text, false);
    Ok(patch
        .focus_region
        .map_or(CaretTarget::Source(patch.caret), CaretTarget::Region))
}

pub fn replace_selection(doc: &mut Document, parts: Vec<Part>, text: &str) -> Result<Bytes> {
    if doc.mode != 2 {
        return Err(tr(L::UnsupportedEdit).into());
    }
    let parsed = projection(doc);
    let selection = parts
        .into_iter()
        .map(|part| part.checked(&parsed))
        .collect::<Option<Vec<_>>>()
        .ok_or(tr(L::ChooseTextBlock))?;
    let patch = parsed
        .paste_text(&doc.text, &selection, text)
        .ok_or(tr(L::EditSourceHint))?;
    doc.patch(patch.range, &patch.text, false);
    Ok(patch.caret)
}

#[derive(Serialize)]
pub struct Caret {
    region: usize,
    start: usize,
    end: usize,
}

pub enum CaretTarget {
    Source(Bytes),
    Region(usize),
}

impl CaretTarget {
    pub fn locate(&self, parsed: &Markdown) -> Option<Caret> {
        match self {
            Self::Source(raw) => caret(parsed, raw),
            Self::Region(region) => parsed.regions.get(*region).map(|_| Caret {
                region: *region,
                start: 0,
                end: 0,
            }),
        }
    }
}

fn caret(parsed: &Markdown, raw: &Range<usize>) -> Option<Caret> {
    let (region, item) = parsed
        .regions
        .iter()
        .enumerate()
        .rev()
        .find(|(_, region)| region.source.start <= raw.start && raw.start <= region.source.end)
        .or_else(|| {
            parsed
                .regions
                .iter()
                .enumerate()
                .find(|(_, region)| raw.start < region.source.start)
        })?;
    let offset = |byte| {
        let units = || item.runs.iter().flat_map(|run| run.units.iter());
        units()
            .position(|unit| unit.as_ref().is_some_and(|span| span.end > byte))
            .unwrap_or_else(|| units().count())
    };
    Some(Caret {
        region,
        start: offset(raw.start),
        end: offset(raw.end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(region: usize, start: usize, end: usize, text: &str) -> Part {
        serde_json::from_value(serde_json::json!({
            "region": region, "start": start, "end": end, "text": text,
        }))
        .unwrap()
    }

    #[test]
    fn continuation_and_inserted_table_have_editable_destinations() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "- [x] First")?;
        let mut doc = Document::open(path, 2)?;
        let at = apply(&mut doc, Command::Break { soft: false }, part(0, 8, 8, ""))?;
        assert_eq!(doc.text, "- [x] First\n- [ ] ");
        let parsed = projection(&doc);
        let cursor = at.locate(&parsed).unwrap();
        assert_eq!(cursor.region, 1);
        assert_eq!(cursor.start, parsed.regions[1].text.encode_utf16().count());
        let at = apply(
            &mut doc,
            Command::Break { soft: false },
            part(cursor.region, cursor.start, cursor.end, ""),
        )?;
        assert_eq!(doc.text, "- [x] First\n\n");
        let parsed = projection(&doc);
        let cursor = at.locate(&parsed).unwrap();
        assert_eq!(cursor.region, 1);
        assert!(parsed.regions[1].source.is_empty());
        let at = apply(&mut doc, Command::InsertTable, part(1, 0, 0, ""))?;
        assert!(doc.text.ends_with("|   |   |\n| --- | --- |\n|   |   |"));
        let parsed = projection(&doc);
        let cursor = at.locate(&parsed).unwrap();
        assert_eq!(parsed.regions[cursor.region].row, Some(0));
        assert_eq!(parsed.regions[cursor.region].cell, Some(0));
        Ok(())
    }

    #[test]
    fn inline_format_preserves_pending_markdown_and_rejects_stale_selection() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "Lead before")?;
        let mut doc = Document::open(path, 2)?;
        let old = part(0, 5, 11, "before");
        doc.patch(5..11, "after", false);
        assert!(apply(&mut doc, Command::Bold, old).is_err());
        let at = apply(&mut doc, Command::Bold, part(0, 5, 10, "after"))?;
        assert_eq!(doc.text, "Lead **after**");
        let cursor = at.locate(&projection(&doc)).unwrap();
        assert_eq!((cursor.start, cursor.end), (5, 10));
        Ok(())
    }
}
