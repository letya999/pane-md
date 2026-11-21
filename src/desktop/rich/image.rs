use std::{
    fmt::Write,
    fs,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, Prefix},
};

use windows::{
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, IWICImagingFactory, WICDecodeMetadataCacheOnDemand,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::PCWSTR,
};

use crate::desktop::ui::decode_url;

pub(super) fn embed(rtf: &mut String, directory: &Path, raw: &str) -> bool {
    let Some((data, width, height, kind)) = load(directory, raw) else {
        return false;
    };
    let goal_width = (width * 15).min(8_700);
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
    true
}

fn load(directory: &Path, raw: &str) -> Option<(Vec<u8>, u32, u32, &'static str)> {
    const REPARSE_POINT: u32 = 0x400;
    const MAX_COMPRESSED: u64 = 8 * 1024 * 1024;
    let path = decode_url(raw).ok()?;
    let relative = Path::new(&path);
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
    let mut local = Path::new(prefix.as_os_str()).join("\\");
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
    let mut file = fs::File::open(&local).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_COMPRESSED {
        return None;
    }
    let data = {
        use std::io::Read;
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        (&mut file)
            .take(MAX_COMPRESSED + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_COMPRESSED {
            return None;
        }
        bytes
    };
    let kind = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "pngblip"
    } else if data.starts_with(b"\xff\xd8\xff") {
        "jpegblip"
    } else {
        // shortcut: PNG and JPEG are embedded natively; other formats need a WIC-to-PNG path.
        return None;
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
        decoder
            .GetFrame(0)
            .ok()?
            .GetSize(&mut width, &mut height)
            .ok()?;
    }

    (width > 0 && height > 0 && width <= 8192 && height <= 8192 && width * height <= 16_000_000)
        .then_some((data, width, height, kind))
}
