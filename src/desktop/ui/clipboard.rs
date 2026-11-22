use windows::Win32::System::{
    DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
    Memory::{GlobalLock, GlobalSize, GlobalUnlock},
};

use super::*;

pub(super) const UNICODE_TEXT: u32 = 13; // CF_UNICODETEXT

pub(super) fn is_paste_key(message: &MSG) -> bool {
    if message.message != WM_KEYDOWN || unsafe { GetKeyState(VK_MENU.0.into()) >= 0 } {
        return false;
    }
    match message.wParam.0 {
        key if key == usize::from(b'V') => unsafe { GetKeyState(VK_CONTROL.0.into()) < 0 },
        key if key == usize::from(VK_INSERT.0) => unsafe { GetKeyState(VK_SHIFT.0.into()) < 0 },
        _ => false,
    }
}

pub(super) fn clipboard_text(window: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(window)).ok()?;
        let text = (|| {
            let handle = HGLOBAL(GetClipboardData(UNICODE_TEXT).ok()?.0);
            let capacity = GlobalSize(handle);
            if capacity < 2 || capacity > isize::MAX as usize {
                return None;
            }
            let pointer = GlobalLock(handle).cast::<u16>();
            if pointer.is_null() {
                return None;
            }
            let text = if pointer.is_aligned() {
                let units = std::slice::from_raw_parts(pointer, capacity / 2);
                units
                    .iter()
                    .position(|&unit| unit == 0)
                    .and_then(|end| String::from_utf16(&units[..end]).ok())
            } else {
                None
            };
            let _ = GlobalUnlock(handle);
            text
        })();
        let _ = CloseClipboard();
        text
    }
}

impl App {
    pub(super) fn visual_replacement(
        &self,
        selected: Range<usize>,
        text: &str,
    ) -> Option<pane_md::markdown::FormatPatch> {
        let doc = &self.docs[self.active?];
        self.parsed.paste_text(
            &doc.text,
            &self.view.edit_parts(&self.parsed, selected)?,
            text,
        )
    }

    pub(super) fn paste_visual(&mut self, text: &str) {
        if self.mode() != 2 || self.draft.is_some() || !self.projection_safe || text.is_empty() {
            return;
        }
        self.flush_edit();
        match self.visual_replacement(selection(self.editor), text) {
            Some(edit) => self.patch(edit.range, &edit.text, edit.caret.start),
            None => self.status(Some(tr(L::EditSourceHint))),
        }
    }

    pub(super) fn edit_selected_cells(&mut self, unit: u16) -> bool {
        let pending = self.pending_cell_input.take();
        if self.mode() != 2
            || self.draft.is_some()
            || !self.projection_safe
            || selection(self.editor).is_empty()
        {
            return false;
        }
        self.flush_edit();
        let Some(index) = self.active else {
            return false;
        };
        let selected = selection(self.editor);
        let Some(parts) = self.view.edit_parts(&self.parsed, selected.clone()) else {
            return false;
        };
        let (Some((first, _)), Some((last, _))) = (parts.first(), parts.last()) else {
            return false;
        };
        let a = &self.parsed.regions[*first];
        let b = &self.parsed.regions[*last];
        if first == last || a.cell.is_none() || b.cell.is_none() || a.owner != b.owner {
            return false;
        }
        let text = match unit {
            0 => String::new(),
            0xd800..=0xdbff => {
                self.pending_cell_input = Some((selected, unit));
                return true;
            }
            0xdc00..=0xdfff => {
                let Some((previous, high)) = pending else {
                    return true;
                };
                if previous != selected {
                    return true;
                }
                String::from_utf16_lossy(&[high, unit])
            }
            _ => String::from_utf16_lossy(&[unit]),
        };
        let source = &self.docs[index].text;
        match self.parsed.paste_text(source, &parts, &text) {
            Some(edit) => self.patch(edit.range, &edit.text, edit.caret.start),
            None => self.status(Some(tr(L::EditSourceHint))),
        }
        true
    }
}
