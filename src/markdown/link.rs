use crate::{Bytes, utf16_byte};
use comrak::nodes::{Node, NodeValue};

use super::{Builder, Edit, FormatError, FormatPatch, Markdown, SourceLink, paste, split_row};

pub struct LinkEdit<'a> {
    existing: Option<&'a SourceLink>,
    label: Bytes,
}

impl LinkEdit<'_> {
    pub fn target(&self) -> Option<&str> {
        self.existing.map(|link| link.target.as_str())
    }
}

impl Builder<'_> {
    pub(super) fn link_span(&self, node: Node<'_>) -> Bytes {
        self.table_link_span(node)
            .unwrap_or_else(|| self.span(node))
    }

    fn table_link_span(&self, node: Node<'_>) -> Option<Bytes> {
        let cell = node
            .ancestors()
            .find(|parent| matches!(parent.data().value, NodeValue::TableCell))?;
        let row = cell.parent()?;
        let column = row
            .children()
            .position(|candidate| candidate.same_node(cell))?;
        let depth = row
            .ancestors()
            .filter(|parent| matches!(parent.data().value, NodeValue::BlockQuote))
            .count();
        let cells = split_row(self.source, self.table_row_span(row, depth));
        let span = cells.get(column)?;
        let raw = self.source.get(span.clone())?;
        let content = raw.trim_start();
        let padding = raw.len() - content.len();
        let baseline = cell.data().sourcepos.start.column + padding;
        let pos = node.data().sourcepos;
        let start = pos.start.column.checked_sub(baseline)?;
        let end = (pos.end.column + 1).checked_sub(baseline)?;
        let offset = |logical| pipe_offset(content, logical).map(|at| span.start + padding + at);
        let range = offset(start)?..offset(end)?;
        self.source.get(range.clone())?;
        Some(range)
    }
}

// Comrak removes the backslash of an escaped pipe before parsing the cell's inline content.
fn pipe_offset(source: &str, expected: usize) -> Option<usize> {
    let mut logical = 0;
    let mut escaped = false;
    for (at, byte) in source.bytes().enumerate() {
        if logical == expected {
            return Some(at);
        }
        if byte == b'\\' && !escaped && source.as_bytes().get(at + 1) == Some(&b'|') {
            escaped = true;
            continue;
        }
        logical += 1;
        escaped = byte == b'\\' && !escaped;
    }
    (logical == expected).then_some(source.len())
}

impl SourceLink {
    pub fn shift_after(&mut self, at: usize, delta: isize) {
        if self.source.start >= at {
            let shift = |span: &mut Bytes| {
                span.start = span.start.saturating_add_signed(delta);
                span.end = span.end.saturating_add_signed(delta);
            };
            shift(&mut self.source);
            if let Some(label) = &mut self.label {
                shift(label);
            }
        }
    }
}

impl Markdown {
    pub fn shift_links_after(&mut self, at: usize, delta: isize) {
        for link in &mut self.source_links {
            link.shift_after(at, delta);
        }
        for (offset, _) in &mut self.nested_images {
            if *offset >= at {
                *offset = offset.saturating_add_signed(delta);
            }
        }
    }

