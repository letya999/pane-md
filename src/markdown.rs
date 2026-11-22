mod format;
pub use format::{BlockFormat, FormatError, FormatPatch, InlineFormat};
mod link;
mod list;
mod paste;
mod table;
pub use list::ListItem;
pub use table::{TableEdit, table_patch};

use std::ops::Range;

use comrak::{
    Arena, Options,
    nodes::{ListType, Node, NodeValue, Sourcepos, TableAlignment},
    parse_document,
};

use crate::{Bytes, newline, normalize_newlines, utf16_byte};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub mark: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Text,
    Cell,
    Code,
    InlineCode,
}

#[derive(Clone, Default)]
pub struct Run {
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
    pub image: Option<String>,
    pub units: Vec<Option<Bytes>>,
    pub edit: Option<Edit>,
    /// Source insertion point for a code line with no text units.
    pub empty_at: Option<usize>,
}

#[derive(Default)]
pub struct Region {
    pub runs: Vec<Run>,
    pub text: String,
    pub source: Bytes,
    pub owner: usize,
    pub heading: String,
    pub slug: String,
    pub kind: &'static str,
    pub row: Option<usize>,
    pub cell: Option<usize>,
    missing_cells: usize,
    pub level: u8,
    pub indent: usize,
    pub quote: bool,
    pub mermaid: bool,
    pub list: Option<ListItem>,
}

impl Region {
    pub fn source_selection(&self, selection: Range<usize>) -> Option<Bytes> {
        if selection.is_empty()
            || utf16_byte(&self.text, selection.start).is_none()
            || utf16_byte(&self.text, selection.end).is_none()
        {
            return None;
        }
        let mut units = self.runs.iter().flat_map(|run| run.units.iter());
        let first = units.nth(selection.start)?.as_ref()?;
        let last = if selection.len() == 1 {
            first
        } else {
            units.nth(selection.len() - 2)?.as_ref()?
        };
        (first.start <= last.end).then_some(first.start..last.end)
    }

    fn edit_range(&self, selection: Range<usize>) -> Option<(Bytes, &Run)> {
        if selection.start > selection.end {
            return None;
        }
        utf16_byte(&self.text, selection.start)?;
        utf16_byte(&self.text, selection.end)?;
        let mut at = 0;
        for run in &self.runs {
            let end = at + run.units.len();
            let span = at..end;
            at = end;
            if selection.start < span.start || selection.end > span.end {
                continue;
            }
            if run.edit.is_none() {
                continue;
            }
            let start = selection.start - span.start;
            let finish = selection.end - span.start;
            let range = if start == finish {
                let left = start
                    .checked_sub(1)
                    .and_then(|i| run.units[i].as_ref())
                    .map(|r| r.end);
                let right = run
                    .units
                    .get(start)
                    .and_then(Option::as_ref)
                    .map(|r| r.start);
                let point = right
                    .or(left)
                    .or(run.empty_at)
                    .or_else(|| run.text.is_empty().then_some(self.source.start))?;
                point..point
            } else {
                let units = run.units.get(start..finish)?;
                let first = units.first()?.as_ref()?;
                let last = units.last()?.as_ref()?;
                if units.windows(2).any(|pair| match (&pair[0], &pair[1]) {
                    (Some(a), Some(b)) => a != b && a.end != b.start,
                    _ => true,
                }) {
                    return None;
                }
                // An entity may represent more than one scalar.
                if start > 0
                    && run.units[start - 1].as_ref() == Some(first)
                    && run.units[start - 1] == run.units[start]
                    && first.end - first.start > 1
                    && utf16_byte(&run.text, start).is_some()
                {
                    return None;
                }
                if run
                    .units
                    .get(finish)
                    .is_some_and(|next| next.as_ref() == Some(last))
                {
                    return None;
                }
                first.start..last.end
            };
            return Some((range, run));
        }
        None
    }

