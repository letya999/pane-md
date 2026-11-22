use super::*;

impl App {
    pub(super) fn show_find(&mut self, headings: bool) {
        if self.active.is_none() {
            return;
        }
        if headings != self.find_headings {
            set_text(self.find_box, "");
        }
        self.find_headings = headings;
        self.replace_open = false;
        self.update_find_hint();
        self.find_open = true;
        self.layout();
        let span = selection(self.editor);
        if !headings
            && !span.is_empty()
            && span.len() < 512
            && let Some(units) = self.view.text.get(span)
            && let Ok(text) = String::from_utf16(units)
            && !text.contains(['\r', '\n', '\t', '\u{0007}'])
        {
            set_text(self.find_box, &text);
        }
        send(self.find_box, EM_SETSEL, 0, -1);
        unsafe {
            let _ = SetFocus(Some(self.find_box));
        }
    }

    pub(super) fn show_replace(&mut self) {
        if self.mode() != 3 {
            return;
        }
        self.show_find(false);
        self.replace_open = true;
        self.layout();
    }

    pub(super) fn replace_matches(&mut self, all: bool) {
        let Some(index) = self.active else { return };
        if self.mode() != 3 {
            return;
        }
        let query = get_text(self.find_box);
        if query.is_empty() {
            return;
        }
        let replacement = get_text(self.replace_box);
        let selected = (!all).then(|| {
            self.view
                .source_selection(&self.parsed, selection(self.editor))
        });
        if let Some((caret, count)) =
            self.docs[index].replace_source(&query, &replacement, self.search_options, selected)
        {
            self.refresh(caret);
            let notice = tr(L::ReplacedCount).replace("{count}", &count.to_string());
            self.status(Some(&notice));
        } else {
            self.status(Some(tr(L::NoReplaceableResults)));
        }
    }

    pub(super) fn search_options_menu(&mut self) {
        let label = |key, checked| {
            if checked {
                format!("✓  {}", tr(key))
            } else {
                tr(key).to_owned()
            }
        };
        let case = label(L::MatchCase, self.search_options.case_sensitive);
        let word = label(L::WholeWord, self.search_options.whole_word);
        self.popup(
            self.find_actions[2],
            &[(MATCH_CASE, &case), (WHOLE_WORD, &word)],
            None,
        );
    }

    pub(super) fn update_find_hint(&self) {
        let hint = wide(tr(if self.find_headings {
            L::FindHeading
        } else {
            L::FindText
        }));
        send(self.find_box, EM_SETCUEBANNER, 1, hint.as_ptr() as isize);
        let replacement = wide(tr(L::ReplacementText));
        send(
            self.replace_box,
            EM_SETCUEBANNER,
            1,
            replacement.as_ptr() as isize,
        );
    }

    pub(super) fn find(&self, backwards: bool) {
        let query = get_text(self.find_box);
        if self.find_headings {
            let query = query.to_lowercase();
            let current = self.current_position();
            // Source editing leaves the rendered snapshot alone while typing.
            let current_markdown = self
                .active
                .filter(|_| self.mode() == 3)
                .map(|index| Markdown::parse(&self.docs[index].text));
            let mut matching = current_markdown
                .as_ref()
                .unwrap_or(&self.parsed)
                .regions
                .iter()
                .filter(|region| region.level > 0 && region.text.to_lowercase().contains(&query));
            let found = if backwards {
                matching
                    .clone()
                    .rfind(|region| region.source.start < current)
                    .or_else(|| matching.next_back())
            } else {
                matching
                    .clone()
                    .find(|region| region.source.start > current)
                    .or_else(|| matching.next())
            };
            if let Some(region) = found {
                self.jump_source(region.source.start);
                self.status(None);
            } else {
                self.status(Some(tr(L::NoHeadings)));
            }
            return;
        }
        if query.is_empty() {
            return;
        }
        let text = String::from_utf16_lossy(&self.view.text);
        let ranges = pane_md::search::matches(&text, &query, self.search_options);
        let mut chars = text.char_indices().peekable();
        let mut offset = 0;
        let mut unit_at = |point| {
            while chars.peek().is_some_and(|&(byte, _)| byte < point) {
                if let Some((_, ch)) = chars.next() {
                    offset += ch.len_utf16();
                }
            }
            offset
        };
        let matches = ranges
            .into_iter()
            .map(|span| unit_at(span.start)..unit_at(span.end))
            .collect::<Vec<_>>();
        let current = selection(self.editor);
        let found = if backwards {
            matches
                .iter()
                .rposition(|span| span.end <= current.start)
                .or_else(|| matches.len().checked_sub(1))
        } else {
            matches
                .iter()
                .position(|span| span.start >= current.end)
                .or_else(|| (!matches.is_empty()).then_some(0))
        };
        if let Some(index) = found {
            select(self.editor, matches[index].clone(), true);
            let notice = tr(L::SearchCount)
                .replace("{current}", &(index + 1).to_string())
                .replace("{total}", &matches.len().to_string());
            self.status(Some(&notice));
        } else {
            self.status(Some(tr(L::NoResults)));
        }
    }

