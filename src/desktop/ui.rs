mod actions;
mod navigation;
mod review_panel;
pub(super) use navigation::decode_url;
use navigation::{FORMAT_ITEMS, TABLE_ITEMS, menu_bar};

use std::{
    cell::RefCell,
    ffi::OsString,
    fs,
    ops::Range,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use notepad_md::{
    diff, edit_source,
    markdown::{Layout, Markdown},
    review::{self, Binding, Review, Status},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
        System::{Com::*, LibraryLoader::*, Registry::*},
        UI::{
            Controls::{RichEdit::*, *},
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

use super::{
    Result,
    file::{Document, Viewport, markdown_path},
    rich::*,
};

const OPEN: usize = 101;
const FOLDER: usize = 102;
const SAVE: usize = 103;
const SAVE_AS: usize = 104;
const CLOSE: usize = 105;
const BACK: usize = 106;
const FIND: usize = 107;
const FIND_NEXT: usize = 108;
const TREE: usize = 109;
const REVIEWS: usize = 110;
const ZOOM_OUT: usize = 111;
const ZOOM_IN: usize = 112;
const ZOOM_RESET: usize = 113;
const UNDO: usize = 114;
const REDO: usize = 115;
const THEME: usize = 116;
const ADD_REVIEW: usize = 117;
const HEADINGS: usize = 118;
const EDIT_REVIEW: usize = 119;
const RESOLVE_REVIEW: usize = 120;
const DELETE_REVIEW: usize = 121;
const REATTACH: usize = 122;
const REVIEW_SAVE: usize = 123;
const REVIEW_CANCEL: usize = 124;
const BOLD: usize = 125;
const ITALIC: usize = 126;
const STRIKE: usize = 127;
const CODE: usize = 128;
const LINK: usize = 129;
const HEADING: usize = 130;
const BULLET: usize = 131;
const ROW_ADD: usize = 132;
const ROW_DELETE: usize = 133;
const COL_ADD: usize = 134;
const COL_DELETE: usize = 135;
const TABLE_TOOLS: usize = 136;
const OPEN_LINK: usize = 137;
const FIND_PREV: usize = 138;
const CLOSE_FIND: usize = 139;
const PARAGRAPH: usize = 140;
const LINE_BREAK: usize = 141;
const SHOW_SOURCE: usize = 142;
const MODES: usize = 150;
const RESULTS: usize = 170;
const FORMAT: usize = 171;
const ID_DOCUMENT: usize = 180;
const WM_SYNC: u32 = WM_APP + 1;
const WM_LINK: u32 = WM_APP + 2;
const WM_DOC_SCROLL: u32 = WM_APP + 3;
const WM_REVIEW_HIT: u32 = WM_APP + 4;
const MODES_TEXT: [&str; 4] = ["WYSIWYG View", "MD View", "WYSIWYG Edit", "MD Source Edit"];

pub fn run(path: Option<PathBuf>, wait: bool) -> Result<()> {
    // All HWNDs, GDI handles, COM objects, and callbacks in this module live on this UI thread.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
        let result = run_window(path, wait);
        CoUninitialize();
        result
    }
}

unsafe fn run_window(path: Option<PathBuf>, wait: bool) -> Result<()> {
    unsafe {
        LoadLibraryW(w!("msftedit.dll"))?;
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TREEVIEW_CLASSES | ICC_TAB_CLASSES,
        })
        .ok()
        .map_err(|e| format!("InitCommonControlsEx: {e}"))?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("notepad-md.window"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("notepad-md"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1210,
            820,
            None,
            None,
            Some(instance),
            None,
        )?;
        let mut app = App::new(hwnd, instance, wait)?;
        if let Some(path) = path {
            if let Err(error) = app.open(path, if wait { 3 } else { 0 }) {
                if wait {
                    app.error("Не удалось открыть файл", &error);
                    let _ = DestroyWindow(hwnd);
                    return Err(error);
                }
                app.error("Не удалось открыть документ", &error);
            }
        } else {
            app.refresh(0);
        }
        let state = Box::new(RefCell::new(app));
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (&*state as *const RefCell<App>) as isize,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        // A terminal host can request SW_HIDE for the console-subsystem launcher.
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = UpdateWindow(hwnd);
        if state.borrow().active.is_some() {
            let _ = SetFocus(Some(state.borrow().editor));
        }
        let mut msg = MSG::default();
        loop {
            let status = GetMessageW(&mut msg, None, 0, 0).0;
            if status == -1 {
                return Err(windows::core::Error::from_thread().into());
            }
            if status == 0 {
                break;
            }
            if let Some(command) = shortcut(&msg, &state.borrow()) {
                send(hwnd, WM_COMMAND, command, 0);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if !pointer.is_null()
            && let Ok(mut app) = (*pointer).try_borrow_mut()
        {
            if let Some(result) = app.message(message, w, l) {
                return LRESULT(result);
            }
        }
        if message == WM_DESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}

unsafe extern "system" fn document_proc(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    parent: usize,
) -> LRESULT {
    unsafe {
        let frame = HWND(parent as *mut _);
        if (message == WM_MOUSEMOVE && w.0 & 1 == 0) || message == WM_LBUTTONUP {
            let point = POINT {
                x: l.0 as i16 as i32,
                y: (l.0 >> 16) as i16 as i32,
            };
            let pointer = GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>;
            if !pointer.is_null()
                && let Ok(app) = (*pointer).try_borrow()
                && (app.mode() == 0 || (app.mode() == 2 && GetKeyState(VK_CONTROL.0.into()) < 0))
                && let Some(at) = hit(hwnd, point)
                && app.view.link(at).is_some()
            {
                let start = char_point(hwnd, at);
                let end = char_point(hwnd, at + 1);
                if point.x >= start.x - 2
                    && (end.y != start.y || point.x < end.x + 2)
                    && point.y >= start.y
                    && point.y < start.y + app.dip(30)
                {
                    if message == WM_MOUSEMOVE {
                        if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                            SetCursor(Some(cursor));
                        }
                        return LRESULT(0);
                    }
                    if selection(hwnd).is_empty() {
                        let _ = PostMessageW(Some(frame), WM_LINK, WPARAM(at), LPARAM(0));
                    }
                }
            }
        }
        if message == WM_MOUSEWHEEL && (w.0 & 8) != 0 {
            let id = if (w.0 >> 16) as i16 > 0 {
                ZOOM_IN
            } else {
                ZOOM_OUT
            };
            let _ = PostMessageW(Some(frame), WM_COMMAND, WPARAM(id), LPARAM(0));
            return LRESULT(0);
        }
        if message == WM_LBUTTONUP && selection(hwnd).is_empty() {
            let pt = POINT {
                x: l.0 as i16 as i32,
                y: (l.0 >> 16) as i16 as i32,
            };
            if let Some(at) = hit(hwnd, pt) {
                let _ = PostMessageW(Some(frame), WM_REVIEW_HIT, WPARAM(at), LPARAM(0));
            }
        }
        if message == WM_PASTE {
            // Pasting through RichEdit's native Unicode path prevents RTF and embedded OLE.
            return LRESULT(send(hwnd, EM_PASTESPECIAL, 13, 0));
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if matches!(message, WM_VSCROLL | WM_MOUSEWHEEL | WM_KEYUP) {
            let _ = PostMessageW(Some(frame), WM_DOC_SCROLL, WPARAM(0), LPARAM(0));
        }
        result
    }
}

fn shortcut(msg: &MSG, app: &App) -> Option<usize> {
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return None;
    }
    let key = msg.wParam.0 as u16;
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0.into()) < 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT.0.into()) < 0 };
    let alt = unsafe { GetKeyState(VK_MENU.0.into()) < 0 };
    if alt && key == VK_LEFT.0 {
        return Some(BACK);
    }
    if key == VK_F3.0 {
        return Some(if shift { FIND_PREV } else { FIND_NEXT });
    }
    if key == VK_TAB.0 && !ctrl && msg.hwnd != app.editor {
        return Some(if shift { 211 } else { 210 });
    }
    if key == VK_F6.0 {
        return Some(200);
    }
    if key == VK_ESCAPE.0 && app.find_open {
        return Some(CLOSE_FIND);
    }
    if key == VK_RETURN.0 && msg.hwnd == app.find_box {
        return Some(if shift { FIND_PREV } else { FIND_NEXT });
    }
    if key == VK_RETURN.0 && msg.hwnd == app.editor {
        if ctrl {
            return Some(OPEN_LINK);
        }
        if app.mode() == 2 {
            return Some(if shift { LINE_BREAK } else { PARAGRAPH });
        }
    }
    if ctrl {
        match key {
            0x4f => return Some(if shift { FOLDER } else { OPEN }),
            0x53 => return Some(if shift { SAVE_AS } else { SAVE }),
            0x57 => return Some(CLOSE),
            0x46 => return Some(FIND),
            0x4d if alt => return Some(ADD_REVIEW),
            0x42 if msg.hwnd == app.editor => return Some(BOLD),
            0x49 if msg.hwnd == app.editor => return Some(ITALIC),
            0x5a if msg.hwnd == app.editor => return Some(if shift { REDO } else { UNDO }),
            0x59 if msg.hwnd == app.editor => return Some(REDO),
            0xbb | 0x6b => return Some(ZOOM_IN),
            0xbd | 0x6d => return Some(ZOOM_OUT),
            0x30 => return Some(ZOOM_RESET),
            0x31..=0x34 if !shift => return Some(MODES + (key - 0x31) as usize),
            0x09 => return Some(if shift { 202 } else { 201 }),
            _ => {}
        }
    }
    None
}