    pub fn patch(
        &self,
        source: &str,
        selection: Range<usize>,
        inserted: &str,
    ) -> Option<(Bytes, String)> {
        let (range, run) = self.edit_range(selection)?;
        let edit = run.edit?;
        if (edit == Edit::Cell
            || edit == Edit::InlineCode
            || self.level != 0
            || matches!(self.kind, "list" | "quote" | "footnote"))
            && inserted.contains(['\r', '\n'])
        {
            return None;
        }
        if self.missing_cells != 0 {
            let raw = paste::literal_line(inserted);
            let (text, _) = self.materialize_cell(source, &raw);
            return Some((range, text));
        }
        if edit == Edit::InlineCode && inserted.contains('`') {
            return None;
        }
        if edit == Edit::Code && inserted.contains(['`', '~', '\n', '\r']) {
            // shortcut: structural code edits belong in source mode until fence/indent rebasing is implemented.
            return None;
        }
        let mut replacement = String::new();
        for ch in inserted.chars() {
            if matches!(edit, Edit::Text | Edit::Cell)
                && matches!(
                    ch,
                    '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '~' | '|' | '#' | '='
                )
            {
                replacement.push('\\');
            }
            replacement.push(ch);
        }
        let ending = newline(source);
        let mut replacement = if edit == Edit::Text && self.kind == "paragraph" {
            normalize_newlines(&replacement, &ending.repeat(2))
        } else {
            normalize_newlines(&replacement, ending)
        };
        if replacement.contains('\u{000b}') {
            if edit != Edit::Text || self.level != 0 {
                return None;
            }
            let indent = match &self.list {
                Some(item) => item.continuation_indent(source)?,
                None if self.kind == "paragraph" => String::new(),
                None => return None,
            };
            replacement = replacement.replace('\u{000b}', &format!("  {ending}{indent}"));
        }
        let replacement = if self.list.is_some()
            && self.source.is_empty()
            && !replacement.is_empty()
            && source[..range.start]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_whitespace())
        {
            format!(" {replacement}")
        } else {
            replacement
        };
        Some((range, replacement))
    }
}

pub enum Layout {
    Text(usize),
    Table {
        rows: Vec<Vec<usize>>,
        alignment: Vec<TableAlignment>,
        source: Bytes,
    },
    Rule,
}

pub struct Owner {
    pub source: Bytes,
    pub after: usize,
}

pub struct SourceLink {
    pub source: Bytes,
    pub target: String,
    pub image: bool,
    pub(crate) footnote: bool,
    label: Option<Bytes>,
    title: String,
    automatic: Option<String>,
}

#[derive(Default)]
pub struct Markdown {
    pub source_links: Vec<SourceLink>,
    pub(crate) nested_images: Vec<(usize, String)>,
    pub regions: Vec<Region>,
    pub layout: Vec<Layout>,
    pub owners: Vec<Owner>,
    pub records: Vec<crate::review::Record>,
    pub(crate) review_metadata: Vec<Bytes>,
    pub diagnostics: Vec<crate::review::Diagnostic>,
}

pub struct Lines {
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl Lines {
    pub fn new(source: &str) -> Self {
        let mut starts = vec![0];
        let mut ends = Vec::new();
        let mut bytes = source.bytes().enumerate().peekable();
        while let Some((at, byte)) = bytes.next() {
            if matches!(byte, b'\n' | b'\r') {
                ends.push(at);
                let lf = byte == b'\r' && bytes.peek().is_some_and(|&(_, b)| b == b'\n');
                if lf {
                    bytes.next();
                }
                starts.push(at + 1 + usize::from(lf));
            }
        }
        ends.push(source.len());
        Self { starts, ends }
    }

    pub fn position(&self, byte: usize) -> (usize, usize) {
        let index = self
            .starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1);
        (index + 1, byte - self.starts[index] + 1)
    }

    pub fn offset(&self, line: usize, column: usize) -> Option<usize> {
        let index = line.checked_sub(1)?;
        let at = self
            .starts
            .get(index)?
            .checked_add(column.checked_sub(1)?)?;
        (at <= self.ends[index]).then_some(at)
    }

    pub fn span(&self, pos: Sourcepos) -> Option<Bytes> {
        let first = pos.start.line.checked_sub(1)?;
        let last = pos.end.line.checked_sub(1)?;
        let start =
            (self.starts.get(first)? + pos.start.column.saturating_sub(1)).min(self.ends[first]);
        let end = (self.starts.get(last)? + pos.end.column).min(self.ends[last]);
        (start <= end).then_some(start..end)
    }

    fn end_of_line(&self, offset: usize) -> usize {
        self.ends[self.position(offset).0 - 1]
    }
}

fn inline_text(node: Node<'_>) -> String {
    node.descendants()
        .filter_map(|child| match &child.data().value {
            NodeValue::Text(text) => Some(text.to_string()),
            NodeValue::Code(code) => Some(code.literal.clone()),
            NodeValue::SoftBreak | NodeValue::LineBreak => Some(" ".into()),
            _ => None,
        })
        .collect()
}

