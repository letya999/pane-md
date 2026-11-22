use std::{
    collections::HashMap,
    ops::Range,
    time::{SystemTime, UNIX_EPOCH},
};

use pane_md::{
    markdown::Markdown,
    normalize_newlines,
    review::{self, Anchor, Binding, Record, Review, Status},
    source_view, utf16_byte,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Result,
    document::Document,
    i18n::{Label as L, tr},
};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Operation {
    Start {
        selection: Option<Selection>,
    },
    Edit {
        id: String,
    },
    Commit {
        text: String,
    },
    Cancel,
    Toggle {
        id: String,
    },
    Delete {
        id: String,
    },
    Reattach {
        id: String,
        selection: Option<Selection>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Selection {
    Source { start: usize, end: usize },
    Visual { first: Part, last: Part },
}

#[derive(Deserialize)]
pub(super) struct Part {
    region: usize,
    start: usize,
    end: usize,
    text: String,
}

impl Part {
    pub(super) fn checked(self, parsed: &Markdown) -> Option<(usize, Range<usize>)> {
        let text = &parsed.regions.get(self.region)?.text;
        let range = utf16_byte(text, self.start)?..utf16_byte(text, self.end)?;
        (normalize_newlines(text.get(range)?, "\n").replace('\u{000b}', "\n") == self.text)
            .then_some((self.region, self.start..self.end))
    }
}

impl Selection {
    fn capture(self, doc: &Document, parsed: &Markdown) -> Option<(usize, String, Anchor)> {
        match self {
            Self::Source { start, end } if !doc.mode.is_multiple_of(2) => {
                let (_, map) = source_view(&doc.text);
                let start = (*map.get(start)?)?;
                let end = (*map.get(end)?)?;
                review::capture_source(parsed, start..end)
            }
            Self::Visual { first, last } if doc.mode.is_multiple_of(2) => {
                let first = first.checked(parsed)?;
                let last = last.checked(parsed)?;
                if first.0 > last.0 || first.0 == last.0 && first.1 != last.1 {
                    return None;
                }
                review::capture_span(parsed, first, last)
            }
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Editor {
    draft: Option<Draft>,
    focus: Option<String>,
}

enum Target {
    New {
        after: usize,
        quote: String,
        anchor: Anchor,
    },
    Existing {
        id: String,
    },
}

struct Draft {
    target: Target,
    text: String,
}

impl Editor {
    pub(super) fn allows(&self, operation: Option<&Operation>) -> bool {
        !self.is_drafting()
            || matches!(
                operation,
                Some(Operation::Commit { .. } | Operation::Cancel)
            )
    }

    pub(super) fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }

    pub(super) fn snapshot(&self, parsed: &Markdown) -> Value {
        let draft = self.draft.as_ref().map(|draft| {
            let (id, quote) = match &draft.target {
                Target::New { quote, .. } => (None, Some(quote.as_str())),
                Target::Existing { id } => (
                    Some(id.as_str()),
                    record(parsed, id)
                        .ok()
                        .map(|entry| entry.review.quote.as_str()),
                ),
            };
            json!({"id": id, "quote": quote, "text": draft.text})
        });
        json!({"draft": draft, "focus": self.focus})
    }

    pub(super) fn apply(
        &mut self,
        doc: &mut Document,
        operation: Operation,
    ) -> Result<&'static str> {
        if !doc.show_reviews {
            return Err(tr(L::ReviewPane).into());
        }
        if !self.allows(Some(&operation)) {
            return Err(tr(L::FinishDraft).into());
        }
        if matches!(operation, Operation::Cancel) {
            self.draft = None;
            return Ok("");
        }
        let parsed = Markdown::parse(&doc.text);
        let capture = |selection: Option<Selection>| {
            selection
                .and_then(|selection| selection.capture(doc, &parsed))
                .ok_or(tr(L::SelectForReviewPortable))
        };
        match operation {
            Operation::Start { selection } => {
                let (after, quote, anchor) = capture(selection)?;
                self.draft = Some(Draft {
                    target: Target::New {
                        after,
                        quote,
                        anchor,
                    },
                    text: String::new(),
                });
                Ok("")
            }
            Operation::Edit { id } => {
                let entry = record(&parsed, &id)?;
                self.focus = Some(id.clone());
                self.draft = Some(Draft {
                    target: Target::Existing { id },
                    text: entry.review.text.clone(),
                });
                Ok("")
            }
            Operation::Commit { text } => {
                let draft = self.draft.as_mut().ok_or(tr(L::CommentUnavailable))?;
                draft.text = normalize_newlines(text.trim(), "\n");
                if draft.text.is_empty() || draft.text.contains('\0') {
                    return Err(tr(L::EnterReview).into());
                }
                let (range, replacement, id) = match &draft.target {
                    Target::New {
                        after,
                        quote,
                        anchor,
                    } => {
                        let review = Review {
                            v: 1,
                            id: uuid::Uuid::new_v4().simple().to_string(),
                            quote: quote.clone(),
                            anchor: anchor.clone(),
                            text: draft.text.clone(),
                            status: Status::Open,
                            updated_at: Some(now()),
                        };
                        (
                            *after..*after,
                            review::insertion(&doc.text, *after, &review),
                            review.id,
                        )
                    }
                    Target::Existing { id } => {
                        let entry = record(&parsed, id)?;
                        let mut updated = entry.review.clone();
                        if updated.text != draft.text {
                            updated.text.clone_from(&draft.text);
                            updated.updated_at = Some(now());
                        }
                        (entry.source.clone(), updated.encode(), id.clone())
                    }
                };
                doc.patch(range, &replacement, false);
                self.draft = None;
                self.focus = Some(id);
                Ok(tr(L::ReviewChanged))
            }
            Operation::Toggle { id } => {
                let entry = record(&parsed, &id)?;
                let mut updated = entry.review.clone();
                updated.status = match updated.status {
                    Status::Open => Status::Resolved,
                    Status::Resolved => Status::Open,
                };
                updated.updated_at = Some(now());
                doc.patch(entry.source.clone(), &updated.encode(), false);
                self.focus = Some(id);
                Ok(tr(L::ReviewChanged))
            }
            Operation::Delete { id } => {
                let entry = record(&parsed, &id)?;
                doc.patch(entry.source.clone(), "", false);
                self.focus = None;
                Ok(tr(L::ReviewChanged))
            }
            Operation::Reattach { id, selection } => {
                let entry = record(&parsed, &id)?;
                let (after, quote, anchor) = capture(selection)?;
                let mut updated = entry.review.clone();
                updated.quote = quote;
                updated.anchor = anchor;
                updated.updated_at = Some(now());
                let (range, replacement) = review::relocate(&doc.text, entry, after, &updated);
                doc.patch(range, &replacement, false);
                self.focus = Some(id);
                Ok(tr(L::ReviewChanged))
            }
            Operation::Cancel => unreachable!(),
        }
    }
}

fn record<'a>(parsed: &'a Markdown, id: &str) -> Result<&'a Record> {
    parsed
        .records
        .iter()
        .find(|record| record.review.id == id)
        .ok_or_else(|| tr(L::CommentUnavailable).into())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) type Highlights = HashMap<usize, Vec<(Range<usize>, usize)>>;

pub(super) fn highlights(doc: &Markdown) -> Highlights {
    let mut marks = Highlights::new();
    for (id, record) in doc.records.iter().enumerate() {
        let Binding::Attached {
            region,
            selection,
            tail,
            ..
        } = &record.binding
        else {
            continue;
        };
        marks
            .entry(*region)
            .or_default()
            .push((selection.clone(), id));
        if let Some((last, finish)) = tail {
            for (index, inner) in doc.regions.iter().enumerate().take(*last).skip(region + 1) {
                let length = inner.text.encode_utf16().count();
                marks.entry(index).or_default().push((0..length, id));
            }
            marks.entry(*last).or_default().push((finish.clone(), id));
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn source_selection(doc: &Document, needle: &str) -> Selection {
        let at = doc.text.find(needle).expect("fixture selection");
        let (_, map) = source_view(&doc.text);
        let offset = |byte| map.iter().position(|&point| point == Some(byte)).unwrap();
        Selection::Source {
            start: offset(at),
            end: offset(at + needle.len()),
        }
    }

    #[test]
    fn comment_lifecycle_preserves_markdown_and_uses_shared_bindings() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("review.md");
        let original = "\u{feff}# Review\r\n\r\nTry **an example 😀**.\r\n\r\n## Update\r\n\r\nAnother place.\r\n";
        fs::write(&path, original)?;
        let mut doc = Document::open(path.clone(), 1)?;
        let mut editor = Editor::default();
        let selection = source_selection(&doc, "**an example 😀**");
        editor.apply(
            &mut doc,
            Operation::Start {
                selection: Some(selection),
            },
        )?;
        assert!(!editor.allows(None));
        assert_eq!(
            editor.snapshot(&Markdown::parse(&doc.text))["draft"]["quote"],
            "an example 😀"
        );
        editor.apply(
            &mut doc,
            Operation::Commit {
                text: "Line one -->\r\nВторая строка".into(),
            },
        )?;
        let parsed = Markdown::parse(&doc.text);
        assert!(parsed.diagnostics.is_empty());
        let entry = &parsed.records[0];
        assert!(matches!(entry.binding, Binding::Attached { .. }));
        assert_eq!(entry.review.text, "Line one -->\nВторая строка");
        let id = entry.review.id.clone();
        assert!(!editor.is_drafting());
        assert_eq!(fs::read_to_string(&path)?, original);

        editor.apply(&mut doc, Operation::Edit { id: id.clone() })?;
        editor.apply(&mut doc, Operation::Cancel)?;
        assert_eq!(Markdown::parse(&doc.text).records[0].review, entry.review);
        editor.apply(&mut doc, Operation::Edit { id: id.clone() })?;
        editor.apply(
            &mut doc,
            Operation::Commit {
                text: "Revised".into(),
            },
        )?;
        editor.apply(&mut doc, Operation::Toggle { id: id.clone() })?;
        assert_eq!(
            Markdown::parse(&doc.text).records[0].review.status,
            Status::Resolved
        );
        editor.apply(&mut doc, Operation::Toggle { id: id.clone() })?;

        let before_moving = doc.text.clone();
        let selection = source_selection(&doc, "Another place.");
        editor.apply(
            &mut doc,
            Operation::Reattach {
                id: id.clone(),
                selection: Some(selection),
            },
        )?;
        let parsed = Markdown::parse(&doc.text);
        let entry = &parsed.records[0];
        assert!(matches!(entry.binding, Binding::Attached { .. }));
        assert_eq!(entry.review.id, id);
        assert_eq!(entry.review.quote, "Another place.");
        assert_eq!(entry.review.status, Status::Open);
        assert_eq!(entry.review.text, "Revised");
        assert!(doc.undo(false).is_some());
        assert_eq!(doc.text, before_moving);
        assert!(doc.undo(true).is_some());
        doc.save()?;
        let saved = fs::read_to_string(&path)?;
        assert!(saved.starts_with('\u{feff}'));
        assert_eq!(
            saved.bytes().filter(|&byte| byte == b'\r').count(),
            saved.bytes().filter(|&byte| byte == b'\n').count()
        );
        assert!(saved.contains("Try **an example 😀**."));

        let before_deleting = doc.text.clone();
        editor.apply(&mut doc, Operation::Delete { id })?;
        assert!(Markdown::parse(&doc.text).records.is_empty());
        assert!(doc.undo(false).is_some());
        assert_eq!(doc.text, before_deleting);
        Ok(())
    }

    #[test]
    fn visual_selection_spans_runs_without_accepting_a_stale_snapshot() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("review.md");
        fs::write(&path, "Before **bold**.\n\nSecond 😀 paragraph.\n")?;
        let mut doc = Document::open(path.clone(), 0)?;
        let mut editor = Editor::default();
        let selection = |text: &str| Selection::Visual {
            first: Part {
                region: 0,
                start: 7,
                end: 12,
                text: text.to_owned(),
            },
            last: Part {
                region: 1,
                start: 0,
                end: 9,
                text: "Second 😀".into(),
            },
        };
        assert!(
            editor
                .apply(
                    &mut doc,
                    Operation::Start {
                        selection: Some(selection("wrong"))
                    }
                )
                .is_err()
        );
        editor.apply(
            &mut doc,
            Operation::Start {
                selection: Some(selection("bold.")),
            },
        )?;
        editor.apply(
            &mut doc,
            Operation::Commit {
                text: "Two places".into(),
            },
        )?;
        let parsed = Markdown::parse(&doc.text);
        assert_eq!(parsed.records[0].review.quote, "bold.\nSecond 😀");
        let html = crate::portable::preview::html(&parsed, Some(&path), &doc.text, false, true);
        assert!(html.contains("<mark class=\"review-mark\" data-reviews=\"0\">bold"));
        assert!(html.contains("<mark class=\"review-mark\" data-reviews=\"0\">Second 😀"));
        Ok(())
    }
}
