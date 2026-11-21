pub mod markdown;
pub mod review;

use std::ops::Range;

/// Byte offsets, half-open, into UTF-8 Markdown without the optional BOM.
pub type Bytes = Range<usize>;

pub fn utf16_byte(text: &str, position: usize) -> Option<usize> {
    let mut offset = 0;
    for (byte, ch) in text.char_indices() {
        if position == offset {
            return Some(byte);
        }
        offset += ch.len_utf16();
        if offset > position {
            return None;
        }
    }
    (position == offset).then_some(text.len())
}

pub fn newline(text: &str) -> &'static str {
    match text.find(['\r', '\n']) {
        Some(at) if text[at..].starts_with("\r\n") => "\r\n",
        Some(at) if text.as_bytes()[at] == b'\r' => "\r",
        _ => "\n",
    }
}

pub fn normalize_newlines(text: &str, ending: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', ending)
}

pub fn diff<T: PartialEq>(before: &[T], after: &[T]) -> (Range<usize>, Range<usize>) {
    let start = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let end = before[start..]
        .iter()
        .rev()
        .zip(after[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (start..before.len() - end, start..after.len() - end)
}

/// RichEdit's source view uses one CR per original line ending.
pub fn source_view(text: &str) -> (Vec<u16>, Vec<Option<usize>>) {
    let mut display = Vec::with_capacity(text.len());
    let mut map = vec![Some(0)];
    let mut chars = text.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let mut end = at + ch.len_utf8();
        if ch == '\r' && chars.peek().is_some_and(|&(_, next)| next == '\n') {
            chars.next();
            end += 1;
        }
        let ch = if ch == '\n' { '\r' } else { ch };
        display.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
        if ch.len_utf16() == 2 {
            map.push(None);
        }
        map.push(Some(end));
    }
    (display, map)
}

/// Apply one localized source edit so line endings outside it stay byte-identical.
pub fn edit_source(
    text: &str,
    before: &[u16],
    map: &[Option<usize>],
    after: &[u16],
) -> Option<(Bytes, String)> {
    let (mut old, mut new) = diff(before, after);
    // A replacement can share the high surrogate of the original scalar.
    if map[old.start].is_none() {
        old.start -= 1;
        new.start -= 1;
    }
    if map[old.end].is_none() {
        old.end += 1;
        new.end += 1;
    }
    let replacement = String::from_utf16(after.get(new)?).ok()?;
    let range = map[old.start]?..map[old.end]?;
    let ending = newline(&text[range.start..]);
    let ending = if text[range.start..].contains(['\r', '\n']) {
        ending
    } else {
        newline(text)
    };
    Some((range, normalize_newlines(&replacement, ending)))
}