fn block_kind(value: &NodeValue) -> &'static str {
    use NodeValue::*;
    match value {
        Heading(_) => "heading",
        List(_) => "list",
        BlockQuote => "quote",
        Table(_) => "table",
        CodeBlock(_) => "code",
        FootnoteDefinition(_) => "footnote",
        HtmlBlock(_) => "html",
        _ => "paragraph",
    }
}

impl Markdown {
    pub fn parse(source: &str) -> Self {
        let arena = Arena::new();
        let mut options = Options::default();
        options.extension.table = true;
        options.extension.autolink = true;
        options.extension.tasklist = true;
        options.extension.strikethrough = true;
        options.extension.footnotes = true;
        options.extension.highlight = true;
        options.extension.front_matter_delimiter = Some("---".into());
        options.parse.leave_footnote_definitions = true;
        let root = parse_document(&arena, source, &options);
        let mut b = Builder {
            source,
            lines: Lines::new(source),
            doc: Self::default(),
            headings: Vec::new(),
            slugs: comrak::Anchorizer::new(),
        };
        for node in root.children() {
            let value = &node.data().value;
            let mut span = b.span(node);
            if matches!(value, NodeValue::FrontMatter(_)) {
                continue;
            }
            if let NodeValue::HtmlBlock(html) = value
                && html.literal.trim_start().starts_with("<!--")
            {
                crate::review::load(&mut b.doc, source, &b.lines, span);
                continue;
            }
            if let NodeValue::Heading(heading) = value {
                b.headings.retain(|&(level, _)| level < heading.level);
                let title = inline_text(node);
                b.headings.push((heading.level, title));
            }
            while span.end > span.start && matches!(source.as_bytes()[span.end - 1], b'\r' | b'\n')
            {
                span.end -= 1;
            }
            let owner = b.doc.owners.len();
            b.doc.owners.push(Owner {
                after: b.lines.end_of_line(span.end),
                source: span,
            });
            b.block(node, owner, block_kind(value), 0, false, String::new());
        }
        for node in root.descendants() {
            let (target, image, footnote, title) = match &node.data().value {
                NodeValue::Link(link) => (link.url.clone(), false, false, link.title.clone()),
                NodeValue::Image(image) => (image.url.clone(), true, false, String::new()),
                NodeValue::FootnoteReference(note) => {
                    (format!("#fn-{}", note.name), false, true, String::new())
                }
                _ => continue,
            };
            let span = b.link_span(node);
            if !span.is_empty() {
                let label = (!image && !footnote)
                    .then(|| {
                        let raw = &source[span.clone()];
                        if raw.starts_with('[') {
                            let end = node
                                .last_child()
                                .map_or(span.start + 1, |child| b.link_span(child).end);
                            (end < span.end && source.as_bytes().get(end) == Some(&b']'))
                                .then_some(span.start + 1..end)
                        } else if raw.starts_with('<') && raw.ends_with('>') {
                            Some(span.start + 1..span.end - 1)
                        } else {
                            Some(span.clone())
                        }
                    })
                    .flatten();
                let automatic = label
                    .as_ref()
                    .filter(|_| !source[span.clone()].starts_with('['))
                    .map(|_| inline_text(node));
                b.doc.source_links.push(SourceLink {
                    source: span,
                    target,
                    image,
                    footnote,
                    label,
                    title,
                    automatic,
                });
            }
        }
        b.doc.source_links.sort_unstable_by_key(|link| {
            (
                link.source.start,
                std::cmp::Reverse(link.source.end),
                link.image,
            )
        });
        let mut previous_end = 0;
        b.doc.source_links.retain(|link| {
            if link.source.start < previous_end {
                if link.image {
                    b.doc
                        .nested_images
                        .push((link.source.start, link.target.clone()));
                }
                return false;
            }
            previous_end = link.source.end;
            true
        });
        crate::review::bind(&mut b.doc, source);
        b.doc
    }

    pub fn push_empty_paragraph(&mut self, caret: usize) {
        let heading = self
            .regions
            .last()
            .map_or_else(String::new, |r| r.heading.clone());
        let owner = self.owners.len();
        self.owners.push(Owner {
            source: caret..caret,
            after: caret,
        });
        self.layout.push(Layout::Text(self.regions.len()));
        self.regions.push(Region {
            runs: vec![Run {
                edit: Some(Edit::Text),
                ..Run::default()
            }],
            source: caret..caret,
            owner,
            kind: "paragraph",
            heading,
            ..Region::default()
        });
    }

