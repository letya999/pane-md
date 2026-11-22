use std::{collections::HashSet, ops::Range};

use serde::{Deserialize, Serialize};

use crate::{
    Bytes,
    markdown::{Lines, Markdown},
    newline, utf16_byte,
};

pub(crate) const TAG: &str = "<!-- pane-md:review";
pub(crate) const LEGACY_TAG: &str = "<!-- notepad-md:review";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
    #[serde(default)]
    pub heading: String,
    pub block: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<usize>,
    pub prefix: String,
    pub suffix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Resolved,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    pub v: u64,
    pub id: String,
    pub quote: String,
    pub anchor: Anchor,
    pub text: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Binding {
    Attached {
        #[serde(skip)]
        region: usize,
        #[serde(skip)]
        selection: Range<usize>,
        #[serde(skip)]
        tail: Option<(usize, Range<usize>)>,
        line: usize,
        column: usize,
        bytes: Bytes,
        row: Option<usize>,
        cell: Option<usize>,
    },
    Missing,
    Ambiguous,
}

#[derive(Debug, Serialize)]
pub struct Record {
    #[serde(flatten)]
    pub review: Review,
    #[serde(flatten)]
    pub binding: Binding,
    #[serde(skip)]
    pub source: Bytes,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub line: usize,
    pub message: String,
}

impl Review {
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("Review contains only JSON types");
        format!("{TAG} {} -->", json.replace("--", "\\u002D\\u002D"))
    }
}

pub(crate) fn load(doc: &mut Markdown, source: &str, lines: &Lines, span: Bytes) {
    let Some(raw) = source.get(span.clone()) else {
        return;
    };
    let trimmed = raw.trim();
    let Some(payload) = trimmed
        .strip_prefix(TAG)
        .or_else(|| trimmed.strip_prefix(LEGACY_TAG))
    else {
        return;
    };
    if !payload.starts_with(char::is_whitespace) && !payload.starts_with("-->") {
        return;
    }
    doc.review_metadata.push(span.clone());
    let parsed = (|| {
        let json = payload
            .strip_suffix("-->")
            .ok_or("missing HTML comment terminator")?
            .trim();
        if json.contains("--") {
            return Err("unescaped double hyphen in review JSON".into());
        }
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let version = value
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .ok_or("missing or invalid version")?;
        if version != 1 {
            return Ok(None);
        }
        let review: Review = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if review.id.trim().is_empty() || review.quote.trim().is_empty() {
            return Err("empty id or quote".into());
        }
        if review.anchor.span.is_some_and(|span| span < 2) {
            return Err("invalid review span".into());
        }
        Ok(Some(review))
    })();
    match parsed {
        Ok(Some(review)) => {
            let start = span.start + raw.len() - raw.trim_start().len();
            doc.records.push(Record {
                review,
                binding: Binding::Missing,
                source: start..start + trimmed.len(),
            });
        }
        Ok(None) => {}
        Err(message) => doc.diagnostics.push(Diagnostic {
            line: lines.position(span.start).0,
            message,
        }),
    }
}

