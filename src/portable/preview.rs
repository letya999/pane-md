use std::{fmt::Write, ops::Range, path::Path};

use super::reviews;
use crate::i18n::{Label as L, tr};
use base64::{Engine, engine::general_purpose::STANDARD};
use html_escape::{encode_double_quoted_attribute, encode_text};
use pane_md::{
    links::{Destination, destination},
    markdown::{Layout, Markdown, Region},
    utf16_byte,
};

pub fn html(
    doc: &Markdown,
    path: Option<&Path>,
    source: &str,
    editing: bool,
    show_reviews: bool,
) -> String {
    let mut out = String::new();
    let mut preview = Preview {
        path,
        source,
        editing,
        images: 12 * 1024 * 1024,
        highlights: if show_reviews {
            reviews::highlights(doc)
        } else {
            Default::default()
        },
    };
    for item in &doc.layout {
        match item {
            Layout::Rule => out.push_str("<hr>"),
            Layout::Text(index) => {
                let region = &doc.regions[*index];
                if region.mermaid {
                    preview.diagram(&mut out, region, *index);
                    continue;
                }
                let tag = match region.level {
                    1 => "h1",
                    2 => "h2",
                    3 => "h3",
                    4 => "h4",
                    5 => "h5",
                    6 => "h6",
                    _ => match region.kind {
                        "code" | "html" => "pre",
                        _ if region.quote => "blockquote",
                        _ => "p",
                    },
                };
                let list = if region.kind == "list" {
                    " list-item"
                } else {
                    ""
                };
                let _ = write!(
                    out,
                    "<{tag} class=\"region{list} indent-{}\" id=\"r{index}\" data-review-region=\"{index}\">",
                    region.indent.min(6)
                );
                preview.runs(&mut out, region, *index);
                let _ = write!(out, "</{tag}>");
            }
            Layout::Table {
                rows, alignment, ..
            } => {
                out.push_str("<div class=\"table-scroll\"><table>");
                for (row_index, row) in rows.iter().enumerate() {
                    out.push_str("<tr>");
                    let tag = if row_index == 0 { "th" } else { "td" };
                    let focus = if editing { " tabindex=\"-1\"" } else { "" };
                    for (column, index) in row.iter().enumerate() {
                        use comrak::nodes::TableAlignment::*;
                        let align = match alignment.get(column) {
                            Some(Center) => "center",
                            Some(Right) => "right",
                            _ => "left",
                        };
                        let _ = write!(
                            out,
                            "<{tag} class=\"{align}\" id=\"r{index}\" data-review-region=\"{index}\"{focus}>"
                        );
                        preview.runs(&mut out, &doc.regions[*index], *index);
                        let _ = write!(out, "</{tag}>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table></div>");
            }
        }
    }
    out
}

struct Preview<'a> {
    path: Option<&'a Path>,
    source: &'a str,
    editing: bool,
    images: usize,
    highlights: reviews::Highlights,
}

impl Preview<'_> {
    fn diagram(&mut self, out: &mut String, region: &Region, index: usize) {
        let _ = write!(
            out,
            "<figure id=\"r{index}\" data-diagram=\"{index}\"><pre data-diagram-source data-review-region=\"{index}\">"
        );
        self.runs(out, region, index);
        let _ = write!(
            out,
            "</pre><img data-diagram-image alt=\"Mermaid\" hidden><figcaption><span data-diagram-status data-pending=\"{}\" data-fallback=\"{}\">{}</span><button type=\"button\" data-show-diagram-source=\"{index}\">{}</button></figcaption></figure>",
            encode_double_quoted_attribute(tr(L::MermaidPending)),
            encode_double_quoted_attribute(tr(L::MermaidFallback)),
            encode_text(tr(L::MermaidFallback)),
            encode_text(tr(L::ShowSource)),
        );
    }

    fn runs(&mut self, out: &mut String, region: &Region, index: usize) {
        let mut at = 0;
        let checklist = region.list.as_ref().and_then(|item| item.task);
        let marks = self.highlights.get(&index).map_or(&[][..], Vec::as_slice);
        for run in &region.runs {
            let start = at;
            at += run.units.len();
            if start == 0
                && let Some(check) = checklist
            {
                let checked = if self.source.as_bytes().get(check) == Some(&b' ') {
                    ""
                } else {
                    " checked"
                };
                let disabled = if self.editing { "" } else { " disabled" };
                let _ = write!(
                    out,
                    "<input type=\"checkbox\" data-task=\"{index}\"{checked}{disabled}>"
                );
                if run.edit.is_none() {
                    let text = encode_double_quoted_attribute(&run.text);
                    let _ = write!(
                        out,
                        "<span data-review-width=\"{at}\" data-review-text=\"{text}\"></span>"
                    );
                    continue;
                }
            }
            if let Some(raw) = &run.image
                && let Some(image) = data_image(self.path, raw, &mut self.images)
            {
                if let Some(target) = &run.link {
                    open_link(out, target);
                }
                let _ = write!(
                    out,
                    "<img src=\"{image}\" alt=\"{}\" data-review-width=\"{}\">",
                    encode_double_quoted_attribute(&run.text),
                    at - start,
                );
                if run.link.is_some() {
                    out.push_str("</a>");
                }
                continue;
            }
            if let Some(raw) = &run.image
                && let Ok(Destination::External(url)) = destination(raw)
                && (url.starts_with("https://") || url.starts_with("http://"))
            {
                let url = encode_double_quoted_attribute(&url);
                let _ = write!(
                    out,
                    "<span class=\"remote-image\" data-review-width=\"{}\">",
                    at - start
                );
                if let Some(target) = &run.link {
                    open_link(out, target);
                }
                let _ = write!(
                    out,
                    "<img data-remote-image=\"{url}\" alt=\"{}\" hidden>",
                    encode_double_quoted_attribute(&run.text),
                );
                if run.link.is_some() {
                    out.push_str("</a>");
                }
                let _ = write!(
                    out,
                    "<button type=\"button\" data-load-remote-image=\"{url}\">{}</button></span>",
                    encode_text(tr(L::LoadRemoteImage))
                );
                continue;
            }
            let mut tags = Vec::new();
            for (enabled, tag) in [
                (run.style.bold, "strong"),
                (run.style.italic, "em"),
                (run.style.strike, "s"),
                (run.style.code, "code"),
                (run.style.mark, "mark"),
            ] {
                if enabled {
                    let _ = write!(out, "<{tag}>");
                    tags.push(tag);
                }
            }
            let link = run.link.as_ref().or(run.image.as_ref());
            if let Some(target) = link {
                open_link(out, target);
            }
            let writable = self.editing && run.edit.is_some() && !region.mermaid;
            if writable {
                let _ = write!(
                    out,
                    "<span data-review-run contenteditable=\"true\" spellcheck=\"false\" data-edit=\"{index}:{start}:{at}\">"
                );
            } else {
                out.push_str("<span data-review-run>");
            }
            write_marked(
                out,
                &run.text.replace(['\r', '\u{000b}'], "\n"),
                start..at,
                marks,
            );
            out.push_str("</span>");
            if link.is_some() {
                out.push_str("</a>");
            }
            for tag in tags.into_iter().rev() {
                let _ = write!(out, "</{tag}>");
            }
        }
    }
}

