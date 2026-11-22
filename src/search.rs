use regex::RegexBuilder;
use serde::Deserialize;

use crate::{Bytes, markdown::Markdown, newline, normalize_newlines};

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct Options {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

pub fn matches(text: &str, query: &str, options: Options) -> Vec<Bytes> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(pattern) = RegexBuilder::new(&regex::escape(query))
        .case_insensitive(!options.case_sensitive)
        .build()
    else {
        return Vec::new();
    };
    let word = |ch: char| ch == '_' || ch.is_alphanumeric();
    pattern
        .find_iter(text)
        .map(|item| item.range())
        .filter(|range| {
            !options.whole_word
                || (!text[..range.start].chars().next_back().is_some_and(word)
                    && !text[range.end..].chars().next().is_some_and(word))
        })
        .collect()
}

pub struct Replacement {
    pub range: Bytes,
    pub text: String,
    pub caret: usize,
    pub count: usize,
}

pub fn replace(
    source: &str,
    query: &str,
    replacement: &str,
    options: Options,
    selected: Option<Bytes>,
) -> Option<Replacement> {
    if replacement.contains('\0') {
        return None;
    }
    let mut ranges = matches(source, query, options);
    if ranges.is_empty() {
        return None;
    }
    if source.contains(crate::review::TAG) || source.contains(crate::review::LEGACY_TAG) {
        let protected = Markdown::parse(source).review_metadata;
        let mut after = 0;
        ranges.retain(|range| {
            while protected
                .get(after)
                .is_some_and(|meta| meta.end <= range.start)
            {
                after += 1;
            }
            protected
                .get(after)
                .is_none_or(|meta| range.end <= meta.start)
        });
    }
    if let Some(selected) = selected {
        let index = ranges
            .iter()
            .position(|range| *range == selected)
            .or_else(|| ranges.iter().position(|range| range.start >= selected.end))
            .unwrap_or(0);
        let range = ranges.get(index)?.clone();
        ranges.clear();
        ranges.push(range);
    }
    let first = ranges.first()?.start;
    let last = ranges.last()?.end;
    let mut text = String::new();
    let mut cursor = first;
    let mut caret = first;
    let fallback = newline(source);
    let multiline = replacement.contains(['\r', '\n']);
    let mut endings = source
        .match_indices(['\r', '\n'])
        .map(|(at, _)| at)
        .peekable();
    for range in &ranges {
        text.push_str(&source[cursor..range.start]);
        if multiline {
            while endings.peek().is_some_and(|&at| at < range.start) {
                endings.next();
            }
            let ending = endings
                .peek()
                .map_or(fallback, |&at| newline(&source[at..]));
            text.push_str(&normalize_newlines(replacement, ending));
        } else {
            text.push_str(replacement);
        }
        if range.start == first {
            caret = first + text.len();
        }
        cursor = range.end;
    }
    Some(Replacement {
        range: first..last,
        text,
        caret,
        count: ranges.len(),
    })
}
