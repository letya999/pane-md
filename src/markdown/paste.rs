use crate::{Bytes, newline, normalize_newlines, utf16_byte};

use super::{Edit, FormatPatch, Markdown, Region, Run, Style};

impl Markdown {
    /// Replaces rendered text with literal text. An empty input deletes the selection.
    pub fn paste_text(
        &self,
        source: &str,
        selection: &[(usize, Bytes)],
        input: &str,
    ) -> Option<FormatPatch> {
        if input
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
        {
            return None;
        }
        let (left, first) = selection.first()?;
        let (right, last) = selection.last()?;
        if first.start > first.end
            || last.start > last.end
            || input.is_empty() && left == right && first.is_empty()
        {
            return None;
        }
        let a = self.regions.get(*left)?;
        let b = self.regions.get(*right)?;
        if a.mermaid || b.mermaid || matches!(a.kind, "html") || matches!(b.kind, "html") {
            return None;
        }

        let joins_items = left != right && a.list.is_some() && b.list.is_some();
        let across_cells =
            left != right && a.kind == "table" && b.kind == "table" && a.owner == b.owner;
        let start = if joins_items {
            first.start.max(a.list_prefix())
        } else {
            first.start
        };
        let end = if joins_items {
            last.end.max(b.list_prefix())
        } else {
            last.end
        };
        let inserted = normalize_newlines(input, "\n");
        let before = a.text.get(..utf16_byte(&a.text, start)?)?;
        let after = b.text.get(utf16_byte(&b.text, end)?..)?;
        let visible_after = visible(after);
        let expected = format!("{}{inserted}{visible_after}", visible(before));
        let local = (left == right)
            .then(|| a.edit_range(first.clone()))
            .flatten();
        let ending = match source.get(a.source.start..)?.find(['\r', '\n']) {
            Some(at) => newline(source.get(a.source.start + at..)?),
            None => newline(source),
        };

        let (raw, paragraphs) = match local.as_ref().and_then(|(_, run)| run.edit) {
            Some(Edit::Code) => (normalize_newlines(&inserted, ending), false),
            Some(Edit::InlineCode) => {
                if inserted.contains(['`', '\n']) {
                    return None;
                }
                (inserted, false)
            }
            _ => {
                let paragraphs = a.plain_paragraph()
                    && b.plain_paragraph()
                    && local
                        .as_ref()
                        .is_none_or(|(_, run)| run.style == Style::default() && run.link.is_none())
                    && inserted.split("\n\n").all(|part| {
                        !part.is_empty() && !part.starts_with('\n') && !part.ends_with('\n')
                    });
                let body = |part: &str| {
                    part.split('\n')
                        .map(literal_line)
                        .collect::<Vec<_>>()
                        .join("<br>")
                };
                let raw = if paragraphs {
                    inserted
                        .split("\n\n")
                        .map(body)
                        .collect::<Vec<_>>()
                        .join(&ending.repeat(2))
                } else {
                    body(&inserted)
                };
                (raw, paragraphs)
            }
        };

        let mut comments = Vec::new();
        let (starts, ends) = if let Some((range, _)) = local {
            let whole_run = input.is_empty() && a.inline_container();
            let left = whole_run
                .then(|| a.splice_bounds(first.start, true))
                .flatten();
            let right = whole_run
                .then(|| a.splice_bounds(first.end, false))
                .flatten();
            (
                [range.start, left.map_or(range.start, |bounds| bounds[1])],
                [range.end, right.map_or(range.end, |bounds| bounds[1])],
            )
        } else if left == right {
            if !a.inline_container() {
                return None;
            }
            (a.splice_bounds(start, true)?, b.splice_bounds(end, false)?)
        } else {
            if right.checked_sub(*left)? + 1 != selection.len() {
                return None;
            }
            for (relative, (index, span)) in selection.iter().enumerate() {
                let item = self.regions.get(*index)?;
                let length = item.text.encode_utf16().count();
                if *index != *left + relative
                    || span.start > span.end
                    || span.end > length
                    || (relative > 0 && span.start != 0)
                    || (*index != *right && span.end != length)
                {
                    return None;
                }
                if across_cells {
                    if item.kind != "table"
                        || item.owner != a.owner
                        || item.runs.is_empty()
                        || relative > 0 && self.regions[index - 1].source.end > item.source.start
                    {
                        return None;
                    }
                } else if joins_items {
                    let list = item.list.as_ref()?;
                    let first_item = a.list.as_ref()?;
                    if item.kind != "list"
                        || item.quote
                        || item.level != 0
                        || item.indent < a.indent
                        || item.owner != a.owner
                        || (relative == 0 || index == right) && item.indent != a.indent
                        || item.indent == a.indent
                            && source.as_bytes().get(list.marker.end.checked_sub(1)?)
                                != source.as_bytes().get(first_item.marker.end.checked_sub(1)?)
                    {
                        return None;
                    }
                    if relative > 0
                        && !source
                            .get(self.regions[index - 1].source.end..list.marker.start)?
                            .trim()
                            .is_empty()
                    {
                        return None;
                    }
                } else if !(item.plain_paragraph()
                    || item.kind == "heading" && item.indent == 0 && !item.quote)
                    && !(item.kind == "list"
                        && item.level == 0
                        && !item.quote
                        && item.list.as_ref().is_some_and(|list| {
                            source
                                .get(..list.marker.start)
                                .is_some_and(|prefix| prefix.rsplit('\n').next() == Some(""))
                        }))
                {
                    return None;
                } else if relative > 0 {
                    let previous = &self.regions[index - 1];
                    if item.owner != previous.owner + 1 {
                        return None;
                    }
                    let gap_end = if let Some(list) = &item.list {
                        let prefix = source.get(list.marker.start..item.source.start)?;
                        if prefix.len() > 32 || prefix.contains(['\r', '\n', '<']) {
                            return None;
                        }
                        list.marker.start
                    } else {
                        item.source.start
                    };
                    self.comments_in_gap(source, previous.source.end..gap_end, &mut comments)?;
                }
            }
            (a.splice_bounds(start, true)?, b.splice_bounds(end, false)?)
        };

        let signature = |region: &Region| {
            (
                region.kind,
                region.level,
                region.indent,
                region.quote,
                region.row,
                region.cell,
                region.list.is_some(),
            )
        };
        let before_decorations = decorations(a)
            .take(before.chars().count())
            .collect::<Vec<_>>();
        let after_decorations = decorations(b)
            .skip(b.text.chars().count() - after.chars().count())
            .collect::<Vec<_>>();
        let tail = &self.regions[right + 1..];
        for (start_index, &start) in starts.iter().enumerate() {
            if start_index == 1 && start == starts[0] {
                continue;
            }
            for (end_index, &end) in ends.iter().enumerate() {
                if end_index == 1 && end == ends[0] {
                    continue;
                }
                let mut cursor = start;
                if comments.iter().any(|comment| {
                    let protected = source
                        .get(cursor..comment.start)
                        .is_none_or(|s| s.contains("<!--"));
                    cursor = comment.end;
                    protected
                }) || source.get(cursor..end).is_none_or(|s| s.contains("<!--"))
                {
                    continue;
                }
                let mut range = start..end;
                let (mut replacement, caret) = a.materialize_cell(source, &raw);
                if across_cells {
                    let mut cursor = a.source.end;
                    for cell in &self.regions[left + 1..=*right] {
                        replacement.push_str(source.get(cursor..cell.source.start)?);
                        cursor = cell.source.end;
                    }
                } else if !comments.is_empty() {
                    replacement.push_str(source.get(end..b.source.end)?);
                    range.end = b.source.end;
                    for span in &comments {
                        if !replacement.is_empty() || !expected.is_empty() {
                            replacement.push_str(ending);
                            replacement.push_str(ending);
                        }
                        replacement.push_str(source.get(span.clone())?);
                    }
                }
                let mut next = source.to_owned();
                next.replace_range(range.clone(), &replacement);
                let mut parsed = Markdown::parse(&next);
                if !comments.is_empty()
                    && !self
                        .review_metadata
                        .iter()
                        .map(|span| &source[span.clone()])
                        .eq(parsed
                            .review_metadata
                            .iter()
                            .map(|span| &next[span.clone()]))
                {
                    continue;
                }
                if tail
                    .last()
                    .is_some_and(|region| region.source.is_empty() && region.plain_paragraph())
                {
                    parsed.push_empty_paragraph(next.len());
                }
                let Some(end) = parsed.regions.len().checked_sub(tail.len()) else {
                    continue;
                };
                let Some(changed) = parsed.regions.get(*left..end) else {
                    continue;
                };
                let preserves_text = match changed.first().zip(changed.last()) {
                    Some((head, back)) if across_cells => {
                        let first = &expected[..expected.len() - visible_after.len()];
                        changed.len() == selection.len()
                            && changed
                                .iter()
                                .zip(&self.regions[*left..=*right])
                                .all(|(edited, prior)| signature(edited) == signature(prior))
                            && changed[1..changed.len() - 1]
                                .iter()
                                .all(|cell| cell.text.is_empty())
                            && [first, first.trim_end_matches([' ', '\t'])]
                                .contains(&visible(&head.text).as_str())
                            && [
                                visible_after.as_str(),
                                visible_after.trim_start_matches([' ', '\t']),
                            ]
                            .contains(&visible(&back.text).as_str())
                            && decorations(head).take(before_decorations.len()).eq(
                                before_decorations
                                    .iter()
                                    .take(head.text.chars().count())
                                    .copied(),
                            )
                            && decorations(back)
                                .skip(
                                    back.text
                                        .chars()
                                        .count()
                                        .saturating_sub(after_decorations.len()),
                                )
                                .eq(after_decorations
                                    .iter()
                                    .skip(
                                        after_decorations
                                            .len()
                                            .saturating_sub(back.text.chars().count()),
                                    )
                                    .copied())
                    }
                    Some((head, back)) => {
                        let offset = back
                            .text
                            .chars()
                            .count()
                            .saturating_sub(after_decorations.len());
                        (paragraphs || changed.len() == 1)
                            && changed
                                .iter()
                                .all(|region| signature(region) == signature(a))
                            && changed
                                .iter()
                                .map(|region| visible(&region.text))
                                .collect::<Vec<_>>()
                                .join("\n\n")
                                == expected
                            && decorations(head)
                                .take(before_decorations.len())
                                .eq(before_decorations.iter().copied())
                            && decorations(back)
                                .skip(offset)
                                .eq(after_decorations.iter().copied())
                    }
                    None => expected.is_empty() && a.plain_paragraph(),
                };
                if !preserves_text {
                    continue;
                }
                if self.regions[..*left]
                    .iter()
                    .chain(tail)
                    .zip(parsed.regions[..*left].iter().chain(&parsed.regions[end..]))
                    .any(|(previous, next)| {
                        signature(previous) != signature(next)
                            || !decorations(previous)
                                .skip(renumbered_prefix(previous, joins_items))
                                .eq(decorations(next).skip(renumbered_prefix(next, joins_items)))
                    })
                {
                    continue;
                }
                let caret = range.start + caret;
                return Some(FormatPatch::new(range, replacement, caret));
            }
        }
        None
    }

