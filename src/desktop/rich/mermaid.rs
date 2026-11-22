use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

use super::image;
use crate::{desktop::ui::WM_DIAGRAM_READY, diagram::Image};

pub(in crate::desktop) type Diagrams = crate::diagram::Diagrams<String>;

pub(in crate::desktop) fn prepare_diagrams<'a>(
    diagrams: &mut Diagrams,
    sources: impl Iterator<Item = &'a str>,
    dark: bool,
    hwnd: HWND,
) {
    let hwnd = hwnd.0 as usize;
    diagrams.prepare(sources, dark, rtf_picture, move || unsafe {
        let _ = PostMessageW(
            Some(HWND(hwnd as *mut _)),
            WM_DIAGRAM_READY,
            WPARAM(0),
            LPARAM(0),
        );
    });
}

fn rtf_picture(image: Image) -> (String, usize) {
    let mut rtf = String::new();
    image::write_rtf(
        &mut rtf,
        &image.png,
        image.width * 2,
        image.height * 2,
        "pngblip",
        image.width * 15,
    );
    let bytes = rtf.len();
    (rtf, bytes)
}
