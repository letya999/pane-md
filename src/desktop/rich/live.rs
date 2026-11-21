use super::*;
use notepad_md::{diff, markdown::Edit};

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