pub(crate) fn bind(doc: &mut Markdown, source: &str) {
    let lines = Lines::new(source);
    let mut ids = HashSet::new();
    doc.records.retain(|record| {
        if ids.insert(record.review.id.clone()) {
            true
        } else {
            doc.diagnostics.push(Diagnostic {
                line: lines.position(record.source.start).0,
                message: format!("duplicate review id {}", record.review.id),
            });
            false
        }
    });
    for record in &mut doc.records {
        let anchor = &record.review.anchor;
        if let Some(span) = anchor.span {
            record.binding = bind_span(&doc.regions, &record.review, span, &lines);
            continue;
        }
        let near = doc
            .owners
            .partition_point(|owner| owner.source.end < record.source.start)
            .checked_sub(1);
        let mut exact = Vec::new();
        let mut nearby = Vec::new();
        for (index, region) in doc.regions.iter().enumerate() {
            for (start, _) in region
                .text
                .char_indices()
                .filter(|(at, _)| region.text[*at..].starts_with(&record.review.quote))
            {
                let end = start + record.review.quote.len();
                let prefix = &region.text[..start];
                let suffix = &region.text[end..];
                let selection =
                    prefix.encode_utf16().count()..region.text[..end].encode_utf16().count();
                let Some(bytes) = region.source_selection(selection.clone()) else {
                    continue;
                };
                let heading = region.heading == anchor.heading;
                let block = region.kind == anchor.block;
                let owner = Some(region.owner) == near;
                let coordinates = region.row == anchor.row && region.cell == anchor.cell;
                let context = prefix.ends_with(&anchor.prefix)
                    && suffix.starts_with(&anchor.suffix)
                    && (!anchor.prefix.is_empty() || prefix.is_empty())
                    && (!anchor.suffix.is_empty() || suffix.is_empty());
                let candidate = (index, selection, bytes);
                if context {
                    // Region coordinates only guide recovery. Two equal matches in the same owner stay ambiguous.
                    let rank = (heading && block, block);
                    exact.push((rank, candidate));
                } else if heading
                    && block
                    && owner
                    && (coordinates
                        || (!anchor.prefix.is_empty() && prefix.ends_with(&anchor.prefix))
                        || (!anchor.suffix.is_empty() && suffix.starts_with(&anchor.suffix)))
                {
                    nearby.push((coordinates, candidate));
                }
            }
        }
        let mut selected = Vec::new();
        if let Some(top) = exact.iter().map(|(rank, _)| *rank).max() {
            selected.extend(
                exact
                    .into_iter()
                    .filter_map(|(rank, candidate)| (rank == top).then_some(candidate)),
            );
        } else if nearby.len() == 1 {
            selected.push(nearby.pop().expect("one candidate").1);
        } else if nearby.len() > 1 {
            record.binding = Binding::Ambiguous;
            continue;
        }
        if selected.len() == 1 {
            let (region, selection, bytes) = selected.pop().expect("one candidate");
            let (line, column) = lines.position(bytes.start);
            let r = &doc.regions[region];
            record.binding = Binding::Attached {
                region,
                selection,
                tail: None,
                line,
                column,
                bytes,
                row: r.row,
                cell: r.cell,
            };
        } else if !selected.is_empty() {
            record.binding = Binding::Ambiguous;
        }
    }
}

fn bind_span(
    regions: &[crate::markdown::Region],
    review: &Review,
    span: usize,
    lines: &Lines,
) -> Binding {
    if span > regions.len() {
        return Binding::Missing;
    }
    let mut best = None;
    let mut tied = false;
    for (index, window) in regions.windows(span).enumerate() {
        let first = &window[0];
        let last = &window[span - 1];
        for (start, _) in first.text.char_indices() {
            let Some(mut quote) = review.quote.strip_prefix(&first.text[start..]) else {
                continue;
            };
            for region in &window[1..span - 1] {
                let Some(rest) = quote
                    .strip_prefix('\n')
                    .and_then(|q| q.strip_prefix(&region.text))
                else {
                    quote = "";
                    break;
                };
                quote = rest;
            }
            let Some(end) = quote
                .strip_prefix('\n')
                .filter(|end| !end.is_empty() && last.text.starts_with(end))
            else {
                continue;
            };
            let selection =
                first.text[..start].encode_utf16().count()..first.text.encode_utf16().count();
            let tail = 0..end.encode_utf16().count();
            let Some((left, right)) = first
                .source_selection(selection.clone())
                .zip(last.source_selection(tail.clone()))
            else {
                continue;
            };
            if left.start > right.end {
                continue;
            }
            let context = first.text[..start].ends_with(&review.anchor.prefix)
                && last.text[end.len()..].starts_with(&review.anchor.suffix)
                && (!review.anchor.prefix.is_empty() || start == 0)
                && (!review.anchor.suffix.is_empty() || end.len() == last.text.len());
            let heading = first.heading == review.anchor.heading;
            if !context && !heading {
                continue;
            }
            let rank = (context, heading, first.kind == review.anchor.block);
            if best.as_ref().is_some_and(|(previous, _)| *previous > rank) {
                continue;
            }
            tied = best.as_ref().is_some_and(|(previous, _)| *previous == rank);
            let (line, column) = lines.position(left.start);
            best = Some((
                rank,
                Binding::Attached {
                    region: index,
                    selection,
                    tail: Some((index + span - 1, tail)),
                    line,
                    column,
                    bytes: left.start..right.end,
                    row: first.row,
                    cell: first.cell,
                },
            ));
        }
    }
    if tied {
        Binding::Ambiguous
    } else {
        best.map_or(Binding::Missing, |(_, binding)| binding)
    }
}