    pub fn source_link(&self, byte: usize) -> Option<&SourceLink> {
        let i = self
            .source_links
            .partition_point(|link| link.source.end <= byte);
        self.source_links
            .get(i)
            .filter(|link| link.source.contains(&byte))
    }

    pub fn anchor_byte(&self, fragment: &str) -> Option<usize> {
        if let Some(name) = fragment.strip_prefix("fnref-")
            && let Some(link) = self
                .source_links
                .iter()
                .find(|link| link.footnote && link.target.strip_prefix("#fn-") == Some(name))
        {
            return Some(link.source.start);
        }
        self.regions
            .iter()
            .find(|region| !region.slug.is_empty() && region.slug == fragment)
            .map(|region| region.source.start)
    }
}

struct Builder<'a> {
    source: &'a str,
    lines: Lines,
    doc: Markdown,
    headings: Vec<(u8, String)>,
    slugs: comrak::Anchorizer,
}

impl Builder<'_> {
    fn span(&self, node: Node<'_>) -> Bytes {
        let data = node.data();
        self.lines
            .span(data.sourcepos)
            .map(|mut span| {
                // Comrak can end a nested table's span before the final closing pipe.
                if matches!(data.value, NodeValue::Table(_)) {
                    span.end = self.lines.end_of_line(span.end);
                }
                span
            })
            .filter(|range| self.source.get(range.clone()).is_some())
            .unwrap_or_default()
    }

    fn table_row_span(&self, row: Node<'_>, quote_depth: usize) -> Bytes {
        if matches!(row.data().value, NodeValue::TableRow(true)) {
            return self.span(row);
        }
        let line = row.data().sourcepos.start.line - 1;
        let start = self.lines.starts[line];
        let end = self.lines.ends[line];
        start + table::row_prefix_len(&self.source[start..end], quote_depth)..end
    }

    fn region(
        &self,
        node: Node<'_>,
        owner: usize,
        kind: &'static str,
        indent: usize,
        quote: bool,
    ) -> Region {
        Region {
            source: self.span(node),
            owner,
            kind,
            indent,
            quote,
            heading: self
                .headings
                .iter()
                .map(|(_, title)| title.as_str())
                .collect::<Vec<_>>()
                .join(" / "),
            ..Region::default()
        }
    }

    fn add(&mut self, mut region: Region) -> usize {
        region.text = region.runs.iter().map(|r| r.text.as_str()).collect();
        let index = self.doc.regions.len();
        self.doc.regions.push(region);
        index
    }

    fn block(
        &mut self,
        node: Node<'_>,
        owner: usize,
        kind: &'static str,
        indent: usize,
        quoted: bool,
        prefix: String,
    ) {
        use NodeValue::*;
        let data = node.data();
        match &data.value {
            List(list) => {
                for (i, item) in node.children().enumerate() {
                    let bullet = match &item.data().value {
                        TaskItem(task) if task.symbol.is_some() => "☑  ".into(),
                        TaskItem(_) => "☐  ".into(),
                        _ if list.list_type == ListType::Ordered => {
                            format!("{}.  ", list.start + i)
                        }
                        _ => "•  ".into(),
                    };
                    let raw = self.span(item);
                    let task = if let TaskItem(task) = &item.data().value {
                        self.lines
                            .span(task.symbol_sourcepos)
                            .map(|span| span.start)
                    } else {
                        None
                    };
                    let mut metadata = ListItem::new(self.source, raw, task);
                    if item.first_child().is_none() && metadata.is_some() {
                        let mut region = self.region(item, owner, kind, indent + 1, quoted);
                        let end = self.lines.end_of_line(region.source.start);
                        region.source = end..end;
                        region.list = metadata.take();
                        region
                            .runs
                            .push(display_run(bullet.clone(), Style::default(), None));
                        region.runs.push(Run {
                            edit: Some(Edit::Text),
                            ..Run::default()
                        });
                        let index = self.add(region);
                        self.doc.layout.push(Layout::Text(index));
                    }
                    for (j, child) in item.children().enumerate() {
                        let index = self.doc.regions.len();
                        self.block(
                            child,
                            owner,
                            kind,
                            indent + 1,
                            quoted,
                            if j == 0 {
                                bullet.clone()
                            } else {
                                String::new()
                            },
                        );
                        if j == 0
                            && matches!(&child.data().value, Paragraph | Heading(_))
                            && let Some(region) = self.doc.regions.get_mut(index)
                        {
                            region.list = metadata.take();
                        }
                    }
                }
            }
            BlockQuote | Item(_) | TaskItem(_) => {
                for child in node.children() {
                    self.block(child, owner, kind, indent + 1, true, String::new());
                }
            }
            FootnoteDefinition(note) => {
                let before = self.doc.regions.len();
                let label = format!("[^{}]  ", note.name);
                for (i, child) in node.children().enumerate() {
                    self.block(
                        child,
                        owner,
                        kind,
                        indent,
                        quoted,
                        if i == 0 { label.clone() } else { String::new() },
                    );
                }
                if let Some(region) = self.doc.regions.get_mut(before) {
                    region.slug = format!("fn-{}", note.name);
                    region.runs.push(display_run(
                        "  ↩".into(),
                        Style::default(),
                        Some(format!("#fnref-{}", note.name)),
                    ));
                    region.text.push_str("  ↩");
                }
            }
            Table(table) => {
                let mut rows = Vec::new();
                let quote_depth = node
                    .ancestors()
                    .filter(|parent| matches!(parent.data().value, BlockQuote))
                    .count();
                for (row_index, row) in node.children().enumerate() {
                    let raw_cells = split_row(self.source, self.table_row_span(row, quote_depth));
                    let mut cells = Vec::new();
                    for (cell_index, cell) in row.children().enumerate() {
                        let mut region = self.region(cell, owner, "table", indent, quoted);
                        if let Some(raw) = raw_cells.get(cell_index) {
                            let s = &self.source[raw.clone()];
                            let start = raw.start + s.len() - s.trim_start().len();
                            region.source = start..(raw.start + s.trim_end().len()).max(start);
                        } else if let Some(last) = raw_cells.last() {
                            region.source = last.end..last.end;
                            region.missing_cells = cell_index + 1 - raw_cells.len();
                        }
                        region.row = Some(row_index);
                        region.cell = Some(cell_index);
                        self.inlines(
                            cell,
                            &mut region,
                            Style {
                                bold: row_index == 0,
                                ..Style::default()
                            },
                        );
                        if region.runs.is_empty() && !raw_cells.is_empty() {
                            region.runs.push(Run {
                                edit: Some(Edit::Cell),
                                ..Run::default()
                            });
                        }
                        cells.push(self.add(region));
                    }
                    rows.push(cells);
                }
                self.doc.layout.push(Layout::Table {
                    rows,
                    alignment: table.alignments.clone(),
                    source: self.span(node),
                });
            }
            ThematicBreak => self.doc.layout.push(Layout::Rule),
            CodeBlock(code) => {
                let mut region = self.region(node, owner, kind, indent, quoted);
                region.mermaid = code.fenced
                    && code
                        .info
                        .split_ascii_whitespace()
                        .next()
                        .is_some_and(|lang| lang.eq_ignore_ascii_case("mermaid"));
                let style = Style {
                    code: true,
                    ..Style::default()
                };
                let literal = if code.literal.contains('\r') {
                    std::borrow::Cow::Owned(normalize_newlines(&code.literal, "\n"))
                } else {
                    std::borrow::Cow::Borrowed(code.literal.as_str())
                };
                let body = literal.strip_suffix('\n').unwrap_or(&literal);
                let mut cursor = region.source.start;
                if code.fenced
                    && let Some(next) = self.lines.starts.get(data.sourcepos.start.line)
                {
                    cursor = *next;
                }
                for line in body.split('\n') {
                    if !region.runs.is_empty() {
                        region.runs.push(display_run("\r".into(), style, None));
                    }
                    let end = self.lines.end_of_line(cursor).min(region.source.end);
                    let found = self
                        .source
                        .get(cursor..end)
                        .and_then(|raw| raw.find(line))
                        .map(|at| cursor + at);
                    let units = found.map(|at| literal_units(line, at));
                    let edit = found.map(|_| Edit::Code);
                    region.runs.push(Run {
                        text: line.into(),
                        units: units.unwrap_or_else(|| empty_units(line)),
                        style,
                        edit,
                        empty_at: found.filter(|_| line.is_empty()),
                        ..Run::default()
                    });
                    cursor = self
                        .lines
                        .starts
                        .get(self.lines.position(cursor).0)
                        .copied()
                        .unwrap_or(end);
                }
                let index = self.add(region);
                self.doc.layout.push(Layout::Text(index));
            }
            Paragraph | Heading(_) => {
                let mut region = self.region(node, owner, kind, indent, quoted);
                if let Heading(heading) = &data.value {
                    region.level = heading.level;
                    region.slug = self.slugs.anchorize(&inline_text(node));
                }
                if !prefix.is_empty() {
                    region
                        .runs
                        .push(display_run(prefix, Style::default(), None));
                }
                self.inlines(node, &mut region, Style::default());
                let index = self.add(region);
                self.doc.layout.push(Layout::Text(index));
            }
            HtmlBlock(html) if !html.literal.trim_start().starts_with("<!--") => {
                let mut region = self.region(node, owner, kind, indent, quoted);
                region.runs.push(display_run(
                    normalize_newlines(html.literal.trim_end(), "\r"),
                    Style {
                        code: true,
                        ..Style::default()
                    },
                    None,
                ));
                let index = self.add(region);
                self.doc.layout.push(Layout::Text(index));
            }
            _ => {}
        }
    }

    fn inlines(&self, node: Node<'_>, region: &mut Region, style: Style) {
        let mut walker = Inlines {
            builder: self,
            cursor: region.source.start,
            end: region.source.end,
            cell: region.kind == "table",
            runs: &mut region.runs,
        };
        for child in node.children() {
            walker.walk(child, style, None);
        }
    }
}

