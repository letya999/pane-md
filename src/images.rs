use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{self, Read, Write as _},
    path::{Component, Path, PathBuf},
};

use pane_md::{
    markdown::{FormatPatch, Markdown},
    newline,
};
use resvg::{tiny_skia, usvg};

use crate::{
    Result,
    i18n::{Label as L, tr},
};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Svg,
}

impl ImageFormat {
    pub fn sniff(data: &[u8], svg: bool) -> Option<Self> {
        match data {
            [137, b'P', b'N', b'G', 13, 10, 26, 10, ..] => Some(Self::Png),
            [255, 216, 255, ..] => Some(Self::Jpeg),
            [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some(Self::Gif),
            [
                b'R',
                b'I',
                b'F',
                b'F',
                _,
                _,
                _,
                _,
                b'W',
                b'E',
                b'B',
                b'P',
                ..,
            ] => Some(Self::Webp),
            [b'B', b'M', ..] => Some(Self::Bmp),
            _ => svg.then_some(Self::Svg),
        }
    }

    #[cfg(not(windows))]
    pub fn from_mime(media: &str) -> Option<Self> {
        [
            Self::Png,
            Self::Jpeg,
            Self::Gif,
            Self::Webp,
            Self::Bmp,
            Self::Svg,
        ]
        .into_iter()
        .find(|format| media.eq_ignore_ascii_case(format.mime()))
    }

    #[cfg(not(windows))]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Bmp => "image/bmp",
            Self::Svg => "image/svg+xml",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
            Self::Svg => "svg",
        }
    }

    pub fn raster_dimensions_valid(self, data: &[u8]) -> bool {
        self != Self::Svg
            && imagesize::blob_size(data).is_ok_and(|size| {
                (1..=8192).contains(&size.width)
                    && (1..=8192).contains(&size.height)
                    && size.width * size.height <= 16_000_000
            })
    }
}

pub fn svg_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

pub fn rasterize_svg(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if data.len() > 512 * 1024 {
        return None;
    }
    let tree = usvg::Tree::from_str(
        std::str::from_utf8(data).ok()?,
        crate::diagram::svg_options(),
    )
    .ok()?;
    let size = tree.size();
    let scale = (2048.0 / size.width()).min(2048.0 / size.height()).min(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixels = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixels.as_mut(),
    );
    let png = pixels.encode_png().ok()?;
    (png.len() <= MAX_BYTES).then_some((png, width, height))
}

pub fn read_file(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(bytes)
}

pub struct ImageReference {
    pub alt: String,
    pub address: String,
}

pub fn import(document: &Path, picked: &Path) -> Result<ImageReference> {
    let folder = document
        .parent()
        .ok_or(tr(L::MissingDirectory))?
        .canonicalize()?;
    let source = picked.canonicalize()?;
    let data = read_file(&source, MAX_BYTES).map_err(|_| tr(L::ImageUnreadable))?;
    let format = ImageFormat::sniff(&data, svg_extension(picked)).ok_or(tr(L::ImageUnreadable))?;
    let supported = if format == ImageFormat::Svg {
        rasterize_svg(&data).is_some()
    } else {
        format.raster_dimensions_valid(&data)
    };
    if !supported {
        return Err(tr(L::ImageUnreadable).into());
    }
    let stem = picked
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let alt: String = stem
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let relative = source
        .strip_prefix(&folder)
        .ok()
        .filter(|path| format != ImageFormat::Svg || svg_extension(path))
        .and_then(image_url);
    let address = match relative {
        Some(relative) => relative,
        None => {
            image_url(&copy_image(&folder, stem, format, &data)?).ok_or(tr(L::ImageUnreadable))?
        }
    };
    Ok(ImageReference { alt, address })
}

pub fn insertion(
    document: &Path,
    source: &str,
    parsed: &Markdown,
    region: Option<usize>,
    picked: &Path,
) -> Result<FormatPatch> {
    let mut at = source.len();
    if let Some((owner, block)) =
        region
            .and_then(|index| parsed.regions.get(index))
            .and_then(|region| {
                parsed
                    .owners
                    .get(region.owner)
                    .map(|owner| (region.owner, owner))
            })
    {
        at = block.after;
        let next = parsed
            .owners
            .get(owner + 1)
            .map_or(source.len(), |next| next.source.start);
        for record in &parsed.records {
            if record.source.start >= at
                && record.source.end <= next
                && source[at..record.source.start].trim().is_empty()
            {
                at = record.source.end;
            }
        }
    }

    let ending = if source[at..].starts_with(['\r', '\n']) {
        newline(&source[at..])
    } else {
        newline(source)
    };
    let missing = |mut text: &str, before: bool| {
        let mut count = 2;
        while count > 0 {
            let next = if before {
                text.strip_suffix("\r\n")
                    .or_else(|| text.strip_suffix(['\r', '\n']))
            } else {
                text.strip_prefix("\r\n")
                    .or_else(|| text.strip_prefix(['\r', '\n']))
            };
            let Some(next) = next else { break };
            count -= 1;
            text = next;
        }
        count
    };
    let prefix = if at == 0 {
        String::new()
    } else {
        ending.repeat(missing(&source[..at], true))
    };
    let suffix = ending.repeat(missing(&source[at..], false));
    let image = import(document, picked)?;
    let mut alt = String::new();
    for ch in image.alt.chars() {
        if matches!(
            ch,
            '\\' | '[' | ']' | '`' | '*' | '_' | '~' | '&' | '<' | '>' | '|'
        ) {
            alt.push('\\');
        }
        alt.push(ch);
    }
    let text = format!("{prefix}![{alt}]({}){suffix}", image.address);
    let caret = if source[at..].trim().is_empty() {
        source.len() + text.len()
    } else {
        at + prefix.len()
    };
    Ok(FormatPatch::new(at..at, text, caret))
}

