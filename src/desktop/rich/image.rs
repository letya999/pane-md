use std::{
    fmt::Write,
    fs,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, Prefix},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use resvg::tiny_skia;
use windows::{
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPRGBA, IWICImagingFactory,
            WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::PCWSTR,
};

use crate::{
    desktop::ui::decode_url,
    images::{MAX_BYTES as MAX_COMPRESSED, rasterize_svg},
};

pub(super) fn embed(rtf: &mut String, directory: Option<&Path>, raw: &str) -> bool {
    let Some((data, width, height, kind)) = load(directory, raw) else {
        return false;
    };
    write_rtf(rtf, &data, width, height, kind, (width * 15).min(8_700));
    true
}

pub(super) fn embed_bytes(rtf: &mut String, bytes: &[u8]) -> bool {
    let Some((data, width, height, kind)) = decode(bytes.to_vec(), false) else {
        return false;
    };
    write_rtf(rtf, &data, width, height, kind, (width * 15).min(8_700));
    true
}

pub(super) fn write_rtf(
    rtf: &mut String,
    data: &[u8],
    width: u32,
    height: u32,
    kind: &str,
    goal_width: u32,
) {
    let goal_height = (height * goal_width / width).max(15);
    let _ = writeln!(
        rtf,
        "{{\\pict\\{kind}\\picw{width}\\pich{height}\\picwgoal{goal_width}\\pichgoal{goal_height}"
    );
    for chunk in data.chunks(40) {
        for byte in chunk {
            let _ = write!(rtf, "{byte:02x}");
        }
        rtf.push('\n');
    }
    rtf.push('}');
}

fn load(directory: Option<&Path>, raw: &str) -> Option<(Vec<u8>, u32, u32, &'static str)> {
    if let Some(inline) = raw.strip_prefix("data:") {
        let (media, encoded) = inline.split_once(',')?;
        let (media, encoding) = media.split_once(';')?;
        let media = media.to_ascii_lowercase();
        if !encoding.eq_ignore_ascii_case("base64")
            || !matches!(
                media.as_str(),
                "image/png"
                    | "image/jpeg"
                    | "image/gif"
                    | "image/bmp"
                    | "image/webp"
                    | "image/tiff"
                    | "image/x-icon"
                    | "image/svg+xml"
            )
            || encoded.len() > MAX_COMPRESSED.div_ceil(3) * 4
        {
            return None;
        }
        let data = STANDARD.decode(encoded).ok()?;
        if data.len() > MAX_COMPRESSED {
            return None;
        }
        return decode(data, media == "image/svg+xml");
    }
    let (path, _) = raw.split_once('#').unwrap_or((raw, ""));
    let path = decode_url(path).ok()?;
    let relative = Path::new(&path);
    let data = read_relative(directory?, relative)?;
    let svg = relative
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    decode(data, svg)
}

fn read_relative(directory: &Path, relative: &Path) -> Option<Vec<u8>> {
    const REPARSE_POINT: u32 = 0x400;
    let path = relative.to_str()?;
    if path.contains(':') || relative.has_root() {
        return None;
    }
    let full = std::path::absolute(directory.join(relative)).ok()?;
    let mut components = full.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) {
        return None;
    }
    if components.next()? != Component::RootDir {
        return None;
    }
    let mut volume = prefix.as_os_str().to_os_string();
    volume.push("\\");
    let mut local = std::path::PathBuf::from(volume);
    let root = local
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Only fixed and removable local volumes are read without a click.
    if !matches!(unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) }, 2 | 3) {
        return None;
    }
    for component in components {
        match component {
            Component::ParentDir => {
                local.pop();
            }
            Component::Normal(part) => {
                local.push(part);
                if fs::symlink_metadata(&local).ok()?.file_attributes() & REPARSE_POINT != 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    crate::images::read_file(&local, MAX_COMPRESSED).ok()
}

fn decode(data: Vec<u8>, svg: bool) -> Option<(Vec<u8>, u32, u32, &'static str)> {
    if svg {
        let (png, width, height) = rasterize_svg(&data)?;
        return Some((png, width, height, "pngblip"));
    }
    let kind = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("pngblip")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("jpegblip")
    } else {
        None
    };
    let mut width = 0;
    let mut height = 0;
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = factory.CreateStream().ok()?;
        stream.InitializeFromMemory(&data).ok()?;
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        frame.GetSize(&mut width, &mut height).ok()?;
        if width == 0 || height == 0 || width > 8192 || height > 8192 || width * height > 16_000_000
        {
            return None;
        }
        if let Some(kind) = kind {
            return Some((data, width, height, kind));
        }

        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
        let mut pixels = tiny_skia::Pixmap::new(width, height)?;
        converter
            .CopyPixels(std::ptr::null(), width * 4, pixels.data_mut())
            .ok()?;
        let png = pixels.encode_png().ok()?;
        (png.len() <= MAX_COMPRESSED).then_some((png, width, height, "pngblip"))
    }
}
