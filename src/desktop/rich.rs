use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    ops::Range,
    path::Path,
};

use pane_md::{
    markdown::{Layout, Markdown, Region, Run, Style},
    review::Binding,
    source_view,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
        UI::{
            Controls::{
                EM_CHARFROMPOS, EM_GETRECT, EM_POSFROMCHAR, EM_SCROLLCARET, EM_SETRECT, RichEdit::*,
            },
            WindowsAndMessaging::*,
        },
    },
    core::PCWSTR,
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn send(hwnd: HWND, id: u32, w: usize, l: isize) -> isize {
    // The control and any message payload are owned by the UI thread.
    unsafe { SendMessageW(hwnd, id, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

pub fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(text).as_ptr()));
    }
}

pub fn get_text(hwnd: HWND) -> String {
    let mut text = vec![0; unsafe { GetWindowTextLengthW(hwnd) } as usize + 1];
    let len = unsafe { GetWindowTextW(hwnd, &mut text) };
    String::from_utf16_lossy(&text[..len.max(0) as usize])
}

pub fn selection(hwnd: HWND) -> Range<usize> {
    let mut range = CHARRANGE::default();
    send(hwnd, EM_EXGETSEL, 0, &mut range as *mut _ as isize);
    range.cpMin.max(0) as usize..range.cpMax.max(0) as usize
}

pub fn select(hwnd: HWND, range: Range<usize>, scroll: bool) {
    let range = CHARRANGE {
        cpMin: range.start as i32,
        cpMax: range.end as i32,
    };
    send(hwnd, EM_EXSETSEL, 0, &range as *const _ as isize);
    if scroll {
        send(hwnd, EM_SCROLLCARET, 0, 0);
    }
}

pub fn scroll_pos(hwnd: HWND) -> POINT {
    let mut pt = POINT::default();
    send(hwnd, EM_GETSCROLLPOS, 0, &mut pt as *mut _ as isize);
    pt
}

pub fn set_scroll(hwnd: HWND, pt: POINT) {
    send(hwnd, EM_SETSCROLLPOS, 0, &pt as *const _ as isize);
}

pub fn hit(hwnd: HWND, point: POINT) -> Option<usize> {
    usize::try_from(send(hwnd, EM_CHARFROMPOS, 0, &point as *const _ as isize)).ok()
}

pub fn char_point(hwnd: HWND, at: usize) -> POINT {
    let mut point = POINT::default();
    send(
        hwnd,
        EM_POSFROMCHAR,
        &mut point as *mut _ as usize,
        at as isize,
    );
    point
}

pub fn text_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    send(hwnd, EM_GETRECT, 0, &mut rect as *mut _ as isize);
    rect
}

pub fn buffer(hwnd: HWND) -> Vec<u16> {
    let len = GETTEXTLENGTHEX {
        flags: GTL_PRECISE | GTL_NUMCHARS,
        codepage: 1200,
    };
    let size = send(hwnd, EM_GETTEXTLENGTHEX, &len as *const _ as usize, 0).max(0) as usize;
    // GETTEXTLENGTHEX counts the internal table characters as well.
    let mut data = vec![0u16; size + 1];
    let query = GETTEXTEX {
        cb: (data.len() * 2) as u32,
        flags: GT_RAWTEXT,
        codepage: 1200,
        ..Default::default()
    };
    let read = send(
        hwnd,
        EM_GETTEXTEX,
        &query as *const _ as usize,
        data.as_mut_ptr() as isize,
    );
    data.truncate(read.max(0) as usize);
    data
}

pub fn margins(hwnd: HWND, width: i32, height: i32, margin: i32, dpi: i32) {
    let rect = RECT {
        left: margin,
        top: 18 * dpi / 96,
        right: (width - margin).max(margin + 100),
        bottom: (height - 6).max(30),
    };
    send(hwnd, EM_SETRECT, 0, &rect as *const _ as isize);
}

pub fn stream(hwnd: HWND, rtf: &str) -> bool {
    let mut reader = rtf.as_bytes();
    let mut stream = EDITSTREAM {
        dwCookie: &mut reader as *mut &[u8] as usize,
        dwError: 0,
        pfnCallback: Some(read_rtf),
    };
    send(
        hwnd,
        EM_STREAMIN,
        SF_RTF as usize,
        &mut stream as *mut _ as isize,
    );
    stream.dwError == 0
}