struct TreeEntry {
    path: PathBuf,
    folder: bool,
    loaded: bool,
}

struct Draft {
    existing: Option<String>,
    capture: Option<(usize, String, review::Anchor)>,
    format: Option<(Range<usize>, Option<String>)>,
}

struct App {
    hwnd: HWND,
    wait: bool,
    toolbar: Vec<(HWND, i32)>,
    font: HFONT,
    brush: HBRUSH,
    dpi: i32,
    dark: bool,
    use_system: bool,
    palette: Palette,
    tab: HWND,
    tree: HWND,
    editor: HWND,
    empty: HWND,
    outline: HWND,
    status: HWND,
    review_list: HWND,
    review_label: HWND,
    review_text: HWND,
    review_bounds: RECT,
    document_top: i32,
    review_actions: [HWND; 4],
    draft_box: HWND,
    draft_actions: [HWND; 2],
    find_box: HWND,
    find_actions: [HWND; 3],
    docs: Vec<Document>,
    active: Option<usize>,
    parsed: Markdown,
    view: Projection,
    diagrams: Diagrams,
    projection_safe: bool,
    tree_open: bool,
    review_open: bool,
    find_open: bool,
    find_headings: bool,
    tree_entries: Vec<TreeEntry>,
    review_indices: Vec<usize>,
    active_review: Option<String>,
    draft: Option<Draft>,
    history: Vec<(PathBuf, Viewport)>,
    zoom: i32,
    zoom_status: HWND,
    workspace: Option<PathBuf>,
    content_width: i32,
}

