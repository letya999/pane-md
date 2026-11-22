use super::*;

pub(super) unsafe extern "system" fn tab_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    frame: usize,
) -> LRESULT {
    unsafe {
        let frame = HWND(frame as *mut _);
        let state = GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>;
        if !state.is_null()
            && matches!(message, WM_LBUTTONDOWN | WM_LBUTTONUP | WM_CAPTURECHANGED)
            && let Ok(mut app) = (*state).try_borrow_mut()
        {
            let point = POINT {
                x: l.0 as i16 as i32,
                y: (l.0 >> 16) as i16 as i32,
            };
            let mut hit = TCHITTESTINFO {
                pt: point,
                ..Default::default()
            };
            let item = send(hwnd, TCM_HITTEST, 0, &mut hit as *mut _ as isize);
            let mut rect = RECT::default();
            let close = item >= 0
                && send(
                    hwnd,
                    TCM_GETITEMRECT,
                    item as usize,
                    &mut rect as *mut _ as isize,
                ) != 0
                && point.x >= rect.right - app.dip(26);
            if message == WM_LBUTTONDOWN && close {
                app.tab_close_down = Some(item as usize);
                drop(app);
                SetCapture(hwnd);
                return LRESULT(0);
            }
            if message == WM_LBUTTONUP
                && let Some(previous) = app.tab_close_down.take()
            {
                drop(app);
                let _ = ReleaseCapture();
                if close && item as usize == previous {
                    let _ = PostMessageW(Some(frame), WM_TAB_CLOSE, WPARAM(previous), LPARAM(0));
                }
                return LRESULT(0);
            }
            if message == WM_CAPTURECHANGED {
                app.tab_close_down = None;
            }
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}

impl App {
    pub(super) fn draw_tab(&self, item: &DRAWITEMSTRUCT) {
        let index = item.itemID as usize;
        if index >= self.docs.len() {
            return;
        }
        let active = self.active == Some(index);
        unsafe {
            let dc = item.hDC;
            let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
            let mut rect = item.rcItem;
            SetDCBrushColor(
                dc,
                Palette::rgb(if active {
                    self.palette.canvas
                } else {
                    self.palette.frame
                }),
            );
            FillRect(dc, &rect, brush);
            if active {
                let edge = RECT {
                    top: rect.bottom - self.dip(2),
                    ..rect
                };
                SetDCBrushColor(dc, Palette::rgb(self.palette.accent));
                FillRect(dc, &edge, brush);
            }
            rect.left += self.dip(8);
            rect.right -= self.dip(24);
            let font = if active { self.strong_font } else { self.font };
            let old = SelectObject(dc, font.into());
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, Palette::rgb(self.palette.text));
            DrawTextW(
                dc,
                &mut wide(&self.tab_name(index)),
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let mut close = RECT {
                left: rect.right,
                right: item.rcItem.right,
                ..rect
            };
            SetTextColor(dc, Palette::rgb(self.palette.muted));
            DrawTextW(
                dc,
                &mut wide("×"),
                &mut close,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER,
            );
            SelectObject(dc, old);
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                let mut focus = item.rcItem;
                let _ = InflateRect(&mut focus, -2, -2);
                let _ = DrawFocusRect(dc, &focus);
            }
        }
    }
}

pub(super) struct DocumentBar {
    pub rich: HWND,
    pub source: HWND,
    pub mode: HWND,
}

impl DocumentBar {
    pub fn new(parent: HWND, instance: HINSTANCE) -> Result<Self> {
        Ok(Self {
            rich: control_button(parent, instance, tr(L::RichFamily), VIEW_RICH)?,
            source: control_button(parent, instance, tr(L::SourceFamily), VIEW_SOURCE)?,
            mode: control_button(parent, instance, tr(L::ViewDropdown), DISPLAY)?,
        })
    }

    pub fn controls(&self) -> [HWND; 3] {
        [self.rich, self.source, self.mode]
    }

    pub fn place(&self, workspace: Workspace, dpi: i32) {
        let d = |n| scale(n, dpi);
        let x = workspace.left + d(15);
        let y = workspace.header + d(7);
        let width = workspace.right - workspace.left;
        let compact = width < d(380);
        let rich = d(if compact { 76 } else { 96 });
        let source = d(if compact { 44 } else { 58 });
        let mode = d(if compact { 142 } else { 182 });
        move_to(self.rich, x, y, rich, d(28), true);
        move_to(self.source, x + rich - 1, y, source, d(28), true);
        move_to(
            self.mode,
            (workspace.right - mode - d(15)).max(x + rich + source + d(4)),
            y,
            mode,
            d(28),
            true,
        );
    }

    pub fn update(&self, active: bool, state: usize) {
        set_text(
            self.mode,
            match state {
                READ => tr(L::ViewDropdown),
                EDITING => tr(L::EditDropdown),
                _ => tr(L::ReviewDropdown),
            },
        );
        for control in self.controls() {
            unsafe {
                let _ = EnableWindow(control, active);
                let _ = InvalidateRect(Some(control), None, false);
            }
        }
    }
}

use super::layout::{Workspace, scale};