    fn comments_in_gap(&self, source: &str, gap: Bytes, out: &mut Vec<Bytes>) -> Option<()> {
        let mut start = gap.start;
        let index = self
            .review_metadata
            .partition_point(|span| span.end <= start);
        for span in self.review_metadata[index..]
            .iter()
            .take_while(|span| span.start < gap.end)
        {
            if span.end > gap.end
                || !source.get(start..span.start)?.trim().is_empty()
                || !source.get(span.clone())?.trim_end().ends_with("-->")
            {
                return None;
            }
            out.push(span.clone());
            start = span.end;
        }
        source.get(start..gap.end)?.trim().is_empty().then_some(())
    }
}

impl Region {
    /// True when removing this text also requires removing its inline delimiters.
    pub fn removes_inline_run(&self, selection: &Bytes) -> bool {
        self.inline_container()
            && self.edit_range(selection.clone()).is_some_and(|(_, run)| {
                selection.len() == run.units.len()
                    && (run.style
                        != Style {
                            bold: self.row == Some(0),
                            ..Style::default()
                        }
                        || run.link.is_some())
            })
    }

    fn inline_container(&self) -> bool {
        matches!(
            self.kind,
            "paragraph" | "heading" | "list" | "quote" | "table"
        )
    }

    fn list_prefix(&self) -> usize {
        self.runs
            .first()
            .filter(|run| self.list.is_some() && run.edit.is_none())
            .map_or(0, |run| run.units.len())
    }