unsafe extern "system" fn read_rtf(
    cookie: usize,
    destination: *mut u8,
    capacity: i32,
    read: *mut i32,
) -> u32 {
    let remaining = unsafe { &mut *(cookie as *mut &[u8]) };
    let count = remaining.len().min(capacity.max(0) as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(remaining.as_ptr(), destination, count);
        *read = count as i32;
    }
    *remaining = &remaining[count..];
    0
}

use super::theme::{EditorStyle, RICH_SIZE, SOURCE_SIZE};
use super::ui::RemoteImages;

#[derive(Default)]
pub struct Projection {
    translated: HashMap<usize, Vec<usize>>,
    pub text: Vec<u16>,
    pub region: Vec<Range<usize>>,
    pub links: Vec<(Range<usize>, String)>,
    pub source: Vec<Option<usize>>,
    pub diagram_regions: HashSet<usize>,
    pub diagram_fallbacks: usize,
    pub diagram_pending: usize,
}

impl Projection {
    pub fn source(text: &str) -> Self {
        let (text, source) = source_view(text);
        Self {
            text,
            source,
            ..Self::default()
        }
    }

    pub fn region_range(&self, selection: Range<usize>) -> Option<(usize, Range<usize>)> {
        let region = self
            .region
            .partition_point(|span| span.end < selection.start);
        for i in region..self.region.len() {
            let span = &self.region[i];
            if span.start > selection.start {
                break;
            }
            if selection.start >= span.start && selection.end <= span.end {
                return Some((
                    i,
                    self.semantic_at(i, selection.start - span.start)
                        ..self.semantic_at(i, selection.end - span.start),
                ));
            }
        }
        None
    }

    pub fn edit_parts(
        &self,
        doc: &Markdown,
        selection: Range<usize>,
    ) -> Option<Vec<(usize, Range<usize>)>> {
        let left = self
            .region
            .partition_point(|span| span.end < selection.start);
        let mut right = self.region.partition_point(|span| span.end < selection.end);
        if self
            .region
            .get(right)
            .is_none_or(|span| selection.end < span.start)
        {
            // RichEdit expands a selection across columns to whole cells and their terminators.
            right = right.checked_sub(1)?;
        }
        let first = self.region.get(left)?;
        let last = self.region.get(right)?;
        if selection.start < first.start || last.end < selection.end {
            let start_table = doc.regions.get(left)?;
            let end_table = doc.regions.get(right)?;
            let terminators = |range: Range<usize>| {
                range.is_empty()
                    || self.text.get(range).is_some_and(|units| {
                        units.iter().all(|u| matches!(u, 7 | 13 | 0xfff9..=0xfffb))
                    })
            };
            if left > right
                || start_table.cell.is_none()
                || end_table.cell.is_none()
                || start_table.owner != end_table.owner
                || !terminators(selection.start..first.start)
                || !terminators(last.end..selection.end)
            {
                return None;
            }
        }
        (left..=right)
            .map(|i| {
                let span = &self.region[i];
                if self.diagram_regions.contains(&i) {
                    return None;
                }
                let start = selection.start.max(span.start) - span.start;
                let end = selection.end.min(span.end) - span.start;
                Some((i, self.semantic_at(i, start)..self.semantic_at(i, end)))
            })
            .collect()
    }

