use std::{collections::HashSet, ops::Range};

use serde::{Deserialize, Serialize};

use crate::{
    Bytes,
    markdown::{Lines, Markdown},
    newline, utf16_byte,
};

const TAG: &str = "<!-- notepad-md:review";

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
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Binding {
    Attached {
        #[serde(skip)]
        region: usize,
        #[serde(skip)]
        selection: Range<usize>,
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
    let Some(payload) = trimmed.strip_prefix(TAG) else {
        return;
    };
    if !payload.starts_with(char::is_whitespace) && !payload.starts_with("-->") {
        return;
    }
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
    };
    Some((doc.owners[r.owner].after, quote, anchor))
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
