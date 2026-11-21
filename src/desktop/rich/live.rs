use super::*;
use notepad_md::{diff, markdown::Edit};

pub(in crate::desktop) fn apply_plain_edit(
    doc: &mut Markdown,
    view: &mut Projection,
    source: &str,
    changed: Range<usize>,
    bytes: &Range<usize>,
    inserted: &str,
) -> bool {
    // shortcut: use the parser for Markdown syntax and review reattachment.
    if !doc.records.is_empty()
        || !doc.diagnostics.is_empty()
        || !inserted.chars().all(char::is_alphanumeric)
        || !source[bytes.clone()].chars().all(char::is_alphanumeric)
    {
        return false;
    }
    let Some((index, local)) = view.region_range(changed.clone()) else {
        return false;
    };
    let region = &mut doc.regions[index];
    let [run] = region.runs.as_mut_slice() else {
        return false;
    };
    if region.level != 0
        || region.indent != 0
        || !matches!(region.kind, "paragraph" | "table")
        || !matches!(run.edit, Some(Edit::Text | Edit::Cell))
        || run.link.is_some()
        || run.image.is_some()
        || source.get(region.source.clone()) != Some(region.text.as_str())
        || !region
            .text
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | ',' | '\''))
        || (inserted.is_empty()
            && (bytes.start == region.source.start || bytes.end == region.source.end))
    {
        return false;
    }
    let first = bytes.start - region.source.start;
    let last = bytes.end - region.source.start;
    let byte_delta = inserted.len() as isize - bytes.len() as isize;
    let utf16_len = inserted.encode_utf16().count();
    let native_delta = utf16_len as isize - changed.len() as isize;
    let owner = region.owner;
    let shift = |point: &mut usize, delta: isize| *point = point.saturating_add_signed(delta);
    if byte_delta != 0 {
        for unit in run.units[local.end..].iter_mut().flatten() {
            shift(&mut unit.start, byte_delta);
            shift(&mut unit.end, byte_delta);
        }
        shift(&mut region.source.end, byte_delta);
    }
    run.text.replace_range(first..last, inserted);
    region.text.replace_range(first..last, inserted);
    let units = inserted.char_indices().flat_map(|(offset, ch)| {
        let at = bytes.start + offset;
        std::iter::repeat_n(Some(at..at + ch.len_utf8()), ch.len_utf16())
    });
    run.units.splice(local, units);

    if byte_delta != 0 {
        let shift_span = |span: &mut Range<usize>| {
            if span.start >= bytes.end {
                shift(&mut span.start, byte_delta);
            }
            if span.end >= bytes.end {
                shift(&mut span.end, byte_delta);
            }
        };
        for region in &mut doc.regions[index + 1..] {
            shift_span(&mut region.source);
            for unit in region
                .runs
                .iter_mut()
                .flat_map(|run| &mut run.units)
                .flatten()
            {
                shift_span(unit);
            }
        }
        shift(&mut doc.owners[owner].source.end, byte_delta);
        shift(&mut doc.owners[owner].after, byte_delta);
        for block in &mut doc.owners[owner + 1..] {
            shift_span(&mut block.source);
            shift(&mut block.after, byte_delta);
        }
        for block in &mut doc.layout {
            if let Layout::Table { source, .. } = block {
                if source.start <= bytes.start && bytes.end <= source.end {
                    shift(&mut source.end, byte_delta);
                } else if source.start >= bytes.end {
                    shift_span(source);
                }
            }
        }
    }
    if native_delta != 0 {
        shift(&mut view.region[index].end, native_delta);
        for span in &mut view.region[index + 1..] {
            shift(&mut span.start, native_delta);
            shift(&mut span.end, native_delta);
        }
        for (span, _) in &mut view.links {
            if span.start >= changed.end {
                shift(&mut span.start, native_delta);
                shift(&mut span.end, native_delta);
            }
        }
    }
    true
}

/// Keep whitespace entered at a Markdown boundary available to the next keystroke.
pub(in crate::desktop) fn fit_trimmed_spaces(
    doc: &mut Markdown,
    view: &mut Projection,
    text: &[u16],
    source: &str,
    caret: usize,
) -> bool {
    let (projected, missing) = diff(&view.text, text);
    if !projected.is_empty()
        || missing.is_empty()
        || !text[missing.clone()].iter().all(|&unit| unit == 32)
    {
        return false;
    }
    let at = projected.start;
    let len = missing.len();
    let Some(byte_start) = caret.checked_sub(len) else {
        return false;
    };
    if !source
        .get(byte_start..caret)
        .is_some_and(|range| range.bytes().all(|b| b == b' '))
    {
        return false;
    }
    let Some((i, local)) = view.region_range(at..at) else {
        return false;
    };
    if view.translated.contains_key(&i) {
        return false;
    }
    let region = &mut doc.regions[i];
    let mut at_run = 0;
    for run in &mut region.runs {
        let end = at_run + run.units.len();
        if run.edit.is_some() && (at_run..=end).contains(&local.start) {
            let index = local.start - at_run;
            let Some(run_at) = notepad_md::utf16_byte(&run.text, index) else {
                return false;
            };
            let Some(region_at) = notepad_md::utf16_byte(&region.text, local.start) else {
                return false;
            };
            let literal = &source[byte_start..caret];
            run.units.splice(
                index..index,
                (byte_start..caret).map(|byte| Some(byte..byte + 1)),
            );
            run.text.insert_str(run_at, literal);
            region.text.insert_str(region_at, literal);
            region.source.start = region.source.start.min(byte_start);
            region.source.end = region.source.end.max(caret);
            view.text.splice(at..at, std::iter::repeat_n(32, len));
            view.region[i].end += len;
            for span in &mut view.region[i + 1..] {
                span.start += len;
                span.end += len;
            }
            for (span, _) in &mut view.links {
                if span.start >= at {
                    span.start += len;
                }
                if span.end > at {
                    span.end += len;
                }
            }
            for record in &mut doc.records {
                if let Binding::Attached {
                    region, selection, ..
                } = &mut record.binding
                    && *region == i
                {
                    if selection.start >= local.start {
                        selection.start += len;
                    }
                    if selection.end > local.start {
                        selection.end += len;
                    }
                }
            }
            return true;
        }
        at_run = end;
    }
    false
}

pub(in crate::desktop) fn fit_new_paragraph(
    doc: &mut Markdown,
    view: &mut Projection,
    text: &[u16],
    source_caret: usize,
) -> bool {
    if text.len() != view.text.len() + 1
        || text.last() != Some(&13)
        || !text.starts_with(&view.text)
    {
        return false;
    }
    let index = doc.regions.len();
    let heading = doc
        .regions
        .last()
        .map_or_else(String::new, |region| region.heading.clone());
    doc.owners.push(notepad_md::markdown::Owner {
        source: source_caret..source_caret,
        after: source_caret,
    });
    doc.regions.push(Region {
        runs: vec![Run {
            edit: Some(Edit::Text),
            ..Run::default()
        }],
        source: source_caret..source_caret,
        owner: doc.owners.len() - 1,
        kind: "paragraph",
        heading,
        ..Region::default()
    });
    doc.layout.push(Layout::Text(index));
    view.text.push(13);
    view.region.push(text.len()..text.len());
    true
}