    fn plain_paragraph(&self) -> bool {
        self.kind == "paragraph" && self.indent == 0 && !self.quote
    }

    fn splice_bounds(&self, at: usize, left: bool) -> Option<[usize; 2]> {
        utf16_byte(&self.text, at)?;
        let units: Vec<_> = self.runs.iter().flat_map(|run| &run.units).collect();
        let before = if at == self.list_prefix() {
            None
        } else {
            at.checked_sub(1).and_then(|at| units.get(at))
        };
        let after = units.get(at);
        if let (Some(a), Some(b)) = (before, after)
            && (a.is_none() || b.is_none() || a == b)
        {
            return None;
        }
        let low = before.map_or(Some(self.source.start), |unit| {
            unit.as_ref().map(|span| span.end)
        })?;
        let high = after.map_or(Some(self.source.end), |unit| {
            unit.as_ref().map(|span| span.start)
        })?;
        (low <= high).then_some(if left { [high, low] } else { [low, high] })
    }
}

fn visible(text: &str) -> String {
    text.replace(['\r', '\u{000b}'], "\n")
}

pub(super) fn decorations(
    region: &Region,
) -> impl Iterator<Item = (char, Style, Option<&str>, Option<&str>)> {
    region.runs.iter().flat_map(
        |Run {
             text,
             style,
             link,
             image,
             ..
         }| {
            text.chars()
                .map(move |ch| (ch, *style, link.as_deref(), image.as_deref()))
        },
    )
}

fn renumbered_prefix(region: &Region, joins_items: bool) -> usize {
    if joins_items
        && region
            .text
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
    {
        region.list_prefix()
    } else {
        0
    }
}

pub(super) fn literal_line(line: &str) -> String {
    let content_start = line.len() - line.trim_start_matches([' ', '\t']).len();
    let content_end = line.trim_end_matches([' ', '\t']).len();
    let mut escaped = String::new();
    for (index, ch) in line.char_indices() {
        if index < content_start || index >= content_end {
            match ch {
                ' ' => {
                    escaped.push_str("&#32;");
                    continue;
                }
                '\t' => {
                    escaped.push_str("&#9;");
                    continue;
                }
                _ => {}
            }
        }
        let starts_list = index == content_start && matches!(ch, '+' | '-')
            || matches!(ch, '.' | ')')
                && line[content_start..index]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit());
        if starts_list
            || matches!(
                ch,
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '~' | '|' | '#' | '='
            )
        {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}
