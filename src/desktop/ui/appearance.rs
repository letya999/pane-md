use super::*;
use crate::desktop::theme::{EditorFont, MAX_FONT_SIZE, MIN_FONT_SIZE, readable};
use windows::Win32::UI::Controls::Dialogs::*;

pub(super) const COMMANDS: [(usize, L); 6] = [
    (RICH_FONT, L::RichFont),
    (SOURCE_FONT, L::SourceFont),
    (PAPER_COLOR, L::PaperColor),
    (TEXT_COLOR, L::TextColor),
    (LINK_COLOR, L::LinkColor),
    (RESET_APPEARANCE, L::ResetAppearance),
];

impl App {
    pub(super) fn choose_appearance(&mut self, command: usize) {
        let Some((_, label)) = COMMANDS.iter().find(|(id, _)| *id == command) else {
            return;
        };
        let title = wide(tr(*label).trim_end_matches('…'));
        let mut appearance = settings::load_appearance();
        match command {
            RICH_FONT | SOURCE_FONT => {
                let font = if command == RICH_FONT {
                    &mut appearance.rich
                } else {
                    &mut appearance.source
                };
                let Some(selected) = self.choose_font(font, &title) else {
                    return;
                };
                *font = selected;
            }
            PAPER_COLOR | TEXT_COLOR | LINK_COLOR => {
                let palette = appearance.style(self.dark).palette;
                let current = match command {
                    PAPER_COLOR => palette.canvas,
                    TEXT_COLOR => palette.text,
                    _ => palette.accent,
                };
                let Some(selected) = self.choose_color(current, &title) else {
                    return;
                };
                if command != PAPER_COLOR && !readable(selected, palette.canvas) {
                    self.error(tr(L::Appearance), &tr(L::ColorContrast));
                    return;
                }
                let colors = appearance.colors_mut(self.dark);
                match command {
                    PAPER_COLOR => colors.paper = Some(selected),
                    TEXT_COLOR => colors.ink = Some(selected),
                    _ => colors.link = Some(selected),
                }
            }
            RESET_APPEARANCE => appearance = EditorAppearance::default(),
            _ => return,
        }
        if let Err(error) = settings::save_appearance(&appearance) {
            self.error(tr(L::AppearanceError), error.as_ref());
            return;
        }
        self.apply_appearance(appearance);
    }

    pub(super) fn apply_appearance(&mut self, appearance: EditorAppearance) {
        self.store_position();
        self.appearance = appearance;
        self.document_background();
        self.refresh_viewport();
        self.invalidate_ruler();
    }

    pub(super) fn document_background(&self) {
        let paper = self.appearance.style(self.dark).palette.canvas;
        send(
            self.editor,
            EM_SETBKGNDCOLOR,
            0,
            Palette::rgb(paper).0 as isize,
        );
    }

    fn choose_font(&self, font: &EditorFont, title: &[u16]) -> Option<EditorFont> {
        let face = wide(&font.face);
        let mut initial = LOGFONTW {
            lfHeight: -(font.half_points * self.dpi + 72) / 144,
            lfWeight: FW_NORMAL.0 as i32,
            lfCharSet: DEFAULT_CHARSET,
            ..Default::default()
        };
        initial.lfFaceName[..face.len()].copy_from_slice(&face);
        let mut dialog = CHOOSEFONTW {
            lStructSize: size_of::<CHOOSEFONTW>() as u32,
            hwndOwner: self.hwnd,
            lpLogFont: &mut initial,
            Flags: CF_INITTOLOGFONTSTRUCT
                | CF_LIMITSIZE
                | CF_FORCEFONTEXIST
                | CF_NOSCRIPTSEL
                | CF_NOVERTFONTS
                | CF_ENABLEHOOK,
            nSizeMin: MIN_FONT_SIZE,
            nSizeMax: MAX_FONT_SIZE,
            lCustData: LPARAM(title.as_ptr() as isize),
            lpfnHook: Some(font_title),
            ..Default::default()
        };
        if !unsafe { ChooseFontW(&mut dialog).as_bool() } {
            self.appearance_dialog_error();
            return None;
        }
        let end = initial.lfFaceName.iter().position(|&unit| unit == 0)?;
        let selected = EditorFont::new(
            String::from_utf16_lossy(&initial.lfFaceName[..end]),
            (dialog.iPointSize + 2) / 5,
        );
        if selected.is_none() {
            self.error(tr(L::Appearance), &tr(L::InvalidFont));
        }
        selected
    }

    fn choose_color(&self, current: u32, title: &[u16]) -> Option<u32> {
        let mut swatches = [COLORREF(0xffffff); 16];
        swatches[0] = Palette::rgb(current);
        let mut dialog = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: self.hwnd,
            rgbResult: Palette::rgb(current),
            lpCustColors: swatches.as_mut_ptr(),
            Flags: CC_RGBINIT | CC_FULLOPEN | CC_ENABLEHOOK,
            lCustData: LPARAM(title.as_ptr() as isize),
            lpfnHook: Some(color_title),
            ..Default::default()
        };
        if !unsafe { ChooseColorW(&mut dialog).as_bool() } {
            self.appearance_dialog_error();
            return None;
        }
        Some(Palette::rgb(dialog.rgbResult.0).0)
    }

    fn appearance_dialog_error(&self) {
        let code = unsafe { CommDlgExtendedError().0 };
        if code != 0 {
            self.error(tr(L::AppearanceDialogError), &format!("0x{code:04X}"));
        }
    }
}

unsafe extern "system" fn font_title(hwnd: HWND, msg: u32, _: WPARAM, l: LPARAM) -> usize {
    if msg == WM_INITDIALOG && l.0 != 0 {
        unsafe {
            let data = (*(l.0 as *const CHOOSEFONTW)).lCustData;
            let _ = SetWindowTextW(hwnd, PCWSTR(data.0 as *const u16));
            // The editor uses the family and size. Markdown supplies bold and italic.
            if let Ok(style) = GetDlgItem(Some(hwnd), cmb2 as i32) {
                let _ = EnableWindow(style, false);
            }
        }
    }
    0
}

unsafe extern "system" fn color_title(hwnd: HWND, msg: u32, _: WPARAM, l: LPARAM) -> usize {
    if msg == WM_INITDIALOG && l.0 != 0 {
        unsafe {
            let data = (*(l.0 as *const CHOOSECOLORW)).lCustData;
            let _ = SetWindowTextW(hwnd, PCWSTR(data.0 as *const u16));
        }
    }
    0
}