struct Inlines<'a, 's> {
    builder: &'a Builder<'s>,
    cursor: usize,
    end: usize,
    cell: bool,
    runs: &'a mut Vec<Run>,
}

impl Inlines<'_, '_> {
    fn walk(&mut self, node: Node<'_>, mut style: Style, link: Option<&str>) {
        use NodeValue::*;
        let data = node.data();
        match &data.value {
            Text(text) => self.text(
                node,
                text,
                style,
                link,
                if self.cell { Edit::Cell } else { Edit::Text },
            ),
            Code(code) => {
                style.code = true;
                self.text(node, &code.literal, style, link, Edit::InlineCode);
            }
            SoftBreak => self
                .runs
                .push(display_run(" ".into(), style, link.map(str::to_owned))),
            LineBreak => self.runs.push(display_run(
                "\u{000b}".into(),
                style,
                link.map(str::to_owned),
            )),
            Strong | Emph | Strikethrough | Highlight | Link(_) => {
                match &data.value {
                    Strong => style.bold = true,
                    Emph => style.italic = true,
                    Strikethrough => style.strike = true,
                    Highlight => style.mark = true,
                    Link(_) => {}
                    _ => unreachable!(),
                }
                let link = match &data.value {
                    Link(data) => Some(data.url.as_str()),
                    _ => link,
                };
                for child in node.children() {
                    self.walk(child, style, link);
                }
            }
            Image(image) => {
                let alt = inline_text(node);
                let mut run = display_run(
                    if alt.is_empty() {
                        "[Изображение]".into()
                    } else {
                        format!("[{alt}]")
                    },
                    style,
                    link.map(str::to_owned),
                );
                run.image = Some(image.url.clone());
                self.runs.push(run);
                self.advance(node);
            }
            FootnoteReference(note) => {
                self.runs.push(display_run(
                    format!("[^{}]", note.name),
                    style,
                    Some(format!("#fn-{}", note.name)),
                ));
                self.advance(node);
            }
            HtmlInline(html) => {
                if ["<br>", "<br/>", "<br />"]
                    .iter()
                    .any(|tag| html.eq_ignore_ascii_case(tag))
                {
                    let span = self.builder.span(node);
                    let mapped = self
                        .builder
                        .source
                        .get(span.clone())
                        .filter(|raw| *raw == html)
                        .map(|_| span);
                    self.runs.push(Run {
                        units: vec![mapped],
                        ..display_run("\u{000b}".into(), style, link.map(str::to_owned))
                    });
                } else if !html.starts_with("<!--") {
                    self.runs.push(display_run(
                        html.clone(),
                        Style {
                            code: true,
                            ..style
                        },
                        None,
                    ));
                }
                self.advance(node);
            }
            _ => {
                let link = match &data.value {
                    Link(data) => Some(data.url.as_str()),
                    _ => link,
                };
                for child in node.children() {
                    self.walk(child, style, link);
                }
            }
        }
    }