    pub(super) fn lint_jump(&self, backwards: bool) {
        let Some(index) = self.active else { return };
        let doc = &self.docs[index];
        let source = &doc.text;
        let issues = match doc.path() {
            Some(path) => pane_md::lint::check_parsed_at(source, &self.parsed, path),
            None => pane_md::lint::check_parsed(source, &self.parsed),
        };
        if issues.is_empty() {
            self.status(Some(tr(L::LintClean)));
            return;
        }
        let lines = pane_md::markdown::Lines::new(source);
        let current = selection(self.editor).start;
        let locations = || {
            issues.iter().enumerate().filter_map(|(n, issue)| {
                let source = lines.offset(issue.line, issue.column)?;
                let visible = self.view.visible_at(&self.parsed, source);
                Some((n, visible))
            })
        };
        let found = if backwards {
            locations()
                .rfind(|&(_, at)| at < current)
                .or_else(|| locations().next_back())
        } else {
            locations()
                .find(|&(_, at)| at > current)
                .or_else(|| locations().next())
        };
        if let Some((n, at)) = found {
            let issue = &issues[n];
            let label = crate::i18n::lint_label(issue.code);
            select(self.editor, at..at, true);
            self.status(Some(&format!(
                "{}/{}   {}:{}   {label}",
                n + 1,
                issues.len(),
                issue.line,
                issue.column
            )));
        }
    }

    pub(super) fn link_at(&self, at: usize) -> Option<(&str, bool)> {
        if !self.projection_safe {
            return None;
        }
        if self.mode() % 2 == 1 {
            return (!self.source_parse_dirty)
                .then(|| self.view.source_at(&self.parsed, at))
                .and_then(|byte| self.parsed.source_link(byte))
                .map(|link| (link.target.as_str(), link.image));
        }
        let raw = self.view.link(at)?;
        Some(
            raw.strip_prefix("image:")
                .map_or((raw, false), |url| (url, true)),
        )
    }

    pub(super) fn follow_at(&mut self, at: usize) {
        if self.draft.is_some() {
            return;
        }
        self.flush_edit();
        self.parse_source_edits();
        let Some((raw, image)) = self.link_at(at) else {
            return;
        };
        let raw = raw.to_owned();
        if image
            && let Ok(pane_md::links::Destination::External(url)) =
                pane_md::links::destination(&raw)
            && (url.to_ascii_lowercase().starts_with("https://")
                || url.to_ascii_lowercase().starts_with("http://"))
            && self.remote_images.picture(&raw).is_none()
        {
            if self.remote_images.request(&raw, &url, self.hwnd) {
                self.status(Some(tr(L::LoadingRemoteImage)));
            }
            return;
        }
        if let Err(error) = self.follow(&raw) {
            self.status(Some(&error.to_string()));
        } else if image {
            self.status(None);
        }
    }