    pub fn review_parts<'a>(
        &'a self,
        doc: &'a Markdown,
        selection: Range<usize>,
    ) -> impl DoubleEndedIterator<Item = (usize, Range<usize>)> + 'a {
        let source = self.source_selection(doc, selection.clone());
        doc.regions
            .iter()
            .enumerate()
            .filter_map(move |(i, region)| {
                let range = if self.source.is_empty() {
                    let span = self.region.get(i)?;
                    if self.diagram_regions.contains(&i) {
                        return None;
                    }
                    let a = selection.start.max(span.start);
                    let b = selection.end.min(span.end);
                    if a >= b {
                        return None;
                    }
                    self.semantic_at(i, a - span.start)..self.semantic_at(i, b - span.start)
                } else {
                    if region.source.end <= source.start || source.end <= region.source.start {
                        return None;
                    }
                    let matches = |range: &Option<Range<usize>>| {
                        range
                            .as_ref()
                            .is_some_and(|r| r.start < source.end && source.start < r.end)
                    };
                    let units = region.runs.iter().flat_map(|run| &run.units);
                    let a = units.clone().position(matches)?;
                    let b = units
                        .enumerate()
                        .filter(|(_, unit)| matches(unit))
                        .last()?
                        .0
                        + 1;
                    a..b
                };
                region.source_selection(range.clone())?;
                Some((i, range))
            })
    }

    pub fn review_range(&self, doc: &Markdown, binding: &Binding) -> Option<Range<usize>> {
        let Binding::Attached {
            region,
            selection,
            tail,
            bytes,
            ..
        } = binding
        else {
            return None;
        };
        if !self.source.is_empty() {
            return Some(self.visible_at(doc, bytes.start)..self.visible_at(doc, bytes.end));
        }
        let (last, end) = tail
            .as_ref()
            .map_or((*region, selection.end), |(i, range)| (*i, range.end));
        (last < self.region.len())
            .then(|| self.native_at(*region, selection.start)..self.native_at(last, end))
    }

    fn semantic_at(&self, region: usize, local: usize) -> usize {
        self.translated
            .get(&region)
            .and_then(|map| map.get(local))
            .copied()
            .unwrap_or(local)
    }

    pub fn native_at(&self, region: usize, semantic: usize) -> usize {
        let local = self
            .translated
            .get(&region)
            .map_or(semantic, |map| map.partition_point(|&at| at < semantic));
        self.region[region].start + local
    }

    pub fn link(&self, at: usize) -> Option<&str> {
        let index = self.links.partition_point(|(span, _)| span.end <= at);
        self.links
            .get(index)
            .filter(|(span, _)| span.contains(&at))
            .map(|(_, target)| target.as_str())
    }

    pub fn source_at(&self, doc: &Markdown, at: usize) -> usize {
        if !self.source.is_empty() {
            return self
                .source
                .get(at)
                .copied()
                .flatten()
                .or_else(|| self.source.get(at.saturating_sub(1)).copied().flatten())
                .unwrap_or_default();
        }
        let i = self.region.partition_point(|span| span.end < at);
        if let Some((span, r)) = self.region.get(i).zip(doc.regions.get(i)) {
            if self.diagram_regions.contains(&i) {
                return if at == span.end {
                    r.source.end
                } else {
                    r.source.start
                };
            }
            let index = self.semantic_at(i, at.saturating_sub(span.start));
            let mut units = r.runs.iter().flat_map(|run| &run.units);
            if at == span.end && index != 0 {
                return units
                    .rev()
                    .find_map(|unit| unit.as_ref())
                    .map_or(r.source.end, |range| range.end);
            }
            return units
                .nth(index)
                .and_then(Option::as_ref)
                .map_or(r.source.start, |range| range.start);
        }
        doc.regions.last().map_or(0, |r| r.source.end)
    }

    pub fn source_selection(&self, doc: &Markdown, range: Range<usize>) -> Range<usize> {
        if !range.is_empty()
            && let Some((region, local)) = self.region_range(range.clone())
        {
            if self.diagram_regions.contains(&region) {
                return doc.regions[region].source.clone();
            }
            if let Some(source) = doc.regions[region].source_selection(local) {
                return source;
            }
        }
        let start = self.source_at(doc, range.start);
        start..self.source_at(doc, range.end).max(start)
    }

    pub fn visible_at(&self, doc: &Markdown, byte: usize) -> usize {
        if !self.source.is_empty() {
            let (mut left, mut right) = (0, self.source.len());
            while left < right {
                let middle = left + (right - left) / 2;
                let at = self.source[middle].unwrap_or_else(|| self.source[middle + 1].unwrap());
                if at < byte {
                    left = middle + 1;
                } else {
                    right = middle;
                }
            }
            return if self.source.get(left) == Some(&None) {
                left + 1
            } else {
                left.min(self.text.len())
            };
        }
        let index = doc.regions.partition_point(|r| r.source.end < byte);
        if let Some((span, r)) = self.region.get(index).zip(doc.regions.get(index)) {
            if r.list.is_some() && r.source.is_empty() {
                return span.end;
            }
            if self.diagram_regions.contains(&index) {
                return if byte == r.source.end {
                    span.end
                } else {
                    span.start
                };
            }
            let offset = r
                .runs
                .iter()
                .flat_map(|run| &run.units)
                .position(|unit| unit.as_ref().is_some_and(|range| range.start >= byte));
            return offset.map_or(
                if byte > r.source.start {
                    span.end
                } else {
                    span.start
                },
                |at| self.native_at(index, at),
            );
        }
        self.text.len()
    }
}