    pub fn link_edit(&self, index: usize, selection: Bytes) -> Result<LinkEdit<'_>, FormatError> {
        let region = self.regions.get(index).ok_or(FormatError::SelectText)?;
        let range = if selection.is_empty() {
            utf16_byte(&region.text, selection.start).ok_or(FormatError::SelectText)?;
            let mut units = region.runs.iter().flat_map(|run| &run.units);
            let at = selection.start;
            let before = (at > 0).then(|| units.nth(at - 1)).flatten();
            units
                .next()
                .into_iter()
                .chain(before)
                .filter_map(Option::as_ref)
                .find(|span| self.source_link(span.start).is_some())
                .cloned()
        } else {
            region.source_selection(selection.clone())
        };
        let raw = range.ok_or(FormatError::SelectText)?;
        if let Some(link) = self.source_link(raw.start) {
            let label = link.label.clone().ok_or(FormatError::LinkSource)?;
            if raw.end > link.source.end
                || link.source.start < region.source.start
                || link.source.end > region.source.end
            {
                return Err(FormatError::LinkSource);
            }
            return Ok(LinkEdit {
                existing: Some(link),
                label,
            });
        }
        if selection.is_empty() {
            return Err(FormatError::SelectText);
        }
        if self
            .source_links
            .iter()
            .any(|link| overlaps(&link.source, &raw))
        {
            return Err(FormatError::LinkSource);
        }
        let (label, run) = region
            .edit_range(selection)
            .ok_or(FormatError::Unsupported)?;
        if !matches!(run.edit, Some(Edit::Text | Edit::Cell)) {
            return Err(FormatError::Unsupported);
        }
        Ok(LinkEdit {
            existing: None,
            label,
        })
    }

    /// A blank address removes one link occurrence. Reference definitions are left in place.
    pub fn change_link(
        &self,
        source: &str,
        index: usize,
        selection: Bytes,
        address: &str,
    ) -> Result<FormatPatch, FormatError> {
        use FormatError::{InvalidLink, LinkOneLine, LinkSource};
        let edit = self.link_edit(index, selection)?;
        let raw_label = source.get(edit.label.clone()).ok_or(LinkSource)?;
        let address = address.trim();
        let wrapped = edit
            .existing
            .map_or(edit.label.clone(), |link| link.source.clone());
        if !address.is_empty() && edit.target() == Some(address) {
            return Ok(FormatPatch::new(
                wrapped.start..wrapped.start,
                "",
                edit.label.start,
            ));
        }
        let automatic = edit.existing.and_then(|link| link.automatic.as_deref());
        let title = edit.existing.map_or("", |link| link.title.as_str());
        let target = if address.is_empty() {
            if edit.existing.is_none() {
                return Err(LinkOneLine);
            }
            None
        } else {
            if address.chars().any(char::is_control) || address.contains(['<', '>']) {
                return Err(LinkOneLine);
            }
            crate::links::destination(address).map_err(|_| InvalidLink)?;
            Some(
                address
                    .replace(' ', "%20")
                    .replace('(', "%28")
                    .replace(')', "%29")
                    .replace('\\', "%5C")
                    .replace('|', "%7C"),
            )
        };
        let mut patch = if let Some(target) = target.as_deref() {
            let url = target.replace('&', "&amp;");
            if edit.existing.is_some()
                && automatic.is_none()
                && let Some(dest) = destination_span(source, edit.label.end, wrapped.end)
            {
                FormatPatch::new(dest, url, edit.label.start)
            } else {
                let label = automatic.map_or_else(|| raw_label.to_owned(), paste::literal_line);
                let mut text = format!("[{label}]({url}");
                if !title.is_empty() {
                    let escaped = title
                        .replace('&', "&amp;")
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('|', "&#124;");
                    text.push_str(&format!(" \"{escaped}\""));
                }
                text.push(')');
                let mut patch = FormatPatch::new(wrapped.clone(), text, wrapped.start + 1);
                patch.caret.end += label.len();
                patch
            }
        } else {
            let label = automatic.map_or_else(
                || raw_label.to_owned(),
                |value| no_autolink(&paste::literal_line(value)),
            );
            FormatPatch::new(wrapped.clone(), label, wrapped.start)
        };
        if !self.valid_link_patch(source, index, &patch, &wrapped, target.as_deref(), title) {
            if target.is_some()
                || automatic.is_some()
                || !(raw_label.contains("://")
                    || raw_label.contains('@')
                    || raw_label.contains("www."))
            {
                return Err(LinkSource);
            }
            patch.text = no_autolink(raw_label);
            if !self.valid_link_patch(source, index, &patch, &wrapped, None, title) {
                return Err(LinkSource);
            }
        }
        Ok(patch)
    }

    fn valid_link_patch(
        &self,
        source: &str,
        index: usize,
        patch: &FormatPatch,
        before: &Bytes,
        target: Option<&str>,
        title: &str,
    ) -> bool {
        let mut next = source.to_owned();
        next.replace_range(patch.range.clone(), &patch.text);
        let parsed = Markdown::parse(&next);
        let old = &self.regions[index];
        let Some(region) = parsed.regions.get(index) else {
            return false;
        };
        let appearance = |(ch, style, _, image)| (ch, style, image);
        let virtual_tail = self.regions.last().is_some_and(|r| {
            r.source.is_empty() && r.source.end == source.len() && r.text.is_empty()
        });
        if (self.regions.len() != parsed.regions.len()
            && !(virtual_tail && self.regions.len() == parsed.regions.len() + 1))
            || (old.kind, old.level, old.indent, old.row, old.cell)
                != (
                    region.kind,
                    region.level,
                    region.indent,
                    region.row,
                    region.cell,
                )
            || !paste::decorations(old)
                .map(appearance)
                .eq(paste::decorations(region).map(appearance))
        {
            return false;
        }
        let delta = patch.text.len() as isize - patch.range.len() as isize;
        let after = before.start..before.end.saturating_add_signed(delta);
        let mut affected = parsed
            .source_links
            .iter()
            .filter(|link| !link.image && overlaps(&link.source, &after));
        if let Some(target) = target
            && !affected.next().is_some_and(|link| {
                link.source == after && link.target == target && link.title == title
            })
        {
            return false;
        }
        if affected.next().is_some() {
            return false;
        }
        let before_links = self
            .source_links
            .iter()
            .filter(|link| !link.image && !overlaps(&link.source, before))
            .map(|link| {
                let span = if link.source.start >= patch.range.end {
                    link.source.start.saturating_add_signed(delta)
                        ..link.source.end.saturating_add_signed(delta)
                } else {
                    link.source.clone()
                };
                (span, link.target.as_str(), link.title.as_str())
            });
        before_links.eq(parsed
            .source_links
            .iter()
            .filter(|link| !link.image && !overlaps(&link.source, &after))
            .map(|link| {
                (
                    link.source.clone(),
                    link.target.as_str(),
                    link.title.as_str(),
                )
            }))
    }
}

fn destination_span(source: &str, label_end: usize, link_end: usize) -> Option<Bytes> {
    let raw = source
        .get(label_end..link_end)?
        .strip_prefix("](")?
        .strip_suffix(')')?;
    let trimmed = raw.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    let at = label_end + 2 + raw.len() - trimmed.len();
    if let Some(angle) = trimmed.strip_prefix('<') {
        let mut escaped = false;
        for (i, ch) in angle.char_indices() {
            if ch == '>' && !escaped {
                return Some(at + 1..at + 1 + i);
            }
            escaped = ch == '\\' && !escaped;
        }
        None
    } else {
        let len = trimmed
            .find(|ch: char| ch.is_ascii_whitespace())
            .unwrap_or(trimmed.len());
        Some(at..at + len)
    }
}

fn no_autolink(label: &str) -> String {
    let mut plain = String::new();
    let mut escaped = false;
    for ch in label.chars() {
        if !escaped && matches!(ch, ':' | '.' | '@') {
            plain.push('\\');
        }
        plain.push(ch);
        escaped = ch == '\\' && !escaped;
    }
    plain
}

fn overlaps(a: &Bytes, b: &Bytes) -> bool {
    a.start < b.end && b.start < a.end
}
