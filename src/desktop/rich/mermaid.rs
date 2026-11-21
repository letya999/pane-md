use std::collections::VecDeque;

use mermaid_svg::Theme;
use resvg::{tiny_skia, usvg};

use super::image;

#[derive(Default)]
pub struct Diagrams {
    options: Option<usvg::Options<'static>>,
    entries: VecDeque<(bool, String, Option<String>)>,
}

impl Diagrams {
    pub(super) fn picture(&mut self, source: &str, dark: bool) -> Option<&str> {
        // Bound parsing/layout of diagrams supplied by documents.
        if source.len() > 24 * 1024
            || source.split('\r').take(257).count() > 256
            || source
                .bytes()
                .filter(|&byte| matches!(byte, b'[' | b'(' | b'{' | b';' | b'>'))
                .take(513)
                .count()
                > 512
        {
            return None;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|(theme, text, _)| *theme == dark && text == source)
        {
            return self.entries[index].2.as_deref();
        }
        const CACHE_BYTES: usize = 8 * 1024 * 1024;
        let options = self.options.get_or_insert_with(|| {
            let mut opts = usvg::Options {
                font_family: "Segoe UI".into(),
                image_href_resolver: usvg::ImageHrefResolver {
                    resolve_data: Box::new(|_, _, _| None),
                    resolve_string: Box::new(|_, _| None),
                },
                ..Default::default()
            };
            let fonts = opts.fontdb_mut();
            fonts.load_system_fonts();
            fonts.set_sans_serif_family("Segoe UI");
            opts
        });
        let picture =
            render(source, dark, options).filter(|rtf| rtf.len() + source.len() <= CACHE_BYTES);
        let new_size = source.len() + picture.as_ref().map_or(0, String::len);
        while self.entries.len() >= 32
            || new_size
                + self
                    .entries
                    .iter()
                    .map(|(_, text, rtf)| text.len() + rtf.as_ref().map_or(0, String::len))
                    .sum::<usize>()
                > CACHE_BYTES
        {
            self.entries.pop_front();
        }
        self.entries.push_back((dark, source.to_owned(), picture));
        self.entries.back()?.2.as_deref()
    }
}

fn render(source: &str, dark: bool, options: &usvg::Options<'_>) -> Option<String> {
    let theme = if dark {
        Theme::dark()
    } else {
        Theme::default()
    }
    .with_font("Segoe UI");
    let input = notepad_md::normalize_newlines(source, "\n");
    let svg = std::panic::catch_unwind(|| mermaid_svg::render_with(&input, &theme))
        .ok()?
        .ok()?;
    let tree = usvg::Tree::from_str(&svg, options).ok()?;
    let size = tree.size();
    let scale = (960.0 / size.width()).min(1200.0 / size.height()).min(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut image = tiny_skia::Pixmap::new(width * 2, height * 2)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale * 2.0, scale * 2.0),
        &mut image.as_mut(),
    );
    if image.data().chunks_exact(4).all(|pixel| pixel[3] == 0) {
        return None;
    }
    let png = image.encode_png().ok()?;
    let mut rtf = String::new();
    image::write_rtf(
        &mut rtf,
        &png,
        image.width(),
        image.height(),
        "pngblip",
        width * 15,
    );
    Some(rtf)
}
