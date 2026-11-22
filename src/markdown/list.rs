use std::ops::Range;

use crate::{Bytes, newline};

use super::Region;

pub struct ListItem {
    pub source: Bytes,
    pub marker: Bytes,
    pub task: Option<usize>,
}

impl ListItem {
    pub(super) fn new(document: &str, source: Bytes, task: Option<usize>) -> Option<Self> {
        let text = document.get(source.clone())?;
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        let token = *text.as_bytes().get(digits)?;
        if !(digits == 0 && matches!(token, b'-' | b'+' | b'*')
            || (1..=9).contains(&digits) && matches!(token, b'.' | b')'))
        {
            return None;
        }
        let end = source.start + digits + 1;
        if document
            .as_bytes()
            .get(end)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            return None;
        }
        if let Some(task) = task {
            let brackets = document.as_bytes().get(task.checked_sub(1)?..task + 2)?;
            if !matches!(brackets, [b'[', b' ' | b'x' | b'X', b']']) {
                return None;
            }
        }
        Some(Self {
            marker: source.start..end,
            source,
            task,
        })
    }

    fn line_prefix<'a>(&self, source: &'a str) -> Option<&'a str> {
        let prefix = source
            .get(..self.marker.start)?
            .rsplit(['\r', '\n'])
            .next()?;
        prefix
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'>'))
            .then_some(prefix)
    }

    pub(super) fn continuation_indent(&self, source: &str) -> Option<String> {
        let prefix = self.line_prefix(source)?;
        let spaces = source
            .get(self.marker.end..)?
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let padding = source.get(self.marker.end..self.marker.end + spaces)?;
        Some(format!(
            "{prefix}{}{padding}",
            " ".repeat(self.marker.len())
        ))
    }

    pub fn task_patch(&self, source: &str) -> Option<(Bytes, &'static str)> {
        let at = self.task?;
        let span = at.checked_sub(1)?..at + 2;
        let next = match source.as_bytes().get(span)? {
            [b'[', b' ', b']'] => "x",
            [b'[', b'x' | b'X', b']'] => " ",
            _ => return None,
        };
        Some((at..at + 1, next))
    }

    pub fn create_task_patch(&self, source: &str) -> Option<(Bytes, &'static str)> {
        self.task.is_none().then_some(())?;
        let from = self.marker.end;
        if !matches!(source.as_bytes().get(from), Some(b' ' | b'\t')) {
            return Some((from..from, " [ ] "));
        }
        let spaces = source[from..]
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        Some((from + spaces..from + spaces, "[ ] "))
    }
}

impl Region {
    pub fn continue_list(
        &self,
        source: &str,
        caret: usize,
    ) -> Option<(Range<usize>, String, usize)> {
        let item = self.list.as_ref()?;
        let prefix = item.line_prefix(source)?;
        let line_start = item.marker.start - prefix.len();
        let marker_line_end = line_end(source, item.marker.start);
        if !source.is_char_boundary(caret) {
            return None;
        }
        if self.source.is_empty() {
            let rest = source.get(marker_line_end..)?;
            if !prefix.trim().is_empty() || !rest.trim().is_empty() {
                return None;
            }
            let replacement = if rest.is_empty() && line_start > 0 {
                newline(source)
            } else {
                ""
            };
            let caret = line_start + replacement.len() + rest.len();
            return Some((line_start..marker_line_end, replacement.to_owned(), caret));
        }

        let line_end = line_end(source, self.source.end);
        if item.source.end > line_end {
            return None;
        }
        let at = if caret == self.source.end {
            line_end
        } else {
            let prefix = &self.runs.first()?.text;
            (self.source.end <= marker_line_end
                && caret >= self.source.start
                && caret < self.source.end
                && self.text.strip_prefix(prefix) == source.get(self.source.clone()))
            .then_some(caret)?
        };
        let ending = if line_end < source.len() {
            newline(&source[line_end..])
        } else {
            newline(source)
        };
        let marker = &source[item.marker.clone()];
        let marker = if marker.len() == 1 {
            marker.to_owned()
        } else {
            let (number, delimiter) = marker.split_at(marker.len() - 1);
            let next = (number.parse::<u32>().ok()? + 1).min(999_999_999);
            format!("{next:0width$}{delimiter}", width = number.len())
        };
        let task = if item.task.is_some() { "[ ] " } else { "" };
        let inserted = format!("{ending}{prefix}{marker} {task}");
        let next = at + inserted.len();
        Some((at..at, inserted, next))
    }
}

fn line_end(source: &str, from: usize) -> usize {
    source[from..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| from + offset)
}
