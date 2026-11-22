use std::{error::Error, fs, path::PathBuf};

use resvg::{tiny_skia, usvg};

fn png(tree: &usvg::Tree, size: u32) -> Vec<u8> {
    let mut image = tiny_skia::Pixmap::new(size, size).expect("icon canvas");
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(size as f32 / 1024.0, size as f32 / 1024.0),
        &mut image.as_mut(),
    );
    image.encode_png().expect("encode icon PNG")
}

fn main() -> Result<(), Box<dyn Error>> {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    let svg = fs::read_to_string(assets.join("pane-md.svg"))?;
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default())?;
    fs::write(assets.join("pane-md.png"), png(&tree, 1024))?;

    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let frames = sizes.map(|size| png(&tree, size));
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + sizes.len() * 16;
    for (size, frame) in sizes.into_iter().zip(&frames) {
        ico.extend_from_slice(&[if size == 256 { 0 } else { size as u8 }; 2]);
        ico.extend_from_slice(&[0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += frame.len();
    }
    for frame in frames {
        ico.extend_from_slice(&frame);
    }
    fs::write(assets.join("pane-md.ico"), ico)?;
    Ok(())
}