    fn follow(&mut self, raw: &str) -> Result<()> {
        use pane_md::links::{Destination, TargetError, destination};

        let target =
            destination(raw).map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                match error {
                    TargetError::Decode(error) => link_decode_error(error),
                    TargetError::Invalid => tr(L::InvalidLink).into(),
                    TargetError::Unsupported => tr(L::LinkUnavailable).into(),
                }
            })?;
        if let Destination::External(url) = target {
            return self.shell_open(&url);
        }
        let index = self.active.ok_or(tr(L::OpenFirst))?;
        let document = &self.docs[index];
        let from = self.link_position(document);
        let (anchor, opened) = match target {
            Destination::Local { path, anchor } => {
                let path = document.resolve_link(&path)?;
                let metadata = fs::metadata(&path)?;
                if !metadata.is_file() && !metadata.is_dir() {
                    return Err(tr(L::LinkUnavailable).into());
                }
                if !metadata.is_dir() && !markdown_path(&path) {
                    return self.open_attachment(&path);
                }
                if self.wait {
                    if Some(path.as_path()) != from.path.as_deref() {
                        return Err(tr(L::WaitLink).into());
                    }
                } else {
                    self.open(path, self.mode() % 2)?;
                }
                (anchor, self.active != Some(index))
            }
            Destination::Anchor(anchor) => (anchor, false),
            Destination::External(_) => unreachable!(),
        };
        let moved = if anchor.is_empty() {
            if raw.contains('#') {
                self.jump_source(0);
                true
            } else {
                false
            }
        } else {
            self.jump_anchor(&anchor)
        };
        if opened || moved {
            self.history.visit(from);
        }
        if !anchor.is_empty() && !moved {
            return Err(format!("{}: #{anchor}", tr(L::AnchorNotFound)).into());
        }
        self.status(None);
        Ok(())
    }

    fn open_attachment(&self, path: &Path) -> Result<()> {
        let supported = [
            "txt", "log", "pdf", "csv", "tsv", "htm", "html", "png", "jpg", "jpeg", "gif", "bmp",
            "ico", "tif", "tiff", "webp", "svg", "avif", "mp3", "mp4", "wav", "webm", "ogg",
            "flac", "docx", "xlsx", "pptx", "odt", "ods", "odp",
        ];
        if !path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| supported.iter().any(|kind| ext.eq_ignore_ascii_case(kind)))
        {
            return Err(tr(L::AttachmentUnavailable).into());
        }
        let target = path.to_string_lossy();
        if let Some(share) = target.strip_prefix(r"\\?\UNC\") {
            self.shell_open(&format!(r"\\{share}"))
        } else {
            self.shell_open(target.strip_prefix(r"\\?\").unwrap_or(&target))
        }
    }

    fn shell_open(&self, target: &str) -> Result<()> {
        let code = unsafe {
            ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(wide(target).as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if code.0 as isize <= 32 {
            return Err(tr(L::LinkOpenError).into());
        }
        Ok(())
    }

    pub(super) fn jump_anchor(&self, fragment: &str) -> bool {
        let Some(byte) = self.parsed.anchor_byte(fragment) else {
            return false;
        };
        if self.mode().is_multiple_of(2)
            && let Some(name) = fragment.strip_prefix("fnref-")
        {
            let note = format!("#fn-{name}");
            let visible = self.view.visible_at(&self.parsed, byte);
            if let Some((span, _)) = self
                .view
                .links
                .iter()
                .filter(|(_, url)| *url == note)
                .min_by_key(|(span, _)| span.start.abs_diff(visible))
            {
                select(self.editor, span.clone(), true);
                return true;
            }
        }
        self.jump_source(byte);
        true
    }

    fn link_position(&self, document: &Document) -> LinkPosition<(Viewport, i32)> {
        document.link_position((self.capture_viewport(), scroll_pos(self.editor).x))
    }

    pub(super) fn walk_link_history(&mut self, forward: bool) {
        if !self.history.available(forward) {
            return;
        }
        self.parse_source_edits();
        let from = self
            .active
            .map(|index| self.link_position(&self.docs[index]));
        let mut failure = tr(L::LinkLocationUnavailable).to_owned();
        while let Some(destination) = self.history.pop(forward) {
            if let Some(index) = destination.open_tab(&self.docs) {
                self.switch(index);
            } else if let Some(path) = destination.path.filter(|path| path.is_file()) {
                if let Err(error) = self.open(path, destination.mode) {
                    failure = error.to_string();
                    continue;
                }
            } else {
                continue;
            }
            let Some(index) = self.active else { continue };
            let family = destination.mode & 1;
            if self.mode() & 1 != family {
                self.change_mode(self.mode() & !1 | family, self.show_review_marks());
            }
            let (viewport, left) = destination.viewport;
            self.docs[index].horizontal_scroll[family] = left;
            unsafe {
                let _ = SetFocus(Some(self.editor));
            }
            self.restore_viewport(&viewport);
            if let Some(from) = from {
                self.history.leave(forward, from);
            }
            self.status(None);
            return;
        }
        self.status(Some(&failure));
    }

    pub(super) fn context_menu(&mut self, source: usize, coordinates: isize) {
        if source != self.editor.0 as usize {
            return;
        }
        let location = if coordinates == -1 {
            None
        } else {
            Some(POINT {
                x: coordinates as i16 as i32,
                y: (coordinates >> 16) as i16 as i32,
            })
        };
        if selection(self.editor).is_empty() {
            return;
        }
        self.popup(
            self.editor,
            &[
                (COPY_SELECTION, tr(L::Copy)),
                (ADD_REVIEW, tr(L::Comment)),
                (CUT_SELECTION, tr(L::Cut)),
            ],
            location,
        );
    }

    pub(super) fn cut_selection(&mut self) {
        let span = selection(self.editor);
        if self.mode() < 2 || !self.projection_safe || span.is_empty() {
            return;
        }
        if self.mode() == 3 {
            send(self.editor, WM_CUT, 0, 0);
            self.sync();
            return;
        }
        let Some(index) = self.active else { return };
        let source = &self.docs[index].text;
        let edit = self
            .view
            .region_range(span.clone())
            .and_then(|(i, local)| {
                let region = &self.parsed.regions[i];
                if region.removes_inline_run(&local) {
                    return None;
                }
                region.patch(source, local, "")
            })
            .map(|(range, text)| {
                let caret = range.start;
                pane_md::markdown::FormatPatch::new(range, text, caret)
            })
            .or_else(|| self.visual_replacement(span, ""));
        if let Some(edit) = edit {
            send(self.editor, WM_CUT, 0, 0);
            if buffer(self.editor) == self.view.text {
                return;
            }
            self.patch(edit.range, &edit.text, edit.caret.start);
        } else {
            self.status(Some(tr(L::EditSourceHint)));
        }
    }

    pub(super) fn popup(&mut self, anchor: HWND, items: &[(usize, &str)], position: Option<POINT>) {
        if let Some(command) = self.popup_choice(anchor, items, position) {
            self.command(command);
        }
    }

    fn popup_choice(
        &self,
        anchor: HWND,
        items: &[(usize, &str)],
        position: Option<POINT>,
    ) -> Option<usize> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            for (id, text) in items {
                let flags = if *id == CUT_SELECTION && (self.mode() < 2 || self.draft.is_some())
                    || *id == ADD_REVIEW && (self.draft.is_some() || !self.projection_safe)
                {
                    MF_GRAYED
                } else if *id == self.display_state() {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                };
                let _ = AppendMenuW(menu, MF_STRING | flags, *id, PCWSTR(wide(text).as_ptr()));
            }
            let point = position.unwrap_or_else(|| {
                let mut rect = RECT::default();
                let _ = GetWindowRect(anchor, &mut rect);
                POINT {
                    x: rect.left + 10,
                    y: if anchor == self.hwnd {
                        rect.top + self.dip(88)
                    } else {
                        rect.bottom
                    },
                }
            });
            let result = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                Some(0),
                self.hwnd,
                None,
            )
            .0;
            let _ = DestroyMenu(menu);
            (result > 0).then_some(result as usize)
        }
    }
}