    fn advance(&mut self, node: Node<'_>) {
        self.cursor = self.cursor.max(self.builder.span(node).end.min(self.end));
    }

    fn text(&mut self, node: Node<'_>, text: &str, style: Style, link: Option<&str>, edit: Edit) {
        let span = self.builder.span(node);
        let source = self.builder.source;
        let hinted = span.start.clamp(self.cursor, self.end);
        let candidates = [hinted, self.cursor];
        let mapped = candidates.into_iter().find_map(|start| {
            let raw = source.get(start..self.end)?;
            // Comrak positions can shift after table escapes or an autolink. Keep the
            // recovery local so a token cannot silently consume unrelated later text.
            raw.char_indices().take(24).find_map(|(delta, _)| {
                let from = start + delta;
                decode_units(
                    &source[from..self.end],
                    text,
                    from,
                    edit != Edit::InlineCode,
                )
            })
        });
        let units = if let Some((end, units)) = mapped {
            self.cursor = end;
            units
        } else {
            self.advance(node);
            empty_units(text)
        };
        self.runs.push(Run {
            text: text.into(),
            style,
            link: link.map(str::to_owned),
            units,
            edit: if link.is_some()
                && source
                    .get(span.clone())
                    .is_some_and(|raw| raw == text && raw.contains("://"))
            {
                None
            } else {
                Some(edit)
            },
            image: None,
            empty_at: None,
        });
    }
}