mod image;
mod live;
mod mermaid;
pub(super) use live::{apply_plain_edit, fit_new_paragraph, fit_trimmed_spaces};
pub(super) use mermaid::{Diagrams, prepare_diagrams};
pub fn source_rtf(
    view: &Projection,
    doc: &Markdown,
    style: &EditorStyle<'_>,
    show_reviews: bool,
) -> String {
    let mut rtf = style.rtf();
    let _ = write!(rtf, "\\pard\\f1\\fs{}\\cf1 ", style.source.half_points);
    let mut events = Vec::<(usize, usize, i32)>::new();
    let mut style = |range: Range<usize>, kind| {
        if range.start < range.end && range.end <= view.text.len() {
            events.extend([(range.start, kind, 1), (range.end, kind, -1)]);
        }
    };
    for region in doc.regions.iter().filter(|region| region.level > 0) {
        let start = view.visible_at(doc, region.source.start);
        let end = start
            + view.text[start..]
                .iter()
                .take_while(|&&unit| unit == 35)
                .count();
        if end > start {
            style(start..end, 0);
            let line_end = start
                + view.text[start..]
                    .iter()
                    .take_while(|&&unit| unit != 13)
                    .count();
            style(start..line_end, 3);
        }
    }
    for link in &doc.source_links {
        style(
            view.visible_at(doc, link.source.start)..view.visible_at(doc, link.source.end),
            0,
        );
    }
    for record in &doc.records {
        style(
            view.visible_at(doc, record.source.start)..view.visible_at(doc, record.source.end),
            1,
        );
        if show_reviews && let Some(span) = view.review_range(doc, &record.binding) {
            style(span, 2);
        }
    }
    events.sort_unstable();
    let mut cursor = 0;
    let mut active = [0; 4];
    for (at, kind, delta) in events {
        if at > cursor {
            source_run(&mut rtf, &view.text[cursor..at], active);
            cursor = at;
        }
        active[kind] += delta;
    }
    source_run(&mut rtf, &view.text[cursor..], active);
    // Keep a last empty source line in the control, even at EOF.
    if view.text.last() == Some(&13) {
        rtf.push_str(r"\par ");
    }
    rtf.push('}');
    rtf
}

fn source_run(rtf: &mut String, units: &[u16], active: [i32; 4]) {
    let color = if active[1] > 0 {
        7
    } else if active[0] > 0 {
        2
    } else {
        1
    };
    let highlight = if active[2] > 0 { 3 } else { 0 };
    let bold = if active[3] > 0 { "\\b" } else { "\\b0" };
    let _ = write!(rtf, "\\cf{color}\\highlight{highlight}{bold} ");
    escaped(rtf, units);
}