fn copy_image(folder: &Path, stem: &str, format: ImageFormat, data: &[u8]) -> io::Result<PathBuf> {
    let stem: String = stem
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '-'
            } else {
                ch
            }
        })
        .take(60)
        .collect();
    let stem = stem.trim_matches(['.', ' ']);
    let stem = if stem.is_empty() { "image" } else { stem };
    let mut pending = tempfile::NamedTempFile::new_in(folder)?;
    pending.write_all(data)?;
    let extension = format.extension();
    for attempt in 0.. {
        let name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{attempt}.{extension}")
        };
        match pending.persist_noclobber(folder.join(&name)) {
            Ok(_) => return Ok(PathBuf::from(name)),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                pending = error.file
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!()
}

fn image_url(path: &Path) -> Option<String> {
    let mut encoded = String::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        if !encoded.is_empty() {
            encoded.push('/');
        }
        for byte in name.to_str()?.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                encoded.push(char::from(byte));
            } else {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    (!encoded.is_empty()).then_some(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_md::{
        links::{Destination, destination},
        review::{self, Binding, Review, Status},
    };

    const PNG: &[u8] = include_bytes!("../assets/pane-md.png");
    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40"><rect width="100" height="40" fill="#cf1a76"/></svg>"##;

    #[test]
    fn local_image_uses_relative_url_and_keeps_existing_comment_near_its_paragraph() -> Result<()> {
        let work = tempfile::tempdir()?;
        let document = work.path().join("Plan.md");
        let figures = work.path().join("figures");
        fs::create_dir(&figures)?;
        let picture = figures.join("схема [A]#1.png");
        fs::write(&picture, PNG)?;
        let start = "# Plan\r\n\r\nReviewed line.\r\n\r\nNext line.\r\n";
        let parsed = Markdown::parse(start);
        let (at, quote, anchor) = review::capture(&parsed, 1, 0..13).unwrap();
        let record = Review {
            v: 1,
            id: "image-review".into(),
            quote,
            anchor,
            text: "Keep me".into(),
            status: Status::Open,
            updated_at: None,
        };
        let mut source = start.to_owned();
        source.insert_str(at, &review::insertion(start, at, &record));
        let parsed = Markdown::parse(&source);
        let patch = insertion(&document, &source, &parsed, Some(1), &picture)?;
        source.replace_range(patch.range, &patch.text);
        assert!(source.find(&record.encode()).unwrap() < source.find("![").unwrap());
        let parsed = Markdown::parse(&source);
        assert!(matches!(
            parsed.records[0].binding,
            Binding::Attached { .. }
        ));
        let link = parsed.source_links.iter().find(|link| link.image).unwrap();
        assert!(link.target.contains("%20%5BA%5D%231.png"));
        let Ok(Destination::Local { path, anchor }) = destination(&link.target) else {
            panic!("expected local image path");
        };
        assert!(anchor.is_empty());
        assert_eq!(work.path().join(path), picture);
        assert_eq!(fs::read_dir(&figures)?.count(), 1);
        assert!(source.ends_with("Next line.\r\n"));
        Ok(())
    }

    #[test]
    fn external_copy_is_atomic_and_never_overwrites_a_neighbor() -> Result<()> {
        let work = tempfile::tempdir()?;
        let external = tempfile::tempdir()?;
        let chosen = external.path().join("Diagram.svg");
        let document = work.path().join("Plan.md");
        fs::write(&chosen, SVG)?;
        fs::write(work.path().join("Diagram.svg"), b"keep existing asset")?;
        let source = "Last paragraph";
        let edit = insertion(
            &document,
            source,
            &Markdown::parse(source),
            Some(0),
            &chosen,
        )?;
        assert_eq!(edit.text, "\n\n![Diagram](Diagram-1.svg)\n\n");
        assert_eq!(edit.caret.start, source.len() + edit.text.len());
        assert_eq!(
            fs::read(work.path().join("Diagram.svg"))?,
            b"keep existing asset"
        );
        assert_eq!(fs::read(work.path().join("Diagram-1.svg"))?, SVG);
        let malformed = external.path().join("Invalid.svg");
        fs::write(&malformed, "<p>Invalid</p>")?;
        assert!(import(&document, &malformed).is_err());
        assert_eq!(fs::read_dir(work.path())?.count(), 2);
        Ok(())
    }
}