fn empty_units(text: &str) -> Vec<Option<Bytes>> {
    vec![None; text.encode_utf16().count()]
}

fn literal_units(text: &str, start: usize) -> Vec<Option<Bytes>> {
    text.char_indices()
        .flat_map(|(at, ch)| {
            std::iter::repeat_n(Some(start + at..start + at + ch.len_utf8()), ch.len_utf16())
        })
        .collect()
}

fn display_run(text: String, style: Style, link: Option<String>) -> Run {
    Run {
        units: empty_units(&text),
        text,
        style,
        link,
        ..Run::default()
    }
}

fn decode_units(
    raw: &str,
    visible: &str,
    start: usize,
    decode: bool,
) -> Option<(usize, Vec<Option<Bytes>>)> {
    if !decode {
        return raw
            .starts_with(visible)
            .then(|| (start + visible.len(), literal_units(visible, start)));
    }
    let mut units = Vec::with_capacity(visible.len());
    let mut at = 0;
    let mut rest = visible;
    while !rest.is_empty() {
        let tail = raw.get(at..)?;
        let ch = tail.chars().next()?;
        let escaped = tail
            .as_bytes()
            .get(1)
            .filter(|byte| byte.is_ascii_punctuation());
        let (value, size) = if ch == '\\'
            && let Some(&byte) = escaped
        {
            (std::borrow::Cow::Owned(char::from(byte).to_string()), 2)
        } else if ch == '&'
            && let Some(end) = tail
                .as_bytes()
                .iter()
                .take(40)
                .position(|&byte| byte == b';')
        {
            let entity = &tail[..=end];
            let decoded = html_escape::decode_html_entities(entity);
            if decoded == entity {
                (std::borrow::Cow::Borrowed(&tail[..1]), 1)
            } else {
                (decoded, entity.len())
            }
        } else {
            (
                std::borrow::Cow::Borrowed(&tail[..ch.len_utf8()]),
                ch.len_utf8(),
            )
        };
        rest = rest.strip_prefix(value.as_ref())?;
        units.extend(std::iter::repeat_n(
            Some(start + at..start + at + size),
            value.encode_utf16().count(),
        ));
        at += size;
    }
    Some((start + at, units))
}

pub fn split_row(source: &str, row: Bytes) -> Vec<Bytes> {
    let Some(raw) = source.get(row.clone()) else {
        return Vec::new();
    };
    let trim = raw.trim();
    let offset = row.start + raw.len() - raw.trim_start().len();
    let mut cells = Vec::new();
    let mut from = usize::from(trim.starts_with('|'));
    let mut escaped = false;
    for (at, ch) in trim.char_indices().skip(from) {
        if ch == '|' && !escaped {
            cells.push(offset + from..offset + at);
            from = at + 1;
        }
        escaped = ch == '\\' && !escaped;
    }
    if from < trim.len() {
        cells.push(offset + from..offset + trim.len());
    }
    cells
}
