mod actions;
mod appearance;
mod atoms;
mod cards;
mod cells;
mod clipboard;
mod layout;
mod lists;
mod molecules;
mod navigation;
mod remote_images;
mod review_panel;
use atoms::control_button;
use molecules::DocumentBar;
pub(super) use navigation::decode_url;
use navigation::menu_bar;
pub(super) use remote_images::RemoteImages;

use std::{
    cell::RefCell,
    ffi::OsString,
    fs,
    ops::Range,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use pane_md::{
    diff, edit_source,
    markdown::Markdown,
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
    file::{
        Document, FolderEntry, LinkHistory, LinkPosition, Viewport, folder_entries, markdown_path,
    },
    i18n::{self, Label as L, tr},
    rich::*,
    settings,
    theme::{EditorAppearance, Palette},
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
const OPEN_LINK: usize = 137;
const FIND_PREV: usize = 138;
const CLOSE_FIND: usize = 139;
const PARAGRAPH: usize = 140;
const LINE_BREAK: usize = 141;
const SHOW_SOURCE: usize = 142;
const MODES: usize = 150;
const RESULTS: usize = 170;
const OUTLINE: usize = 172;
const OUTLINE_LIST: usize = 173;
const FAMILY: usize = 174;
const DISPLAY: usize = 175;
const READ: usize = 176;
const COMMENTS: usize = 177;
const EDITING: usize = 178;
const VIEW_RICH: usize = 187;
const VIEW_SOURCE: usize = 188;
const CARD_MENU_BASE: usize = 500;
const REVIEW_COMMIT: usize = 189;
const REVIEW_DETAILS: usize = 190;
const LINT_NEXT: usize = 191;
const LINT_PREV: usize = 192;
const MAKE_TASK: usize = 193;
const TOGGLE_TASK: usize = 194;
const NUMBERED_LIST: usize = 195;
const APPEARANCE: usize = 240;
const RICH_FONT: usize = 241;
const SOURCE_FONT: usize = 242;
const PAPER_COLOR: usize = 243;
const TEXT_COLOR: usize = 244;
const LINK_COLOR: usize = 245;
const RESET_APPEARANCE: usize = 246;
const INSERT_IMAGE: usize = 247;
const SHOW_REPLACE: usize = 248;
const REPLACE_ONE: usize = 249;
const REPLACE_ALL: usize = 250;
const FIND_OPTIONS: usize = 251;
const MATCH_CASE: usize = 252;
const WHOLE_WORD: usize = 253;
const REFRESH_FILES: usize = 254;
const NEW_NOTE: usize = 255;
const FORWARD_LINK: usize = 256;
const HEADING_FIRST: usize = 220;
const HEADING_LAST: usize = HEADING_FIRST + 5;
const REVIEW_ALL: usize = 181;
const REVIEW_COLLAPSE: usize = 182;
const THEME_LIGHT: usize = 183;
const THEME_DARK: usize = 184;
const LANG_RU: usize = 185;
const LANG_EN: usize = 186;
const COPY_SELECTION: usize = 206;
const CUT_SELECTION: usize = 208;
const CARD_BASE: usize = 400;
const CARD_SLOTS: usize = 96;
const ID_DOCUMENT: usize = 180;
const WM_SYNC: u32 = WM_APP + 1;
const WM_LINK: u32 = WM_APP + 2;
const WM_DOC_SCROLL: u32 = WM_APP + 3;
const WM_REVIEW_HIT: u32 = WM_APP + 4;
const WM_SELECTION_MENU: u32 = WM_APP + 5;
const WM_TAB_CLOSE: u32 = WM_APP + 6;
const WM_TASK: u32 = WM_APP + 7;
pub(super) const WM_DIAGRAM_READY: u32 = WM_APP + 8;
const REFRESH_SIDEBARS: usize = 1;
const SS_OWNERDRAW: WINDOW_STYLE = WINDOW_STYLE(0x0d);

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
        let icon_id = PCWSTR(std::ptr::without_provenance(1));
        let icon =
            LoadIconW(Some(instance), icon_id).or_else(|_| LoadIconW(None, IDI_APPLICATION))?;
        let small_icon = LoadImageW(
            Some(instance),
            icon_id,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_SHARED,
        )
        .map_or(icon, |handle| HICON(handle.0));
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hIcon: icon,
            hIconSm: small_icon,
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("pane-md.window"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("pane-md"),
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
                    app.error(tr(L::FileError), &error);
                    let _ = DestroyWindow(hwnd);
                    return Err(error);
                }
                app.error(tr(L::OpenError), &error);
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
        // The launcher can pass a minimized show state on the first ShowWindow call.
        if wait && IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = UpdateWindow(hwnd);
        if state.borrow().active.is_some() {
            if wait {
                let _ = SetForegroundWindow(hwnd);
            }
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
            let editor = state.borrow().editor;
            if msg.hwnd == editor && clipboard::is_paste_key(&msg) {
                send(editor, WM_PASTE, 0, 0);
                continue;
            }
            if msg.hwnd == editor
                && msg.message == WM_KEYDOWN
                && msg.wParam.0 == usize::from(VK_TAB.0)
                && GetKeyState(VK_CONTROL.0.into()) >= 0
                && GetKeyState(VK_MENU.0.into()) >= 0
                && state
                    .borrow_mut()
                    .move_table_cell(GetKeyState(VK_SHIFT.0.into()) < 0)
            {
                continue;
            }
            let command = shortcut(&msg, &state.borrow());
            if let Some(command) = command {
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
        if let Some(result) = navigation::menu_message(hwnd, message, w, l) {
            return result;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if !pointer.is_null()
            && let Ok(mut app) = (*pointer).try_borrow_mut()
            && let Some(result) = app.message(message, w, l)
        {
            return LRESULT(result);
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
        if let Some(result) = lists::pointer(frame, message, w, l) {
            return result;
        }
        if (message == WM_MOUSEMOVE && w.0 & 1 == 0) || message == WM_LBUTTONUP {
            let point = POINT {
                x: l.0 as i16 as i32,
                y: (l.0 >> 16) as i16 as i32,
            };
            let pointer = GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>;
            if !pointer.is_null()
                && let Ok(app) = (*pointer).try_borrow()
                && (app.mode() == 0 || GetKeyState(VK_CONTROL.0.into()) < 0)
                && let Some(at) = hit(hwnd, point)
                && app.link_at(at).is_some()
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
            let pointer = GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>;
            if let Some(state) = pointer.as_ref()
                && let Ok(mut app) = state.try_borrow_mut()
                && app.mode() == 2
            {
                if let Some(text) = clipboard::clipboard_text(hwnd) {
                    app.paste_visual(&text);
                }
                return LRESULT(0);
            }
            return LRESULT(send(
                hwnd,
                EM_PASTESPECIAL,
                clipboard::UNICODE_TEXT as usize,
                0,
            ));
        }
        let edit_cell = match message {
            WM_CHAR if w.0 == 8 => Some(0),
            WM_CHAR if (32..=u16::MAX as usize).contains(&w.0) => Some(w.0 as u16),
            WM_KEYDOWN if w.0 == usize::from(VK_DELETE.0) => Some(0),
            _ => None,
        };
        if let Some(unit) = edit_cell
            && let Some(state) =
                (GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>).as_ref()
            && let Ok(mut app) = state.try_borrow_mut()
            && app.edit_selected_cells(unit)
        {
            return LRESULT(0);
        }
        let result = DefSubclassProc(hwnd, message, w, l);
        if message == WM_LBUTTONUP {
            let span = selection(hwnd);
            if !span.is_empty()
                && char_point(hwnd, span.start).y != char_point(hwnd, span.end - 1).y
            {
                let mut point = POINT {
                    x: l.0 as i16 as i32,
                    y: (l.0 >> 16) as i16 as i32,
                };
                let _ = ClientToScreen(hwnd, &mut point);
                let coordinates = (point.x as u16 as usize) | ((point.y as u16 as usize) << 16);
                let _ = PostMessageW(
                    Some(frame),
                    WM_SELECTION_MENU,
                    WPARAM(0),
                    LPARAM(coordinates as isize),
                );
            }
        }
        if message == WM_SETTEXT {
            // A multiline control can accept WM_SETTEXT without sending EN_CHANGE.
            send(
                frame,
                WM_COMMAND,
                ID_DOCUMENT | ((EN_CHANGE as usize) << 16),
                hwnd.0 as isize,
            );
        }
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
    if alt && !ctrl && !shift {
        match key {
            key if key == VK_LEFT.0 => return Some(BACK),
            key if key == VK_RIGHT.0 => return Some(FORWARD_LINK),
            _ => {}
        }
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
    if key == VK_F5.0 && !ctrl && !alt && !shift && app.workspace.is_some() {
        return Some(REFRESH_FILES);
    }
    if key == VK_ESCAPE.0 && app.find_open {
        return Some(CLOSE_FIND);
    }
    if key == VK_ESCAPE.0 {
        if app.draft.is_some() {
            return Some(REVIEW_CANCEL);
        }
        if app.reviews_visible() && app.review_expanded {
            return Some(REVIEW_COLLAPSE);
        }
    }
    if ctrl && key == VK_RETURN.0 && app.draft.is_some() {
        return Some(REVIEW_SAVE);
    }
    if key == VK_F8.0 && !ctrl && !alt {
        return Some(if shift { LINT_PREV } else { LINT_NEXT });
    }
    if key == VK_RETURN.0 && msg.hwnd == app.find_box {
        return Some(if shift { FIND_PREV } else { FIND_NEXT });
    }
    if key == VK_RETURN.0 && msg.hwnd == app.replace_box && app.replace_open {
        return Some(if ctrl { REPLACE_ALL } else { REPLACE_ONE });
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
            0x4e if !alt && !shift && !app.wait => return Some(NEW_NOTE),
            0x4f => return Some(if shift { FOLDER } else { OPEN }),
            0x53 => return Some(if shift { SAVE_AS } else { SAVE }),
            0x57 => return Some(CLOSE),
            0x46 => return Some(FIND),
            0x48 if !shift && !alt && app.mode() == 3 => return Some(SHOW_REPLACE),
            0x4d if alt => return Some(ADD_REVIEW),
            0x45 if alt => return Some(EDIT_REVIEW),
            0x44 if alt => return Some(DELETE_REVIEW),
            0x58 if msg.hwnd == app.editor => return Some(CUT_SELECTION),
            0x42 if msg.hwnd == app.editor => return Some(BOLD),
            0x4c if shift && !alt && msg.hwnd == app.editor => return Some(MAKE_TASK),
            0x20 if !shift && !alt && msg.hwnd == app.editor => return Some(TOGGLE_TASK),
            0x4b if !alt && msg.hwnd == app.editor && app.mode() == 2 => {
                return Some(if shift { INSERT_IMAGE } else { LINK });
            }
            0x49 if msg.hwnd == app.editor => return Some(ITALIC),
            0x5a if msg.hwnd == app.editor => return Some(if shift { REDO } else { UNDO }),
            0x59 if msg.hwnd == app.editor => return Some(REDO),
            0xbb | 0x6b => return Some(ZOOM_IN),
            0xbd | 0x6d => return Some(ZOOM_OUT),
            0x30 => return Some(ZOOM_RESET),
            0x31..=0x36 if alt && !shift && msg.hwnd == app.editor => {
                return Some(HEADING_FIRST + (key - 0x31) as usize);
            }
            0x31..=0x34 if !alt && !shift => return Some(MODES + (key - 0x31) as usize),
            0x09 => return Some(if shift { 202 } else { 201 }),
            _ => {}
        }
    }
    None
}

struct TreeEntry {
    item: HTREEITEM,
    path: PathBuf,
    folder: bool,
    loaded: bool,
}

struct Draft {
    existing: Option<String>,
    capture: Option<(usize, String, review::Anchor)>,
    format: Option<(usize, Range<usize>)>,
}

struct App {
    hwnd: HWND,
    wait: bool,
    bar: DocumentBar,
    outline_toggle: HWND,
    review_toggle: HWND,
    open_tab: HWND,
    source_lines: Vec<usize>,
    font: HFONT,
    strong_font: HFONT,
    brush: HBRUSH,
    surface: HBRUSH,
    caption_surface: HBRUSH,
    dpi: i32,
    dark: bool,
    theme_choice: usize,
    palette: Palette,
    appearance: EditorAppearance,
    tab: HWND,
    tab_close_down: Option<usize>,
    task_down: Option<(usize, usize)>,
    tree: HWND,
    editor: HWND,
    empty: HWND,
    outline: HWND,
    outline_list: HWND,
    status: HWND,
    review_list: HWND,
    review_label: HWND,
    review_text: HWND,
    review_bounds: RECT,
    document_top: i32,
    review_actions: [HWND; 4],
    review_all_button: HWND,
    review_collapse: HWND,
    review_cards: Vec<cells::ReviewCard>,
    draft_quote: HWND,
    draft_frame: Option<RECT>,
    card_rows: Vec<usize>,
    review_expanded: bool,
    review_all: bool,
    draft_box: HWND,
    draft_actions: [HWND; 2],
    find_box: HWND,
    find_actions: [HWND; 4],
    replace_box: HWND,
    replace_actions: [HWND; 2],
    replace_open: bool,
    search_options: pane_md::search::Options,
    docs: Vec<Document>,
    active: Option<usize>,
    parsed: Markdown,
    view: Projection,
    diagrams: Diagrams,
    remote_images: RemoteImages,
    projection_safe: bool,
    sync_pending: bool,
    pending_cell_input: Option<(Range<usize>, u16)>,
    source_parse_dirty: bool,
    tree_open: bool,
    outline_open: bool,
    review_open: bool,
    find_open: bool,
    find_headings: bool,
    tree_entries: Vec<TreeEntry>,
    review_indices: Vec<usize>,
    active_review: Option<String>,
    draft: Option<Draft>,
    history: LinkHistory<(Viewport, i32)>,
    zoom: i32,
    zoom_buttons: [HWND; 3],
    workspace: Option<PathBuf>,
    content_width: i32,
    editor_metrics: (i32, i32, i32),
    editor_placement: (i32, i32, i32, i32, bool),
}

impl App {
    fn new(hwnd: HWND, instance: HINSTANCE, wait: bool) -> Result<Self> {
        unsafe {
            let theme = settings::read_preference("Theme")
                .map(|value| value as usize)
                .filter(|value| matches!(*value, THEME | THEME_LIGHT | THEME_DARK))
                .unwrap_or(THEME_LIGHT);
            let menu = menu_bar(theme)?;
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
            let tab = child(
                hwnd,
                instance,
                WC_TABCONTROLW,
                "",
                WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(TCS_OWNERDRAWFIXED),
                0,
            )?;
            let _ = SetWindowSubclass(tab, Some(molecules::tab_proc), 1, hwnd.0 as usize);
            let tree = child(
                hwnd,
                instance,
                WC_TREEVIEWW,
                "",
                WS_TABSTOP
                    | WINDOW_STYLE(
                        TVS_HASBUTTONS
                            | TVS_FULLROWSELECT
                            | TVS_SHOWSELALWAYS
                            | TVS_DISABLEDRAGDROP,
                    ),
                0,
            )?;
            let ctrl = |class, label, style, id| child(hwnd, instance, class, label, style, id);
            let btn = |label, id| control_button(hwnd, instance, label, id);
            let label = |text| ctrl(w!("STATIC"), text, SS_OWNERDRAW, 0);
            let field = |id, style| ctrl(w!("EDIT"), "", WS_TABSTOP | WS_BORDER | style, id);
            let review_list = ctrl(
                w!("LISTBOX"),
                "",
                WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32),
                RESULTS,
            )?;
            let mut app = Self {
                hwnd,
                wait,
                bar: DocumentBar::new(hwnd, instance)?,
                outline_toggle: btn(tr(L::Outline), OUTLINE)?,
                review_toggle: btn(tr(L::Comments), REVIEWS)?,
                open_tab: btn(tr(L::OpenTab), OPEN)?,
                source_lines: Vec::new(),
                font: HFONT::default(),
                strong_font: HFONT::default(),
                brush: HBRUSH::default(),
                surface: HBRUSH::default(),
                caption_surface: HBRUSH::default(),
                dpi: GetDpiForWindow(hwnd).max(96) as i32,
                palette: Palette::new(false),
                appearance: settings::load_appearance(),
                dark: false,
                theme_choice: theme,
                tab,
                tab_close_down: None,
                task_down: None,
                tree,
                editor,
                empty: label(tr(L::EmptyEditor))?,
                outline: ctrl(w!("STATIC"), tr(L::Files), SS_OWNERDRAW, 0)?,
                outline_list: ctrl(
                    w!("LISTBOX"),
                    "",
                    WS_TABSTOP
                        | WS_VSCROLL
                        | WINDOW_STYLE(
                            (LBS_NOTIFY
                                | LBS_NOINTEGRALHEIGHT
                                | LBS_OWNERDRAWFIXED
                                | LBS_HASSTRINGS) as u32,
                        ),
                    OUTLINE_LIST,
                )?,
                status: ctrl(w!("STATIC"), "", SS_OWNERDRAW, 0)?,
                review_list,
                review_bounds: RECT::default(),
                document_top: 0,
                review_label: label(tr(L::Comments))?,
                review_text: ctrl(
                    w!("EDIT"),
                    "",
                    WS_TABSTOP
                        | WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
                    0,
                )?,
                review_actions: [
                    btn(tr(L::AddComment), ADD_REVIEW)?,
                    btn(tr(L::ReviewEdit), EDIT_REVIEW)?,
                    btn(tr(L::Delete), DELETE_REVIEW)?,
                    btn(tr(L::More), 204)?,
                ],
                review_all_button: btn(tr(L::All), REVIEW_ALL)?,
                review_collapse: btn(tr(L::Collapse), REVIEW_COLLAPSE)?,
                review_cards: Vec::new(),
                draft_quote: label("")?,
                draft_frame: None,
                card_rows: Vec::new(),
                review_expanded: false,
                review_all: false,
                draft_box: field(
                    0,
                    WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
                )?,
                draft_actions: [
                    btn(tr(L::SaveComment), REVIEW_COMMIT)?,
                    btn(tr(L::Cancel), REVIEW_CANCEL)?,
                ],
                find_box: field(FIND, WINDOW_STYLE(ES_AUTOHSCROLL as u32))?,
                find_actions: [
                    btn(tr(L::Back), FIND_PREV)?,
                    btn(tr(L::Next), FIND_NEXT)?,
                    btn(tr(L::SearchOptions), FIND_OPTIONS)?,
                    btn(tr(L::Close), CLOSE_FIND)?,
                ],
                replace_box: field(0, WINDOW_STYLE(ES_AUTOHSCROLL as u32))?,
                replace_actions: [
                    btn(tr(L::ReplaceOne), REPLACE_ONE)?,
                    btn(tr(L::ReplaceAll), REPLACE_ALL)?,
                ],
                replace_open: false,
                search_options: pane_md::search::Options::default(),
                docs: Vec::new(),
                active: None,
                parsed: Markdown::default(),
                view: Projection::default(),
                diagrams: Diagrams::default(),
                remote_images: RemoteImages::default(),
                projection_safe: true,
                sync_pending: false,
                pending_cell_input: None,
                source_parse_dirty: false,
                tree_open: false,
                outline_open: false,
                review_open: !wait,
                find_open: false,
                find_headings: false,
                tree_entries: Vec::new(),
                review_indices: Vec::new(),
                active_review: None,
                draft: None,
                history: LinkHistory::default(),
                zoom: 100,
                zoom_buttons: [
                    btn("−", ZOOM_OUT)?,
                    btn("100%", ZOOM_RESET)?,
                    btn("+", ZOOM_IN)?,
                ],
                workspace: None,
                content_width: 900,
                editor_metrics: (0, 0, 0),
                editor_placement: (0, 0, 0, 0, false),
            };
            send(editor, EM_EXLIMITTEXT, 0, i32::MAX as isize - 1);
            send(editor, EM_SETEVENTMASK, 0, ENM_CHANGE as isize);
            send(editor, EM_SETUNDOLIMIT, 0, 0);
            send(editor, EM_SETREADONLY, 1, 0);
            // Keep prose wrapped to the formatting rectangle as the workspace resizes.
            send(editor, EM_SETTARGETDEVICE, 0, 0);
            send(app.draft_box, EM_SETLIMITTEXT, i32::MAX as usize - 1, 0);
            DragAcceptFiles(hwnd, !wait);
            app.theme();
            app.fonts();
            app.layout();
            Ok(app)
        }
    }

    fn message(&mut self, id: u32, w: WPARAM, l: LPARAM) -> Option<isize> {
        match id {
            WM_INITMENUPOPUP => {
                let popup = HMENU(w.0 as *mut _);
                for (command, forward) in [(BACK, false), (FORWARD_LINK, true)] {
                    let enabled = self.draft.is_none() && self.history.available(forward);
                    let flags = if enabled { MF_ENABLED } else { MF_GRAYED };
                    unsafe {
                        let _ = EnableMenuItem(popup, command as u32, MF_BYCOMMAND | flags);
                    }
                }
                None
            }
            WM_COMMAND => {
                let control = w.0 & 0xffff;
                let notification = (w.0 >> 16) & 0xffff;
                if control == ID_DOCUMENT && notification == EN_CHANGE as usize {
                    if !self.sync_pending {
                        self.sync_pending = true;
                        unsafe {
                            let _ = PostMessageW(Some(self.hwnd), WM_SYNC, WPARAM(0), LPARAM(0));
                        }
                    }
                } else if control == RESULTS && notification == LBN_SELCHANGE as usize {
                    self.flush_edit();
                    self.parse_source_edits();
                    self.review_select(true);
                } else if control == OUTLINE_LIST && notification == LBN_SELCHANGE as usize {
                    self.jump_outline();
                } else if notification == 0 {
                    let command = if control == OUTLINE
                        && l.0 == self.outline_toggle.0 as isize
                        && self.tree_open
                        && !self.outline_open
                        && self.nav_is_visible()
                    {
                        TREE
                    } else {
                        control
                    };
                    self.command(command);
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
                        self.error(tr(L::OpenError), &error);
                    }
                }
                Some(0)
            }
            WM_SYNC => {
                self.flush_edit();
                Some(0)
            }
            WM_TIMER if w.0 == REFRESH_SIDEBARS => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), REFRESH_SIDEBARS);
                }
                self.parse_source_edits();
                self.refresh_outline();
                Some(0)
            }
            WM_LINK => {
                self.follow_at(w.0);
                Some(0)
            }
            WM_TASK => {
                if self.active == usize::try_from(l.0).ok() {
                    self.toggle_task(w.0);
                }
                Some(0)
            }
            WM_DIAGRAM_READY => {
                self.refresh_diagrams();
                Some(0)
            }
            WM_DOC_SCROLL => {
                self.place_review_card();
                self.invalidate_ruler();
                Some(0)
            }
            WM_REVIEW_HIT => {
                self.review_hit(w.0);
                Some(0)
            }
            WM_SELECTION_MENU => {
                self.context_menu(self.editor.0 as usize, l.0);
                Some(0)
            }
            WM_TAB_CLOSE => {
                self.flush_edit();
                self.close_tab(w.0);
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
                if self.theme_choice == THEME && self.dark != system_dark() {
                    self.store_position();
                    self.theme();
                    self.refresh_viewport();
                }
                Some(0)
            }
            WM_DROPFILES => {
                self.drop_files(HDROP(w.0 as *mut _));
                Some(0)
            }
            WM_CLOSE => {
                self.flush_edit();
                if self.draft.is_some() {
                    self.status(Some(tr(L::FinishDraft)));
                    return Some(0);
                }
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
            WM_GETMINMAXINFO if l.0 != 0 => {
                let limits = unsafe { &mut *(l.0 as *mut MINMAXINFO) };
                limits.ptMinTrackSize.x = self.dip(530);
                limits.ptMinTrackSize.y = self.dip(380);
                Some(0)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                unsafe {
                    let dc = BeginPaint(self.hwnd, &mut paint);
                    self.paint_layout(dc, paint.rcPaint);
                    let _ = EndPaint(self.hwnd, &paint);
                }
                Some(0)
            }
            WM_DRAWITEM if l.0 != 0 => {
                let item = unsafe { &*(l.0 as *const DRAWITEMSTRUCT) };
                if item.CtlType == ODT_STATIC {
                    self.draw_label(item);
                } else if item.hwndItem == self.tab {
                    self.draw_tab(item);
                } else if item.hwndItem == self.outline_list {
                    self.draw_outline(item);
                } else if (CARD_BASE..CARD_BASE + CARD_SLOTS).contains(&(item.CtlID as usize)) {
                    self.draw_review_card(item);
                } else {
                    self.draw_button(item);
                }
                Some(1)
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
                let control = HWND(l.0 as *mut _);
                let paper = [
                    self.draft_box,
                    self.find_box,
                    self.replace_box,
                    self.draft_quote,
                ]
                .contains(&control)
                    || control == self.review_text && !self.review_indices.is_empty();
                unsafe {
                    let dc = HDC(w.0 as *mut _);
                    SetTextColor(dc, Palette::rgb(self.palette.text));
                    SetBkColor(
                        dc,
                        Palette::rgb(if paper {
                            self.palette.canvas
                        } else {
                            self.palette.frame
                        }),
                    );
                }
                Some(if paper { self.surface } else { self.brush }.0 as isize)
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

    fn fonts(&mut self) {
        unsafe {
            let font = atoms::ui_font(self.dip(14));
            let strong = atoms::ui_font_weight(self.dip(15), FW_SEMIBOLD.0);
            for hwnd in self.controls() {
                send(hwnd, WM_SETFONT, font.0 as usize, 1);
            }
            send(
                self.tab,
                TCM_SETPADDING,
                0,
                (self.dip(27) | (self.dip(5) << 16)) as isize,
            );
            send(
                self.tab,
                TCM_SETITEMSIZE,
                0,
                (self.dip(106) | (self.dip(30) << 16)) as isize,
            );
            send(self.tree, TVM_SETITEMHEIGHT, self.dip(24) as usize, 0);
            for hwnd in [self.outline_list, self.review_list] {
                send(hwnd, LB_SETITEMHEIGHT, 0, self.dip(30) as isize);
            }
            for hwnd in [
                self.review_text,
                self.draft_box,
                self.find_box,
                self.replace_box,
            ] {
                let margin = self.dip(7) as isize;
                send(
                    hwnd,
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
                    margin | (margin << 16),
                );
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
            if !self.strong_font.is_invalid() {
                let _ = DeleteObject(self.strong_font.into());
            }
            self.font = font;
            self.strong_font = strong;
        }
    }

    fn controls(&self) -> impl Iterator<Item = HWND> + '_ {
        [self.outline_toggle, self.open_tab]
            .into_iter()
            .chain(self.bar.controls())
            .chain([
                self.review_toggle,
                self.draft_quote,
                self.tab,
                self.tree,
                self.empty,
                self.outline,
                self.status,
                self.outline_list,
                self.review_label,
                self.review_list,
                self.review_text,
                self.draft_box,
                self.find_box,
                self.replace_box,
            ])
            .chain(self.review_actions)
            .chain([self.review_all_button, self.review_collapse])
            .chain(
                self.review_cards
                    .iter()
                    .flat_map(|card| [card.body, card.menu]),
            )
            .chain(self.draft_actions)
            .chain(self.find_actions)
            .chain(self.replace_actions)
            .chain(self.zoom_buttons)
    }

    fn theme_menu(&self) {
        if !self.caption_surface.is_invalid() {
            unsafe {
                let info = MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    hbrBack: self.caption_surface,
                    ..Default::default()
                };
                let _ = SetMenuInfo(GetMenu(self.hwnd), &info);
                let _ = DrawMenuBar(self.hwnd);
            }
        }
    }

    fn theme(&mut self) {
        self.dark = self.theme_choice == THEME_DARK || self.theme_choice == THEME && system_dark();
        self.palette = Palette::new(self.dark);
        unsafe {
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
            if !self.surface.is_invalid() {
                let _ = DeleteObject(self.surface.into());
            }
            self.brush = CreateSolidBrush(Palette::rgb(self.palette.frame));
            self.surface = CreateSolidBrush(Palette::rgb(self.palette.canvas));
            let old_caption = self.caption_surface;
            self.caption_surface = CreateSolidBrush(Palette::rgb(self.palette.caption));
            self.theme_menu();
            if !old_caption.is_invalid() {
                let _ = DeleteObject(old_caption.into());
            }
            let dark = self.dark as i32;
            let caption = Palette::rgb(self.palette.caption);
            let caption_text = Palette::rgb(self.palette.on_caption);
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_CAPTION_COLOR,
                &caption as *const _ as _,
                size_of_val(&caption) as u32,
            );
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_TEXT_COLOR,
                &caption_text as *const _ as _,
                size_of_val(&caption_text) as u32,
            );
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
        self.document_background();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            if !self.brush.is_invalid() {
                let _ = DeleteObject(self.brush.into());
            }
            if !self.surface.is_invalid() {
                let _ = DeleteObject(self.surface.into());
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
            if !self.strong_font.is_invalid() {
                let _ = DeleteObject(self.strong_font.into());
            }
            if !self.caption_surface.is_invalid() {
                let _ = DeleteObject(self.caption_surface.into());
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