impl App {
    fn new(hwnd: HWND, instance: HINSTANCE, wait: bool) -> Result<Self> {
        unsafe {
            let menu = menu_bar()?;
            SetMenu(hwnd, Some(menu)).map_err(|e| format!("SetMenu: {e}"))?;
            let editor = child(
                hwnd,
                instance,
                w!("RICHEDIT50W"),
                "",
                WS_TABSTOP
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | WINDOW_STYLE(
                        (ES_MULTILINE
                            | ES_AUTOVSCROLL
                            | ES_AUTOHSCROLL
                            | ES_NOHIDESEL
                            | ES_WANTRETURN) as u32,
                    ),
                ID_DOCUMENT,
            )?;
            SetWindowSubclass(editor, Some(document_proc), 1, hwnd.0 as usize)
                .ok()
                .map_err(|e| format!("SetWindowSubclass: {e}"))?;
            let tab = child(hwnd, instance, WC_TABCONTROLW, "", WS_TABSTOP, 0)?;
            let tree = child(
                hwnd,
                instance,
                WC_TREEVIEWW,
                "",
                WS_TABSTOP
                    | WINDOW_STYLE(
                        TVS_HASBUTTONS
                            | TVS_HASLINES
                            | TVS_LINESATROOT
                            | TVS_SHOWSELALWAYS
                            | TVS_DISABLEDRAGDROP,
                    ),
                0,
            )?;
            let ctrl = |class, label, style, id| child(hwnd, instance, class, label, style, id);
            let btn = |label, id| ctrl(w!("BUTTON"), label, WS_TABSTOP, id);
            let label = |text| ctrl(w!("STATIC"), text, WINDOW_STYLE(0), 0);
            let field = |id, style| ctrl(w!("EDIT"), "", WS_TABSTOP | WS_BORDER | style, id);
            let mut toolbar = Vec::new();
            if !wait {
                toolbar.push((btn("Открыть…", OPEN)?, 102));
                toolbar.push((btn("Папка", TREE)?, 72));
            }
            for (i, name) in MODES_TEXT.iter().enumerate() {
                toolbar.push((
                    ctrl(
                        w!("BUTTON"),
                        name,
                        WS_TABSTOP | WINDOW_STYLE((BS_AUTOCHECKBOX | BS_PUSHLIKE) as u32),
                        MODES + i,
                    )?,
                    if i % 2 == 0 { 118 } else { 109 },
                ));
            }
            toolbar.push((btn("Найти", FIND)?, 70));
            toolbar.push((btn("Ревью", REVIEWS)?, 73));
            toolbar.push((btn("Формат", FORMAT)?, 79));
            toolbar.push((btn("Таблица", TABLE_TOOLS)?, 83));
            let review_list = ctrl(
                w!("LISTBOX"),
                "",
                WS_TABSTOP
                    | WS_VSCROLL
                    | WS_BORDER
                    | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
                RESULTS,
            )?;
            let mut app = Self {
                hwnd,
                wait,
                toolbar,
                font: HFONT::default(),
                brush: HBRUSH::default(),
                dpi: GetDpiForWindow(hwnd).max(96) as i32,
                palette: Palette::new(false),
                dark: false,
                use_system: false,
                tab,
                tree,
                editor,
                empty: label("Откройте Markdown-файл или перетащите его в окно.")?,
                outline: label("Папка")?,
                status: label("")?,
                review_list,
                review_bounds: RECT::default(),
                document_top: 0,
                review_label: label("Комментарии")?,
                review_text: ctrl(
                    w!("EDIT"),
                    "",
                    WS_VSCROLL
                        | WS_BORDER
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                )?,
                review_actions: [
                    btn("Добавить", ADD_REVIEW)?,
                    btn("Править", EDIT_REVIEW)?,
                    btn("Решить", RESOLVE_REVIEW)?,
                    btn("Ещё", 204)?,
                ],
                draft_box: field(
                    0,
                    WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
                )?,
                draft_actions: [
                    btn("Применить", REVIEW_SAVE)?,
                    btn("Отмена", REVIEW_CANCEL)?,
                ],
                find_box: field(FIND, WINDOW_STYLE(ES_AUTOHSCROLL as u32))?,
                find_actions: [
                    btn("Назад", FIND_PREV)?,
                    btn("Далее", FIND_NEXT)?,
                    btn("Закрыть", CLOSE_FIND)?,
                ],
                docs: Vec::new(),
                active: None,
                parsed: Markdown::default(),
                view: Projection::default(),
                diagrams: Diagrams::default(),
                projection_safe: true,
                tree_open: false,
                review_open: false,
                find_open: false,
                find_headings: false,
                tree_entries: Vec::new(),
                review_indices: Vec::new(),
                active_review: None,
                draft: None,
                history: Vec::new(),
                zoom: 100,
                zoom_status: label("100%")?,
                workspace: None,
                content_width: 900,
            };
            send(editor, EM_EXLIMITTEXT, 0, i32::MAX as isize - 1);
            send(editor, EM_SETEVENTMASK, 0, ENM_CHANGE as isize);
            send(editor, EM_SETUNDOLIMIT, 0, 0);
            send(editor, EM_SETREADONLY, 1, 0);
            send(app.draft_box, EM_SETLIMITTEXT, 32_768, 0);
            DragAcceptFiles(hwnd, !wait);
            app.theme();
            app.fonts();
            app.layout();
            Ok(app)
        }
    }