pub(in crate::desktop) fn decode_url(value: &str) -> Result<String> {
    pane_md::links::decode_percent(value).map_err(link_decode_error)
}

fn link_decode_error(
    error: pane_md::links::DecodeError,
) -> Box<dyn std::error::Error + Send + Sync> {
    use pane_md::links::DecodeError;
    match error {
        DecodeError::Escape => tr(L::InvalidLinkEscape).into(),
        DecodeError::Control => tr(L::InvalidLink).into(),
        DecodeError::Utf8(error) => error.into(),
    }
}

const MAIN_MENUS: [L; 5] = [
    L::MenuFile,
    L::MenuEdit,
    L::MenuView,
    L::MenuFormat,
    L::MenuSettings,
];

pub(super) unsafe fn menu_message(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> Option<LRESULT> {
    unsafe {
        match message {
            WM_MEASUREITEM if w.0 == 0 && l.0 != 0 => {
                let measure = &mut *(l.0 as *mut MEASUREITEMSTRUCT);
                if measure.CtlType != ODT_MENU {
                    return None;
                }
                let label = tr(*MAIN_MENUS.get(measure.itemData)?).replace('&', "");
                let label = label.encode_utf16().collect::<Vec<_>>();
                let dc = GetDC(Some(hwnd));
                let old = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &label, &mut size);
                SelectObject(dc, old);
                let _ = ReleaseDC(Some(hwnd), dc);
                let dpi = GetDpiForWindow(hwnd).max(96);
                measure.itemWidth = size.cx as u32 + 16 * dpi / 96;
                measure.itemHeight = size.cy as u32 + 6 * dpi / 96;
                Some(LRESULT(1))
            }
            WM_DRAWITEM if w.0 == 0 && l.0 != 0 => {
                let item = &*(l.0 as *const DRAWITEMSTRUCT);
                if item.CtlType != ODT_MENU {
                    return None;
                }
                let label = tr(*MAIN_MENUS.get(item.itemData)?);
                let palette = Palette::new(false);
                let hot = item.itemState.0 & (ODS_SELECTED.0 | ODS_HOTLIGHT.0) != 0;
                super::atoms::fill(
                    item.hDC,
                    &item.rcItem,
                    if hot {
                        palette.caption_hover
                    } else {
                        palette.caption
                    },
                );
                let old = SelectObject(item.hDC, GetStockObject(DEFAULT_GUI_FONT));
                SetTextColor(item.hDC, Palette::rgb(palette.on_caption));
                SetBkMode(item.hDC, TRANSPARENT);
                let mut rect = item.rcItem;
                let flags = DT_CENTER
                    | DT_VCENTER
                    | DT_SINGLELINE
                    | if item.itemState.0 & ODS_NOACCEL.0 != 0 {
                        DT_HIDEPREFIX
                    } else {
                        DRAW_TEXT_FORMAT(0)
                    };
                DrawTextW(item.hDC, &mut wide(label), &mut rect, flags);
                SelectObject(item.hDC, old);
                Some(LRESULT(1))
            }
            WM_MENUCHAR if GetMenu(hwnd).0 as isize == l.0 => {
                let key = char::from_u32((w.0 & 0xffff) as u32)?
                    .to_lowercase()
                    .next()?;
                let index = MAIN_MENUS.iter().position(|&label| {
                    tr(label)
                        .split_once('&')
                        .and_then(|(_, tail)| tail.chars().next())
                        .and_then(|ch| ch.to_lowercase().next())
                        == Some(key)
                })?;
                Some(LRESULT(((MNC_EXECUTE as usize) << 16 | index) as isize))
            }
            _ => None,
        }
    }
}