pub fn capture_span(
    doc: &Markdown,
    first: (usize, Range<usize>),
    last: (usize, Range<usize>),
) -> Option<(usize, String, Anchor)> {
    if first.0 == last.0 {
        return capture(doc, first.0, first.1);
    }
    let (.., first_quote, mut anchor) = capture(doc, first.0, first.1)?;
    let (after, last_quote, end_anchor) = capture(doc, last.0, last.1)?;
    let between = doc.regions.get(first.0 + 1..last.0)?;
    let mut quote = first_quote;
    for region in between {
        quote.push('\n');
        quote.push_str(&region.text);
    }
    quote.push('\n');
    quote.push_str(&last_quote);
    anchor.suffix = end_anchor.suffix;
    anchor.span = Some(last.0 - first.0 + 1);
    Some((after, quote, anchor))
}

pub fn capture(
    doc: &Markdown,
    region: usize,
    selection: Range<usize>,
) -> Option<(usize, String, Anchor)> {
    let r = doc.regions.get(region)?;
    r.source_selection(selection.clone())?;
    let start = utf16_byte(&r.text, selection.start)?;
    let end = utf16_byte(&r.text, selection.end)?;
    let quote = r.text[start..end].to_owned();
    if quote.trim().is_empty() {
        return None;
    }
    let before = &r.text[..start];
    let after = &r.text[end..];
    let first = before.char_indices().rev().nth(31).map_or(0, |(at, _)| at);
    let last = after
        .char_indices()
        .nth(32)
        .map_or(after.len(), |(at, _)| at);
    let anchor = Anchor {
        heading: r.heading.clone(),
        block: r.kind.to_owned(),
        row: r.row,
        cell: r.cell,
        prefix: before[first..].to_owned(),
        suffix: after[..last].to_owned(),
        span: None,
    };
    Some((doc.owners[r.owner].after, quote, anchor))
}

/// Project a Markdown source selection onto reviewable text, omitting Markdown delimiters.
pub fn capture_source(doc: &Markdown, range: Bytes) -> Option<(usize, String, Anchor)> {
    if range.is_empty()
        || doc
            .records
            .iter()
            .any(|record| record.source.start < range.end && range.start < record.source.end)
    {
        return None;
    }
    let mut parts = doc
        .regions
        .iter()
        .enumerate()
        .filter_map(|(index, region)| {
            let mut selected = region
                .runs
                .iter()
                .flat_map(|run| &run.units)
                .enumerate()
                .filter_map(|(unit, source)| {
                    let source = source.as_ref()?;
                    (source.start < range.end && range.start < source.end).then_some(unit)
                });
            let start = selected.next()?;
            let end = selected.last().unwrap_or(start) + 1;
            Some((index, start..end))
        });
    let first = parts.next()?;
    let last = parts.next_back().unwrap_or_else(|| first.clone());
    capture_span(doc, first, last)
}

/// Move a record to its new owner as one undoable edit.
pub fn relocate(source: &str, record: &Record, after: usize, updated: &Review) -> (Bytes, String) {
    let old = record.source.clone();
    if after <= old.start {
        let new = insertion(source, after, updated);
        (
            after..old.end,
            format!("{new}{}", &source[after..old.start]),
        )
    } else if after >= old.end {
        let new = insertion(source, after, updated);
        (
            old.start..after,
            format!("{}{new}", &source[old.end..after]),
        )
    } else {
        (old, updated.encode())
    }
}

pub fn insertion(source: &str, after: usize, review: &Review) -> String {
    let ending = newline(source);
    let tail = &source[after..];
    let has_gap = tail
        .strip_prefix("\r\n")
        .or_else(|| tail.strip_prefix(['\n', '\r']))
        .is_some_and(|next| {
            next.trim_start_matches([' ', '\t'])
                .starts_with(['\n', '\r'])
        });
    let separator = if tail.trim().is_empty() || has_gap {
        ""
    } else {
        ending
    };
    format!("{ending}{ending}{}{separator}", review.encode())
}