pub fn render(
    doc: &Markdown,
    style: &EditorStyle<'_>,
    prose_width: i32,
    directory: Option<&Path>,
    diagrams: &Diagrams,
    remote_images: &RemoteImages,
    show_reviews: bool,
) -> (String, Projection) {
    let mut rtf = style.rtf();
    let rich_size = |size| (size * style.rich.half_points + RICH_SIZE / 2) / RICH_SIZE;
    let code_size = (22 * style.source.half_points + SOURCE_SIZE / 2) / SOURCE_SIZE;
    let code_spacing = 315 * code_size / 22;
    let mut view = Projection {
        region: vec![0..0; doc.regions.len()],
        ..Projection::default()
    };
    let mut highlights = HashMap::<usize, Vec<Range<usize>>>::new();
    for record in &doc.records {
        if show_reviews
            && let Binding::Attached {
                region,
                selection,
                tail,
                ..
            } = &record.binding
        {
            highlights
                .entry(*region)
                .or_default()
                .push(selection.clone());
            if let Some((last, finish)) = tail {
                for index in region + 1..*last {
                    highlights
                        .entry(index)
                        .or_default()
                        .push(0..doc.regions[index].text.encode_utf16().count());
                }
                highlights.entry(*last).or_default().push(finish.clone());
            }
        }
    }
    let mut previous_was_paragraph = false;
    for block in &doc.layout {
        if previous_was_paragraph {
            rtf.push_str("\\par\n");
            view.text.push(13);
        }
        previous_was_paragraph = true;
        match block {
            Layout::Text(index) => {
                let region = &doc.regions[*index];
                let (picture, pending) = if region.mermaid {
                    diagrams.picture(&region.text)
                } else {
                    (None, false)
                };
                view.diagram_pending += usize::from(pending);
                view.diagram_fallbacks +=
                    usize::from(region.mermaid && !pending && picture.is_none());
                let size = rich_size(match region.level {
                    0 => RICH_SIZE,
                    1 => 56,
                    2 => 43,
                    3 => 30,
                    _ => 27,
                });
                let before = if region.level != 0 { 260 } else { 0 };
                let after = match (region.level, region.kind) {
                    (1, _) => 225,
                    (_, "list" | "quote") => 64,
                    _ => 165,
                };
                let _ = write!(
                    rtf,
                    "\\pard\\plain\\f0\\cf1\\fs{size}\\sb{before}\\sa{after}\\li{} ",
                    region.indent * 260
                );
                if region.level != 0 {
                    rtf.push_str(r"\b ");
                }
                if region.quote {
                    rtf.push_str(r"\brdrl\brdrs\brdrw20\brdrcf5\brsp95\cf7 ");
                }
                if picture.is_none() && matches!(region.kind, "code" | "html") {
                    let _ = write!(
                        rtf,
                        "\\f1\\fs{code_size}\\sb105\\sa140\\sl{code_spacing}\\slmult0\\shading10000\\cbpat6\\li220\\ri220 "
                    );
                }
                if let Some(picture) = picture {
                    let start = view.text.len();
                    rtf.push_str(picture);
                    view.text.push(0xfffc);
                    view.region[*index] = start..view.text.len();
                    view.diagram_regions.insert(*index);
                    continue;
                }
                render_region(
                    &mut rtf,
                    &mut view,
                    *index,
                    region,
                    highlights.get(index),
                    directory,
                    remote_images,
                );
            }
            Layout::Rule => {
                rtf.push_str(r"\pard\plain\fs12\sb160\sa180\brdrb\brdrs\brdrw12\brdrcf5 ");
                // A visible placeholder forces RichEdit to retain an empty rule paragraph.
                escaped(&mut rtf, &[160]);
                view.text.push(160);
            }
            Layout::Table {
                rows, alignment, ..
            } => {
                previous_was_paragraph = false;
                let mut widths = vec![0; alignment.len()];
                for row in rows {
                    for (col, id) in row.iter().enumerate() {
                        let count = doc.regions[*id].text.chars().count() as i32;
                        widths[col] = widths[col].max((120 * count + 240).clamp(1350, 3800));
                    }
                }
                let total: i32 = widths.iter().sum();
                if total < prose_width && !widths.is_empty() {
                    let extra = (prose_width - total) / widths.len() as i32;
                    widths.iter_mut().for_each(|width| *width += extra);
                }
                for (row_index, row) in rows.iter().enumerate() {
                    rtf.push_str("\\trowd\\trgaph108\\trleft0\\trrh420");
                    let mut right = 0;
                    for width in &widths {
                        if row_index == 0 {
                            rtf.push_str(r"\clcbpat6");
                        } else if row_index % 2 == 0 {
                            rtf.push_str(r"\clcbpat9");
                        }
                        for side in ['t', 'l', 'b', 'r'] {
                            let _ = write!(rtf, "\\clbrdr{side}\\brdrs\\brdrw10\\brdrcf5");
                        }
                        right += width;
                        let _ = write!(rtf, "\\cellx{right}");
                    }
                    rtf.push('\n');
                    view.text.extend([0xfff9, 13]);
                    for (col, index) in row.iter().enumerate() {
                        let _ = write!(
                            rtf,
                            "\\pard\\plain\\intbl\\f0\\cf1\\fs{}\\sb35\\sa45",
                            rich_size(22)
                        );
                        rtf.push_str(match alignment[col] {
                            comrak::nodes::TableAlignment::Center => r"\qc ",
                            comrak::nodes::TableAlignment::Right => r"\qr ",
                            _ => r"\ql ",
                        });
                        render_region(
                            &mut rtf,
                            &mut view,
                            *index,
                            &doc.regions[*index],
                            highlights.get(index),
                            directory,
                            remote_images,
                        );
                        rtf.push_str("\\cell\n");
                        view.text.push(7);
                    }
                    rtf.push_str("\\row\n");
                    view.text.extend([0xfffb, 13]);
                }
            }
        }
    }
    if view.text.last() == Some(&13)
        && let Some(Layout::Text(index)) = doc.layout.last()
        && doc.regions[*index].text.is_empty()
        && doc.regions[*index].source.is_empty()
    {
        rtf.push_str(r"\par ");
    }
    rtf.push('}');
    (rtf, view)
}