fn open_link(out: &mut String, target: &str) {
    let _ = write!(
        out,
        "<a href=\"#\" data-link=\"{}\">",
        encode_double_quoted_attribute(target)
    );
}

fn write_marked(out: &mut String, text: &str, run: Range<usize>, marks: &[(Range<usize>, usize)]) {
    let visible: Vec<_> = marks
        .iter()
        .filter_map(|(range, index)| {
            let lo = range.start.max(run.start);
            let hi = range.end.min(run.end);
            if lo >= hi {
                return None;
            }
            let first = utf16_byte(text, lo - run.start)?;
            let last = utf16_byte(text, hi - run.start)?;
            Some((first..last, index))
        })
        .collect();
    if visible.is_empty() {
        out.push_str(&encode_text(text));
        return;
    }
    let mut boundaries = vec![0, text.len()];
    for (range, _) in &visible {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    for bounds in boundaries.windows(2) {
        let first = bounds[0];
        let owners: Vec<_> = visible
            .iter()
            .filter(|(span, _)| span.contains(&first))
            .map(|(_, index)| index.to_string())
            .collect();
        if !owners.is_empty() {
            let _ = write!(
                out,
                "<mark class=\"review-mark\" data-reviews=\"{}\">",
                owners.join(" ")
            );
        }
        out.push_str(&encode_text(&text[first..bounds[1]]));
        if !owners.is_empty() {
            out.push_str("</mark>");
        }
    }
}

fn data_image(document: Option<&Path>, raw: &str, budget: &mut usize) -> Option<String> {
    use crate::images::{ImageFormat, MAX_BYTES, rasterize_svg, read_file, svg_extension};
    let limit = (*budget).min(MAX_BYTES);
    let (data, format) = if let Some(inline) = raw.strip_prefix("data:") {
        let (kind, encoded) = inline.split_once(',')?;
        let (media, encoding) = kind.split_once(';')?;
        let format = ImageFormat::from_mime(media)?;
        if !encoding.eq_ignore_ascii_case("base64") || encoded.len() > limit.div_ceil(3) * 4 {
            return None;
        }
        let data = STANDARD.decode(encoded).ok()?;
        if data.len() > limit || ImageFormat::sniff(&data, format == ImageFormat::Svg)? != format {
            return None;
        }
        (data, format)
    } else {
        let Destination::Local { path, anchor } = destination(raw).ok()? else {
            return None;
        };
        if !anchor.is_empty() || path.is_absolute() {
            return None;
        }
        let root = document?.parent()?.canonicalize().ok()?;
        let path = root.join(path).canonicalize().ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        let data = read_file(&path, limit).ok()?;
        let format = ImageFormat::sniff(&data, svg_extension(&path))?;
        (data, format)
    };
    let (data, mime) = if format == ImageFormat::Svg {
        (rasterize_svg(&data)?.0, ImageFormat::Png.mime())
    } else {
        if !format.raster_dimensions_valid(&data) {
            return None;
        }
        (data, format.mime())
    };
    *budget = budget.checked_sub(data.len())?;
    Some(format!("data:{mime};base64,{}", STANDARD.encode(data)))
}