    fn message(&mut self, id: u32, w: WPARAM, l: LPARAM) -> Option<isize> {
        match id {
            WM_COMMAND => {
                let control = w.0 & 0xffff;
                let notification = (w.0 >> 16) & 0xffff;
                if control == ID_DOCUMENT && notification == EN_CHANGE as usize {
                    unsafe {
                        let _ = PostMessageW(Some(self.hwnd), WM_SYNC, WPARAM(0), LPARAM(0));
                    }
                } else if control == RESULTS && notification == LBN_SELCHANGE as usize {
                    self.review_select(true);
                } else if notification == 0 {
                    self.command(control);
                }
                Some(0)
            }
            WM_NOTIFY if l.0 != 0 => {
                let header = unsafe { &*(l.0 as *const NMHDR) };
                if header.hwndFrom == self.tab && header.code == TCN_SELCHANGE {
                    let index = send(self.tab, TCM_GETCURSEL, 0, 0);
                    if index >= 0 {
                        self.switch(index as usize);
                    }
                }
                if header.hwndFrom == self.tree {
                    let notice = unsafe { &*(l.0 as *const NMTREEVIEWW) };
                    if header.code == TVN_ITEMEXPANDINGW && notice.action == TVE_EXPAND {
                        self.expand(notice.itemNew.hItem, notice.itemNew.lParam.0 as usize);
                    } else if header.code == TVN_SELCHANGEDW
                        && let Some(entry) = self.tree_entries.get(notice.itemNew.lParam.0 as usize)
                        && !entry.folder
                        && let Err(error) = self.open(entry.path.clone(), 0)
                    {
                        self.error("Не удалось открыть документ", &error);
                    }
                }
                Some(0)
            }
            WM_SYNC => {
                self.sync();
                Some(0)
            }
            WM_LINK => {
                self.follow_at(w.0);
                Some(0)
            }
            WM_DOC_SCROLL => {
                self.align_reviews();
                self.place_review_card();
                Some(0)
            }
            WM_REVIEW_HIT => {
                self.review_hit(w.0);
                Some(0)
            }
            WM_SIZE => {
                self.layout();
                Some(0)
            }
            WM_DPICHANGED => {
                self.dpi = (w.0 & 0xffff) as i32;
                self.fonts();
                let rect = unsafe { &*(l.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                self.layout();
                Some(0)
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                self.sync();
                self.store_position();
                self.theme();
                self.refresh_viewport();
                Some(0)
            }
            WM_DROPFILES => {
                self.drop_files(HDROP(w.0 as *mut _));
                Some(0)
            }
            WM_CLOSE => {
                self.sync();
                for i in 0..self.docs.len() {
                    if !self.confirm_close(i) {
                        return Some(0);
                    }
                }
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
                Some(0)
            }
            WM_CONTEXTMENU => {
                self.context_menu(w.0, l.0);
                Some(0)
            }
            WM_ERASEBKGND => {
                let mut rect = RECT::default();
                unsafe {
                    let _ = GetClientRect(self.hwnd, &mut rect);
                    FillRect(HDC(w.0 as *mut _), &rect, self.brush);
                }
                Some(1)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                unsafe {
                    let dc = HDC(w.0 as *mut _);
                    SetTextColor(dc, Palette::rgb(self.palette.text));
                    SetBkColor(dc, Palette::rgb(self.palette.frame));
                }
                Some(self.brush.0 as isize)
            }
            _ => None,
        }
    }

    fn mode(&self) -> usize {
        self.active.map_or(0, |i| self.docs[i].mode)
    }
    fn dip(&self, size: i32) -> i32 {
        size * self.dpi / 96
    }

    fn layout(&mut self) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rect);
            let d = |value: i32| value * self.dpi / 96;
            let width = rect.right;
            let height = rect.bottom;
            let mut x = d(12);
            let mut y = d(10);
            for (button, item_width) in &self.toolbar {
                let button_width = d(*item_width);
                if x + button_width + d(12) > width && x > d(12) {
                    x = d(12);
                    y += d(34);
                }
                move_to(*button, x, y, button_width, d(29), true);
                x += button_width + d(5);
            }
            y += d(39);
            if self.find_open {
                let search_width = (width - d(280)).clamp(d(100), d(460));
                move_to(self.find_box, d(12), y, search_width, d(26), true);
                let mut left = d(19) + search_width;
                for button in self.find_actions {
                    move_to(button, left, y, d(77), d(26), true);
                    left += d(82);
                }
                y += d(35);
            } else {
                move_to(self.find_box, 0, 0, 0, 0, false);
                for button in self.find_actions {
                    move_to(button, 0, 0, 0, 0, false);
                }
            }
            move_to(
                self.tab,
                d(12),
                y,
                width - d(24),
                d(30),
                self.active.is_some() && !self.wait,
            );
            if self.active.is_some() && !self.wait {
                y += d(30);
            }
            let body = (height - y - d(27)).max(d(60));
            let tree_width = if self.tree_open && !self.wait {
                d(220).min(width / 3)
            } else {
                0
            };
            let review_width = if self.review_open && self.active.is_some() {
                d(306).min(width / 2)
            } else {
                0
            };
            move_to(
                self.outline,
                d(13),
                y + d(8),
                tree_width - d(16),
                d(20),
                tree_width > 0,
            );
            move_to(
                self.tree,
                d(9),
                y + d(34),
                tree_width - d(14),
                body - d(38),
                tree_width > 0,
            );
            let left = tree_width + d(9);
            let central = (width - left - review_width - d(9)).max(d(100));
            move_to(self.editor, left, y, central, body, self.active.is_some());
            move_to(
                self.empty,
                left + d(25),
                y + d(39),
                central - d(32),
                d(50),
                self.active.is_none(),
            );
            self.content_width = central;
            let margin = ((central - d(950)) / 2).max(d(24));
            margins(self.editor, central, body, margin, self.dpi);
            let rx = width - review_width + d(2);
            let rw = review_width - d(14);
            let visible = review_width > 0;
            move_to(self.review_label, rx, y + d(8), rw, d(22), visible);
            let button_width = (rw - d(12)) / 4;
            for (i, button) in self.review_actions.iter().enumerate() {
                move_to(
                    *button,
                    rx + i as i32 * (button_width + d(4)),
                    y + d(36),
                    button_width,
                    d(26),
                    visible,
                );
            }
            self.document_top = y;
            let list_height = (body / 4).clamp(d(58), d(125));
            move_to(self.review_list, rx, y + d(72), rw, list_height, visible);
            let content_y = y + d(84) + list_height;
            let content_height = (body - list_height - d(100)).max(d(40));
            self.review_bounds = RECT {
                left: rx,
                top: content_y,
                right: rx + rw,
                bottom: content_y + content_height,
            };
            self.place_review_card();
            move_to(
                self.draft_box,
                rx,
                content_y,
                rw,
                (content_height - d(36)).max(d(40)),
                visible && self.draft.is_some(),
            );
            for (i, button) in self.draft_actions.iter().enumerate() {
                move_to(
                    *button,
                    rx + i as i32 * d(106),
                    content_y + content_height - d(28),
                    d(100),
                    d(25),
                    visible && self.draft.is_some(),
                );
            }
            move_to(
                self.status,
                d(13),
                height - d(21),
                width - d(97),
                d(19),
                true,
            );
            move_to(
                self.zoom_status,
                width - d(65),
                height - d(21),
                d(60),
                d(19),
                true,
            );
        }
    }

    fn fonts(&mut self) {
        unsafe {
            let font = CreateFontW(
                -self.dip(14),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            for hwnd in self.controls() {
                send(hwnd, WM_SETFONT, font.0 as usize, 1);
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
            self.font = font;
        }
    }

    fn controls(&self) -> impl Iterator<Item = HWND> + '_ {
        self.toolbar
            .iter()
            .map(|(hwnd, _)| *hwnd)
            .chain([
                self.tab,
                self.tree,
                self.empty,
                self.outline,
                self.status,
                self.zoom_status,
                self.review_label,
                self.review_list,
                self.review_text,
                self.draft_box,
                self.find_box,
            ])
            .chain(self.review_actions)
            .chain(self.draft_actions)
            .chain(self.find_actions)
    }

    fn theme(&mut self) {
        self.dark = self.use_system && system_dark();
        self.palette = Palette::new(self.dark);
        unsafe {
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
            self.brush = CreateSolidBrush(Palette::rgb(self.palette.frame));
            let dark = self.dark as i32;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as _,
                size_of_val(&dark) as u32,
            );
            let theme = if self.dark {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            };
            for hwnd in self.controls() {
                let _ = SetWindowTheme(hwnd, theme, PCWSTR::null());
            }
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
        send(
            self.tree,
            TVM_SETBKCOLOR,
            0,
            Palette::rgb(self.palette.frame).0 as isize,
        );
        send(
            self.tree,
            TVM_SETTEXTCOLOR,
            0,
            Palette::rgb(self.palette.text).0 as isize,
        );
        send(
            self.tree,
            TVM_SETLINECOLOR,
            0,
            Palette::rgb(self.palette.line).0 as isize,
        );
        send(
            self.editor,
            EM_SETBKGNDCOLOR,
            0,
            Palette::rgb(self.palette.canvas).0 as isize,
        );
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
        }
    }
}

fn child(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    title: &str,
    style: WINDOW_STYLE,
    id: usize,
) -> Result<HWND> {
    Ok(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(wide(title).as_ptr()),
            WS_CHILD | style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(HMENU(id as *mut _)),
            Some(instance),
            None,
        )
        .map_err(|e| format!("CreateWindowExW {}: {e}", class.display()))?
    })
}

fn move_to(hwnd: HWND, x: i32, y: i32, w: i32, h: i32, show: bool) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            w.max(0),
            h.max(0),
            SWP_NOZORDER | SWP_NOACTIVATE | if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW },
        );
    }
}

fn system_dark() -> bool {
    let mut value = 1u32;
    let mut size = size_of_val(&value) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 0
}

#[cfg(test)]
mod tests;