fn render_region(
    rtf: &mut String,
    view: &mut Projection,
    index: usize,
    region: &Region,
    highlights: Option<&Vec<Range<usize>>>,
    directory: Option<&Path>,
    remote_images: &RemoteImages,
) {
    let start = view.text.len();
    let mut semantic = 0;
    let mut translated = vec![0];
    let mut changed_length = false;
    for run in &region.runs {
        let at = view.text.len();
        if let Some(image) = &run.image
            && match remote_images.picture(image) {
                Some(bytes) => image::embed_bytes(rtf, bytes),
                None => image::embed(rtf, directory, image),
            }
        {
            view.text.push(0xfffc);
            translated.push(semantic + run.units.len());
            changed_length = true;
        } else {
            render_run(rtf, view, run, semantic, highlights);
            translated.extend(semantic + 1..=semantic + run.units.len());
        }
        semantic += run.units.len();
        if let Some(image) = &run.image
            && remote_images.picture(image).is_none()
            && matches!(
                pane_md::links::destination(image),
                Ok(pane_md::links::Destination::External(_))
            )
        {
            view.links
                .push((at..view.text.len(), format!("image:{image}")));
        } else if let Some(link) = &run.link {
            view.links.push((at..view.text.len(), link.clone()));
        } else if let Some(image) = &run.image {
            view.links
                .push((at..view.text.len(), format!("image:{image}")));
        }
    }
    view.region[index] = start..view.text.len();
    if changed_length {
        view.translated.insert(index, translated);
    }
}

fn render_run(
    rtf: &mut String,
    view: &mut Projection,
    run: &Run,
    start: usize,
    highlights: Option<&Vec<Range<usize>>>,
) {
    rtf.push_str(r"{\highlight0");
    let Style {
        bold,
        italic,
        strike,
        code,
        mark,
    } = run.style;
    if bold {
        rtf.push_str(r"\b");
    }
    if italic {
        rtf.push_str(r"\i");
    }
    if strike {
        rtf.push_str(r"\strike");
    }
    if code {
        rtf.push_str(r"\f1");
    }
    if run.link.is_some() || run.image.is_some() {
        rtf.push_str(r"\cf2\ul");
    }
    rtf.push(' ');
    let mut last_mark = None;
    for (i, ch) in run.text.encode_utf16().enumerate() {
        let highlight =
            highlights.is_some_and(|spans| spans.iter().any(|span| span.contains(&(start + i))));
        let color = if highlight {
            3
        } else if mark {
            4
        } else {
            0
        };
        if last_mark != Some(color) {
            let _ = write!(rtf, "\\highlight{color} ");
            last_mark = Some(color);
        }
        escape(rtf, ch);
        view.text.push(ch);
    }
    rtf.push('}');
}

pub fn escaped(out: &mut String, units: &[u16]) {
    for &unit in units {
        escape(out, unit);
    }
}

fn escape(out: &mut String, unit: u16) {
    match unit {
        9 => out.push_str(r"\tab "),
        11 => out.push_str(r"\line "),
        13 | 10 => out.push_str("\\par\n"),
        32..=126 => {
            let ch = char::from(unit as u8);
            if matches!(ch, '\\' | '{' | '}') {
                out.push('\\');
            }
            out.push(ch);
        }
        _ => {
            let _ = write!(out, "\\u{}?", unit as i16);
        }
    }
}
