use super::*;

pub(super) unsafe extern "system" fn tool_button_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if message == WM_MOUSEMOVE {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_QUERY,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            if TrackMouseEvent(&mut track).is_ok()
                && (track.hwndTrack != hwnd || track.dwFlags.0 & TME_LEAVE.0 == 0)
            {
                track.dwFlags = TME_LEAVE;
                track.hwndTrack = hwnd;
                let _ = TrackMouseEvent(&mut track);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        } else if message == WM_MOUSELEAVE {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

pub(super) fn control_button(
    parent: HWND,
    instance: HINSTANCE,
    label: &str,
    id: usize,
) -> Result<HWND> {
    let button = child(
        parent,
        instance,
        w!("BUTTON"),
        label,
        WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(BS_OWNERDRAW as u32),
        id,
    )?;
    unsafe {
        let _ = SetWindowSubclass(button, Some(tool_button_proc), 1, 0);
    }
    Ok(button)
}

pub(super) fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        SetDCBrushColor(dc, Palette::rgb(color));
        FillRect(dc, rect, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
}

pub(super) fn rounded(dc: HDC, r: RECT, radius: i32, color: u32, stroke: u32) {
    unsafe {
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        let pen = SelectObject(dc, GetStockObject(DC_PEN));
        SetDCBrushColor(dc, Palette::rgb(color));
        SetDCPenColor(dc, Palette::rgb(stroke));
        let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
        SelectObject(dc, pen);
        SelectObject(dc, brush);
    }
}

pub(super) fn text(
    dc: HDC,
    font: HFONT,
    color: u32,
    value: &str,
    mut rect: RECT,
    flags: DRAW_TEXT_FORMAT,
) {
    unsafe {
        let previous = SelectObject(dc, font.into());
        SetTextColor(dc, Palette::rgb(color));
        SetBkMode(dc, TRANSPARENT);
        DrawTextW(dc, &mut wide(value), &mut rect, flags | DT_NOPREFIX);
        SelectObject(dc, previous);
    }
}

pub(super) fn ui_font(pixels: i32) -> HFONT {
    ui_font_weight(pixels, FW_NORMAL.0)
}

pub(super) fn ui_font_weight(pixels: i32, weight: u32) -> HFONT {
    unsafe {
        CreateFontW(
            -pixels,
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    }
}

impl App {
    pub(super) fn draw_label(&self, item: &DRAWITEMSTRUCT) {
        let status = item.hwndItem == self.status;
        let draft = item.hwndItem == self.draft_quote;
        let p = self.palette;
        fill(
            item.hDC,
            &item.rcItem,
            if draft { p.canvas } else { p.frame },
        );
        text(
            item.hDC,
            if draft || item.hwndItem == self.outline || item.hwndItem == self.review_label {
                self.strong_font
            } else {
                self.font
            },
            if status || item.hwndItem == self.empty {
                p.muted
            } else {
                p.text
            },
            &get_text(item.hwndItem),
            item.rcItem,
            if item.hwndItem == self.empty {
                DT_WORDBREAK
            } else {
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS
            },
        );
    }

    pub(super) fn draw_button(&self, item: &DRAWITEMSTRUCT) {
        let id = item.CtlID as usize;
        let state = |flag: ODS_FLAGS| item.itemState.0 & flag.0 != 0;
        let disabled = state(ODS_DISABLED);
        let pressed = state(ODS_SELECTED);
        let selected = self.active.is_some()
            && match id {
                VIEW_RICH => self.mode().is_multiple_of(2),
                VIEW_SOURCE => self.mode() % 2 == 1,
                _ => false,
            };
        let dc = item.hDC;
        let p = self.palette;
        let mut cursor = POINT::default();
        let hover = !disabled
            && unsafe {
                GetCursorPos(&mut cursor).is_ok() && WindowFromPoint(cursor) == item.hwndItem
            };
        let solid = id == REVIEW_COMMIT
            && self
                .draft
                .as_ref()
                .is_some_and(|draft| draft.format.is_none());
        let bordered = matches!(id, VIEW_RICH | VIEW_SOURCE | DISPLAY);
        let card_menu = (CARD_MENU_BASE..CARD_MENU_BASE + CARD_SLOTS).contains(&id);
        let background = if card_menu
            || matches!(id, REVIEW_COMMIT | REVIEW_CANCEL) && self.draft_frame.is_some()
        {
            p.canvas
        } else {
            p.frame
        };
        fill(dc, &item.rcItem, background);
        if bordered || hover || pressed || solid {
            let face = if pressed {
                p.line
            } else if solid {
                p.action
            } else if selected {
                p.highlight
            } else if hover {
                p.header
            } else if bordered {
                p.canvas
            } else {
                background
            };
            let mut rect = item.rcItem;
            rect.bottom -= 1;
            rounded(
                dc,
                rect,
                self.dip(8),
                face,
                if selected || solid {
                    p.action
                } else if bordered {
                    p.line
                } else {
                    face
                },
            );
        }
        let icon = match id {
            OUTLINE if !self.nav_is_visible() => Some(Icon::OpenPane(false)),
            OUTLINE => Some(Icon::Chevron(false)),
            REVIEWS if !self.reviews_visible() => Some(Icon::OpenPane(true)),
            REVIEWS => Some(Icon::Chevron(true)),
            ADD_REVIEW | OPEN => Some(Icon::Plus),
            _ if card_menu || id == 204 => Some(Icon::More),
            _ => None,
        };
        let ink = if disabled {
            p.muted
        } else if solid && !pressed {
            p.on_action
        } else if selected {
            p.accent
        } else {
            p.text
        };
        if let Some(icon) = icon {
            icon.draw(dc, item.rcItem, ink, self.dpi);
        } else {
            text(
                dc,
                if id == DISPLAY {
                    self.strong_font
                } else {
                    self.font
                },
                ink,
                &get_text(item.hwndItem),
                item.rcItem,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
            );
        }
        if state(ODS_FOCUS) && !state(ODS_NOFOCUSRECT) {
            let mut rect = item.rcItem;
            unsafe {
                let _ = InflateRect(&mut rect, -2, -2);
                let _ = DrawFocusRect(dc, &rect);
            }
        }
    }
}

enum Icon {
    Chevron(bool),
    OpenPane(bool),
    Plus,
    More,
}

impl Icon {
    fn draw(self, dc: HDC, rect: RECT, color: u32, dpi: i32) {
        let x = (rect.left + rect.right) / 2;
        let y = (rect.top + rect.bottom) / 2;
        let d = |n| super::layout::scale(n, dpi);
        let segment = |a, b, c, e| unsafe {
            let _ = MoveToEx(dc, x + d(a), y + d(b), None);
            let _ = LineTo(dc, x + d(c), y + d(e));
        };
        unsafe {
            let previous = SelectObject(dc, GetStockObject(DC_PEN));
            SetDCPenColor(dc, Palette::rgb(color));
            match self {
                Self::Chevron(right) | Self::OpenPane(right) => {
                    let collapsed = matches!(self, Self::OpenPane(_));
                    let dir = if collapsed ^ right { 1 } else { -1 };
                    let offset = if collapsed { 6 * dir } else { 0 };
                    segment(offset - 3 * dir, -5, offset + 2 * dir, 0);
                    segment(offset + 2 * dir, 0, offset - 3 * dir, 6);
                    if collapsed && right {
                        segment(0, -5, 10, -5);
                        segment(10, -5, 10, 3);
                        segment(10, 3, 5, 3);
                        segment(5, 3, 2, 6);
                        segment(2, 6, 2, 3);
                        segment(2, 3, 0, 3);
                        segment(0, 3, 0, -5);
                    } else if collapsed {
                        for row in [-4, 0, 4] {
                            segment(-11, row, -9, row);
                            segment(-7, row, 0, row);
                        }
                    }
                }
                Self::Plus => {
                    segment(-5, 0, 6, 0);
                    segment(0, -5, 0, 6);
                }
                Self::More => {
                    for n in [-4, 0, 4] {
                        fill(
                            dc,
                            &RECT {
                                left: x + d(n),
                                right: x + d(n) + d(2),
                                top: y,
                                bottom: y + d(2),
                            },
                            color,
                        );
                    }
                }
            }
            SelectObject(dc, previous);
        }
    }
}