pub(super) fn menu_bar(theme: usize) -> Result<HMENU> {
    let sections: &[(&str, &[(usize, &str)])] = &[
        (
            tr(L::MenuFile),
            &[
                (NEW_NOTE, tr(L::NewNote)),
                (OPEN, tr(L::OpenFileMenu)),
                (FOLDER, tr(L::OpenDirectoryMenu)),
                (SAVE, tr(L::SaveMenu)),
                (SAVE_AS, tr(L::SaveAsMenu)),
                (CLOSE, tr(L::CloseTab)),
            ],
        ),
        (
            tr(L::MenuEdit),
            &[
                (UNDO, tr(L::UndoMenu)),
                (REDO, tr(L::RedoMenu)),
                (FIND, tr(L::FindMenu)),
                (SHOW_REPLACE, tr(L::ReplaceMenu)),
                (FIND_NEXT, tr(L::FindNext)),
                (FIND_PREV, tr(L::FindPrevious)),
                (LINT_NEXT, tr(L::LintNext)),
                (LINT_PREV, tr(L::LintPrevious)),
                (0, ""),
                (ADD_REVIEW, tr(L::AddReviewMenu)),
                (EDIT_REVIEW, tr(L::EditReviewMenu)),
                (RESOLVE_REVIEW, tr(L::ToggleResolved)),
                (REATTACH, tr(L::Reattach)),
                (205, tr(L::SavedHeading)),
                (DELETE_REVIEW, tr(L::DeleteReviewMenu)),
            ],
        ),
        (
            tr(L::MenuView),
            &[
                (TREE, tr(L::ShowFiles)),
                (REFRESH_FILES, tr(L::RefreshFiles)),
                (REVIEWS, tr(L::ReviewPane)),
                (HEADINGS, tr(L::FindHeadingMenu)),
                (OUTLINE, tr(L::Outline)),
                (REVIEW_ALL, tr(L::AllComments)),
                (FAMILY, tr(L::SwitchFamily)),
                (READ, tr(L::View)),
                (COMMENTS, tr(L::Review)),
                (EDITING, tr(L::Edit)),
                (SHOW_SOURCE, tr(L::ShowSource)),
                (BACK, tr(L::BackLink)),
                (FORWARD_LINK, tr(L::ForwardLink)),
                (ZOOM_IN, tr(L::ZoomIn)),
                (ZOOM_OUT, tr(L::ZoomOut)),
                (ZOOM_RESET, tr(L::ZoomReset)),
            ],
        ),
        (
            tr(L::MenuFormat),
            &[
                (BOLD, tr(L::Bold)),
                (ITALIC, tr(L::Italic)),
                (STRIKE, tr(L::Strike)),
                (CODE, tr(L::Code)),
                (LINK, tr(L::Link)),
                (INSERT_IMAGE, tr(L::InsertImage)),
                (HEADING_FIRST, tr(L::Heading1)),
                (HEADING_FIRST + 1, tr(L::Heading2)),
                (HEADING_FIRST + 2, tr(L::Heading3)),
                (HEADING_FIRST + 3, tr(L::Heading4)),
                (HEADING_FIRST + 4, tr(L::Heading5)),
                (HEADING_LAST, tr(L::Heading6)),
                (0, ""),
                (BULLET, tr(L::Bullet)),
                (NUMBERED_LIST, tr(L::NumberedItem)),
                (MAKE_TASK, tr(L::MakeTask)),
                (TOGGLE_TASK, tr(L::ToggleTask)),
                (0, ""),
                (ROW_ADD, tr(L::InsertRow)),
                (ROW_DELETE, tr(L::DeleteRow)),
                (COL_ADD, tr(L::InsertColumn)),
                (COL_DELETE, tr(L::DeleteColumn)),
            ],
        ),
        (
            tr(L::MenuSettings),
            &[
                (THEME_LIGHT, tr(L::LightTheme)),
                (THEME_DARK, tr(L::DarkTheme)),
                (THEME, tr(L::SystemTheme)),
                (APPEARANCE, tr(L::Appearance)),
                (0, ""),
                (LANG_RU, tr(L::Russian)),
                (LANG_EN, tr(L::English)),
            ],
        ),
    ];
    unsafe {
        let menu = CreateMenu()?;
        for (index, (title, items)) in sections.iter().enumerate() {
            let popup = CreatePopupMenu()?;
            for (id, name) in *items {
                if *id == APPEARANCE {
                    let appearance = CreatePopupMenu()?;
                    for (command, label) in super::appearance::COMMANDS {
                        AppendMenuW(
                            appearance,
                            MF_STRING,
                            command,
                            PCWSTR(wide(tr(label)).as_ptr()),
                        )?;
                    }
                    AppendMenuW(
                        popup,
                        MF_POPUP,
                        appearance.0 as usize,
                        PCWSTR(wide(name).as_ptr()),
                    )?;
                    continue;
                }
                if *id == 0 {
                    AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null())?;
                    continue;
                }
                let selected = *id == theme
                    || *id == LANG_RU && !i18n::english()
                    || *id == LANG_EN && i18n::english();
                let flags = if selected {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                };
                AppendMenuW(popup, flags, *id, PCWSTR(wide(name).as_ptr()))?;
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                popup.0 as usize,
                PCWSTR(wide(title).as_ptr()),
            )?;
            SetMenuItemInfoW(
                menu,
                index as u32,
                true,
                &MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE | MIIM_DATA,
                    fType: MFT_OWNERDRAW,
                    dwItemData: index,
                    ..Default::default()
                },
            )?;
        }
        Ok(menu)
    }
}
