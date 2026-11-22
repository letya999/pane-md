use super::atoms::{fill, rounded, text};
use super::*;

pub(super) struct ReviewCard {
    pub body: HWND,
    pub menu: HWND,
}

impl ReviewCard {
    pub fn new(parent: HWND, instance: HINSTANCE, slot: usize, font: HFONT) -> Result<Self> {
        let card = Self {
            body: control_button(parent, instance, "", CARD_BASE + slot)?,
            menu: control_button(parent, instance, tr(L::More), CARD_MENU_BASE + slot)?,
        };
        for control in [card.body, card.menu] {
            send(control, WM_SETFONT, font.0 as usize, 0);
        }
        unsafe {
            let _ = SetWindowPos(
                card.menu,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Ok(card)
    }

    pub fn place(&self, rect: RECT, dpi: i32) {
        let d = |n| super::layout::scale(n, dpi);
        move_to(
            self.body,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            true,
        );
        move_to(
            self.menu,
            rect.right - d(31),
            rect.top + d(5),
            d(26),
            d(23),
            true,
        );
    }

    pub fn hide(&self) {
        move_to(self.body, 0, 0, 0, 0, false);
        move_to(self.menu, 0, 0, 0, 0, false);
    }
}

impl App {
    pub(super) fn draw_review_card(&self, item: &DRAWITEMSTRUCT) {
        let Some(&index) = self.card_rows.get(item.CtlID as usize - CARD_BASE) else {
            return;
        };
        let record = &self.parsed.records[index];
        let review = &record.review;
        let active = self.active_review.as_ref() == Some(&review.id);
        let dc = item.hDC;
        let p = self.palette;
        let rect = item.rcItem;
        fill(dc, &rect, p.frame);
        rounded(
            dc,
            rect,
            self.dip(12),
            p.canvas,
            if active { p.action } else { p.line },
        );
        if active {
            fill(
                dc,
                &RECT {
                    right: rect.left + self.dip(3),
                    top: rect.top + self.dip(6),
                    bottom: rect.bottom - self.dip(6),
                    ..rect
                },
                p.action,
            );
        }
        let quote = format!("«{}»", super::cards::excerpt(&review.quote, 90));
        let preview = match record.binding {
            Binding::Missing => Some(tr(L::ReviewUnattached)),
            Binding::Ambiguous => Some(tr(L::ReviewAmbiguous)),
            Binding::Attached { .. } if review.status == Status::Resolved => {
                Some(tr(L::ReviewResolved))
            }
            _ => None,
        };
        let body = super::cards::excerpt(&review.text, 180);
        let body = preview.map_or(body.clone(), |state| format!("{state} · {body}"));
        let area = RECT {
            left: rect.left + self.dip(12),
            right: rect.right - self.dip(11),
            ..rect
        };
        text(
            dc,
            self.strong_font,
            p.text,
            &quote,
            RECT {
                top: area.top + self.dip(10),
                right: area.right - self.dip(22),
                bottom: area.top + self.dip(31),
                ..area
            },
            DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        text(
            dc,
            self.font,
            p.text,
            &body,
            RECT {
                top: area.top + self.dip(32),
                bottom: area.bottom - self.dip(7),
                ..area
            },
            DT_WORDBREAK | DT_END_ELLIPSIS | DT_EDITCONTROL,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            let mut focus = rect;
            unsafe {
                let _ = InflateRect(&mut focus, -4, -4);
                let _ = DrawFocusRect(dc, &focus);
            }
        }
    }
    pub(super) fn draw_outline(&self, item: &DRAWITEMSTRUCT) {
        let dc = item.hDC;
        let mut rect = item.rcItem;
        fill(dc, &rect, self.palette.frame);
        let Some(region) = self
            .parsed
            .regions
            .iter()
            .filter(|r| r.level > 0)
            .nth(item.itemID as usize)
        else {
            return;
        };
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        if selected {
            fill(
                dc,
                &RECT {
                    right: rect.left + self.dip(2),
                    top: rect.top + self.dip(5),
                    bottom: rect.bottom - self.dip(5),
                    ..rect
                },
                self.palette.accent,
            );
        }
        rect.left += self.dip(12 + (region.level as i32 - 1) * 8);
        text(
            dc,
            if selected {
                self.strong_font
            } else {
                self.font
            },
            self.palette.text,
            &region.text,
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            unsafe {
                let _ = DrawFocusRect(dc, &item.rcItem);
            }
        }
    }
}
