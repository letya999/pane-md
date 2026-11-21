use super::*;

impl App {
    pub(super) fn open(&mut self, path: PathBuf, mode: usize) -> Result<()> {
        let path = fs::canonicalize(path)?;
        if path.is_dir() {
            if self.wait {
                return Err("в режиме ожидания нужен файл".into());
            }
            self.folder(path);
            return Ok(());
        }
        if let Some(index) = self.docs.iter().position(|doc| doc.path == path) {
            self.switch(index);
            return Ok(());
        }
        if self.wait && self.active.is_some() {
            return Ok(());
        }
        let doc = Document::open(path, mode)?;
        self.store_position();
        let index = self.docs.len();
        self.docs.push(doc);
        let mut name = wide(&self.tab_name(index));
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(name.as_mut_ptr()),
            ..Default::default()
        };
        send(self.tab, TCM_INSERTITEMW, index, &item as *const _ as isize);
        self.active = Some(index);
        self.reset_review();
        self.refresh(0);
        Ok(())
    }

    fn folder(&mut self, path: PathBuf) {
        send(self.tree, TVM_DELETEITEM, 0, TVI_ROOT.0 as isize);
        self.tree_entries.clear();
        self.tree_open = true;
        self.workspace = Some(path.clone());
        let root = self.tree_insert(TVI_ROOT, path, true);
        self.expand(root, 0);
        send(
            self.tree,
            TVM_EXPAND,
            TVE_EXPAND.0 as usize,
            root.0 as isize,
        );
        self.layout();
    }

    fn tree_insert(&mut self, parent: HTREEITEM, path: PathBuf, folder: bool) -> HTREEITEM {
        let mut name = wide(
            &path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
        );
        let index = self.tree_entries.len();
        self.tree_entries.push(TreeEntry {
            path,
            folder,
            loaded: false,
        });
        let item = TVINSERTSTRUCTW {
            hParent: parent,
            hInsertAfter: TVI_LAST,
            Anonymous: TVINSERTSTRUCTW_0 {
                item: TVITEMW {
                    mask: TVIF_TEXT | TVIF_PARAM | TVIF_CHILDREN,
                    pszText: PWSTR(name.as_mut_ptr()),
                    lParam: LPARAM(index as isize),
                    cChildren: TVITEMEXW_CHILDREN(i32::from(folder)),
                    ..Default::default()
                },
            },
        };
        HTREEITEM(send(
            self.tree,
            TVM_INSERTITEMW,
            0,
            &item as *const _ as isize,
        ))
    }

    pub(super) fn expand(&mut self, item: HTREEITEM, index: usize) {
        let Some(entry) = self.tree_entries.get_mut(index) else {
            return;
        };
        if !entry.folder || entry.loaded {
            return;
        }
        let listing = match fs::read_dir(&entry.path) {
            Ok(listing) => listing,
            Err(error) => {
                self.error("Не удалось прочитать папку", &error);
                return;
            }
        };
        entry.loaded = true;
        let mut entries = listing
            .filter_map(std::result::Result::ok)
            .filter_map(|child| {
                let kind = child.file_type().ok()?;
                let path = child.path();
                (kind.is_dir() || (kind.is_file() && markdown_path(&path)))
                    .then_some((kind.is_dir(), path))
            })
            .collect::<Vec<_>>();
        entries.sort_by_cached_key(|(folder, path)| {
            (
                !folder,
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase(),
            )
        });
        if entries.is_empty() {
            let info = TVITEMW {
                mask: TVIF_CHILDREN,
                hItem: item,
                cChildren: TVITEMEXW_CHILDREN(0),
                ..Default::default()
            };
            send(self.tree, TVM_SETITEMW, 0, &info as *const _ as isize);
        }
        for (folder, path) in entries {
            self.tree_insert(item, path, folder);
        }
    }

    pub(super) fn drop_files(&mut self, drop: HDROP) {
        unsafe {
            for index in 0..DragQueryFileW(drop, u32::MAX, None) {
                let len = DragQueryFileW(drop, index, None);
                let mut buffer = vec![0; len as usize + 1];
                DragQueryFileW(drop, index, Some(&mut buffer));
                let path = PathBuf::from(OsString::from_wide(&buffer[..len as usize]));
                if let Err(error) = self.open(path, 0) {
                    self.error("Не удалось открыть документ", &error);
                }
            }
            DragFinish(drop);
        }
    }

    fn tab_name(&self, index: usize) -> String {
        let doc = &self.docs[index];
        format!(
            "{}{}",
            doc.path.file_name().unwrap_or_default().to_string_lossy(),
            if doc.dirty() { " *" } else { "" }
        )
    }

    fn title(&self) {
        if let Some(index) = self.active {
            let name = self.tab_name(index);
            let mut title = wide(&name);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(title.as_mut_ptr()),
                ..Default::default()
            };
            send(self.tab, TCM_SETITEMW, index, &item as *const _ as isize);
            set_text(self.hwnd, &format!("{name} — notepad-md"));
        } else {
            set_text(self.hwnd, "notepad-md");
        }
    }

    pub(super) fn store_position(&mut self) {
        self.sync();
        if let Some(i) = self.active {
            self.docs[i].viewport = self.capture_viewport();
            let family = self.mode() % 2;
            self.docs[i].horizontal_scroll[family] = scroll_pos(self.editor).x;
        }
    }

    pub(super) fn current_position(&self) -> usize {
        self.view
            .source_at(&self.parsed, selection(self.editor).start)
    }

    pub(super) fn capture_viewport(&self) -> Viewport {
        let rect = text_rect(self.editor);
        let p = POINT {
            x: rect.left + 1,
            y: rect.top + 1,
        };
        let at = hit(self.editor, p).unwrap_or(0).min(self.view.text.len());
        Viewport {
            selection: self
                .view
                .source_selection(&self.parsed, selection(self.editor)),
            top: self.view.source_at(&self.parsed, at),
            line_y: char_point(self.editor, at).y - rect.top,
        }
    }

    pub(super) fn restore_viewport(&self, viewport: &Viewport) {
        let start = self.view.visible_at(&self.parsed, viewport.selection.start);
        let end = self.view.visible_at(&self.parsed, viewport.selection.end);
        select(self.editor, start..end.max(start), false);
        let top = self.view.visible_at(&self.parsed, viewport.top);
        // EM_GETSCROLLPOS can truncate the vertical coordinate on long documents.
        set_scroll(self.editor, POINT::default());
        let y = char_point(self.editor, top).y;
        let rect = text_rect(self.editor);
        let x = self
            .active
            .map_or(0, |i| self.docs[i].horizontal_scroll[self.mode() % 2]);
        set_scroll(
            self.editor,
            POINT {
                x,
                y: (y - rect.top - viewport.line_y).max(0),
            },
        );
        self.place_review_card();
    }

    pub(super) fn refresh_viewport(&mut self) {
        let viewport = self
            .active
            .map_or_else(Viewport::default, |i| self.docs[i].viewport.clone());
        self.refresh(0);
        self.restore_viewport(&viewport);
    }

    pub(super) fn switch(&mut self, index: usize) {
        if index >= self.docs.len() || self.active == Some(index) {
            return;
        }
        self.store_position();
        self.active = Some(index);
        self.reset_review();
        self.refresh_viewport();
    }

    fn reset_review(&mut self) {
        self.active_review = None;
        self.draft = None;
    }

    pub(super) fn refresh(&mut self, position: usize) {
        if let Some(i) = self.active {
            self.parsed = Markdown::parse(&self.docs[i].text);
            send(self.tab, TCM_SETCURSEL, i, 0);
        } else {
            self.parsed = Markdown::default();
        }
        self.rebuild_editor(position);
        self.refresh_reviews();
        self.title();
        self.layout();
        for (button, _) in &self.toolbar {
            let id = unsafe { GetDlgCtrlID(*button) } as usize;
            if (MODES..MODES + 4).contains(&id) {
                send(
                    *button,
                    BM_SETCHECK,
                    usize::from(self.mode() == id - MODES),
                    0,
                );
            }
            let enabled = match id {
                OPEN | TREE => true,
                FORMAT | TABLE_TOOLS => self.active.is_some() && self.mode() == 2,
                _ => self.active.is_some(),
            };
            unsafe {
                let _ = EnableWindow(*button, enabled);
            }
        }
        self.status(None);
    }

    fn render_data(&mut self) -> (String, Projection) {
        let Some(i) = self.active else {
            return (self.palette.rtf() + "}", Projection::default());
        };
        if self.mode() % 2 == 1 {
            let view = Projection::source(&self.docs[i].text);
            (source_rtf(&view.text, self.palette), view)
        } else {
            render(
                &self.parsed,
                self.palette,
                10_800,
                self.docs[i].path.parent().unwrap_or(Path::new(".")),
                &mut self.diagrams,
                self.dark,
            )
        }
    }

    fn rebuild_editor(&mut self, position: usize) {
        let (rtf, view) = self.render_data();
        self.view = view;
        send(self.editor, WM_SETREDRAW, 0, 0);
        send(self.editor, EM_SETREADONLY, 0, 0);
        let loaded = stream(self.editor, &rtf);
        let text = buffer(self.editor);
        if self.view.text.last() == Some(&13)
            && text == self.view.text[..self.view.text.len() - 1]
            && self.mode() % 2 == 0
        {
            self.view.text.pop();
        }
        self.projection_safe = loaded && text == self.view.text;
        if !self.projection_safe && self.active.is_some() {
            eprintln!(
                "notepad-md: RichEdit projection differs ({} native, {} expected)",
                text.len(),
                self.view.text.len()
            );
        }
        send(
            self.editor,
            EM_SETREADONLY,
            usize::from(self.mode() < 2 || !self.projection_safe || self.draft.is_some()),
            0,
        );
        self.apply_zoom();
        set_scroll(self.editor, POINT::default());
        self.jump_source(position);
        send(self.editor, WM_SETREDRAW, 1, 0);
        unsafe {
            let _ = InvalidateRect(Some(self.editor), None, true);
        }
    }

    pub(super) fn jump_source(&self, byte: usize) {
        let pos = self.view.visible_at(&self.parsed, byte);
        select(self.editor, pos..pos, true);
    }

    pub(super) fn status(&self, override_text: Option<&str>) {
        let info = self.active.map(|index| {
            let doc = &self.docs[index];
            let warnings = self.parsed.diagnostics.len();
            let path = doc.path.to_string_lossy();
            let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
            let mut text = format!("{}    {path}", MODES_TEXT[doc.mode]);
            if self.view.diagram_fallbacks != 0 {
                text.push_str(&format!(
                    "    Mermaid исходником: {}",
                    self.view.diagram_fallbacks
                ));
            }
            if warnings > 0 {
                format!("{text}    Повреждённых записей ревью: {warnings}")
            } else {
                text
            }
        });
        let default = if self.active.is_some() && !self.projection_safe && self.mode() >= 2 {
            "Редактирование недоступно: RichEdit изменил служебные позиции."
        } else {
            info.as_deref().unwrap_or("Готово")
        };
        set_text(self.status, override_text.unwrap_or(default));
    }

    pub(super) fn sync(&mut self) {
        let Some(i) = self.active else { return };
        if self.mode() < 2 || !self.projection_safe {
            return;
        }
        let after = buffer(self.editor);
        if self.view.text == after {
            return;
        }
        let selected = selection(self.editor);
        let scroll = scroll_pos(self.editor);
        let (mut old, mut new) = diff(&self.view.text, &after);
        let previous_position = self.view.source_at(&self.parsed, old.start);
        let source = &self.docs[i].text;
        let patch = if self.mode() == 3 {
            edit_source(source, &self.view.text, &self.view.source, &after)
        } else {
            if self
                .view
                .text
                .get(old.start)
                .is_some_and(|u| (0xdc00..=0xdfff).contains(u))
            {
                old.start = old.start.saturating_sub(1);
                new.start = new.start.saturating_sub(1);
            }
            if self
                .view
                .text
                .get(old.end)
                .is_some_and(|u| (0xdc00..=0xdfff).contains(u))
            {
                old.end += 1;
                new.end += 1;
            }
            let inserted = after
                .get(new)
                .and_then(|units| String::from_utf16(units).ok());
            self.view.region_range(old.clone()).zip(inserted).and_then(
                |((region, selection), inserted)| {
                    if self.view.diagram_regions.contains(&region)
                        || inserted
                            .chars()
                            .any(|ch| matches!(ch, '\u{fff9}'..='\u{fffc}' | '\u{0007}' | '\0'))
                    {
                        return None;
                    }
                    self.parsed.regions[region].patch(source, selection, &inserted)
                },
            )
        };
        if let Some((range, text)) = patch {
            let position = range.start + text.len();
            let live_edit = self.mode() == 2
                && apply_plain_edit(&mut self.parsed, &mut self.view, source, old, &range, &text);
            self.docs[i].patch(range, &text, true);
            if live_edit {
                self.view.text = after;
            } else if self.mode() == 3 {
                self.view = Projection::source(&self.docs[i].text);
                if self.view.text != after {
                    self.rebuild_editor(position);
                }
            } else {
                self.parsed = Markdown::parse(&self.docs[i].text);
                let (_, mut view) = self.render_data();
                if after == view.text
                    || fit_trimmed_spaces(
                        &mut self.parsed,
                        &mut view,
                        &after,
                        &self.docs[i].text,
                        position,
                    )
                    || (text.ends_with(['\r', '\n'])
                        && fit_new_paragraph(&mut self.parsed, &mut view, &after, position))
                {
                    self.view = view;
                } else {
                    self.rebuild_editor(position);
                    set_scroll(self.editor, scroll);
                    let pos = selected.start.min(self.view.text.len());
                    select(self.editor, pos..pos, false);
                }
                if !self.parsed.records.is_empty() {
                    self.refresh_reviews();
                }
            }
            self.title();
            self.status(None);
        } else {
            self.rebuild_editor(previous_position);
            set_scroll(self.editor, scroll);
            self.status(Some("Этот диапазон можно изменить в MD Source Edit."));
        }
    }

    pub(super) fn patch(&mut self, range: Range<usize>, replacement: &str, position: usize) {
        if let Some(i) = self.active {
            let scroll = scroll_pos(self.editor);
            self.docs[i].patch(range, replacement, false);
            self.refresh(position);
            set_scroll(self.editor, scroll);
        }
    }

    fn save(&mut self, index: usize) -> bool {
        match self.docs[index].save() {
            Ok(()) => {
                self.title();
                self.status(Some("Сохранено"));
                true
            }
            Err(error) => {
                self.error("Сохранение не выполнено", &error);
                false
            }
        }
    }

    fn save_as(&mut self) {
        let Some(i) = self.active else { return };
        let name = self.docs[i]
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let Some(path) = self.choose(false, true, name.as_deref()) else {
            return;
        };
        match self.docs[i].save_as(path) {
            Ok(()) => {
                self.title();
                self.status(Some("Сохранено"));
            }
            Err(error) => self.error("Не удалось сохранить под новым именем", &error),
        }
    }

    pub(super) fn confirm_close(&mut self, index: usize) -> bool {
        if !self.docs[index].dirty() {
            return true;
        }
        let text = format!(
            "Сохранить изменения в «{}»?",
            self.tab_name(index).trim_end_matches(" *")
        );
        let result = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(wide(&text).as_ptr()),
                w!("notepad-md"),
                MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON1,
            )
        };
        match result {
            IDYES => self.save(index),
            IDNO => true,
            _ => false,
        }
    }

    fn close_tab(&mut self) {
        if self.wait {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return;
        }
        let Some(i) = self.active else { return };
        if !self.confirm_close(i) {
            return;
        }
        self.docs.remove(i);
        send(self.tab, TCM_DELETEITEM, i, 0);
        self.active = (!self.docs.is_empty()).then_some(i.min(self.docs.len().saturating_sub(1)));
        self.reset_review();
        self.refresh_viewport();
    }

    pub(super) fn error(&self, title: &str, error: &dyn std::fmt::Display) {
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(wide(&error.to_string()).as_ptr()),
                PCWSTR(wide(title).as_ptr()),
                MB_OK | MB_ICONERROR,
            );
        }
    }

    fn choose(&self, directory: bool, save: bool, name: Option<&str>) -> Option<PathBuf> {
        let result = (|| unsafe {
            let dialog: IFileDialog = if save {
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?
            } else {
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?
            };
            let mut options = FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR;
            if directory {
                options |= FOS_PICKFOLDERS;
            } else if !save {
                options |= FOS_FILEMUSTEXIST;
            }
            if save {
                options |= FOS_OVERWRITEPROMPT;
            }
            dialog.SetOptions(options)?;
            if let Some(name) = name {
                dialog.SetFileName(PCWSTR(wide(name).as_ptr()))?;
            }
            dialog.SetTitle(if directory {
                w!("Открыть папку")
            } else if save {
                w!("Сохранить под новым именем")
            } else {
                w!("Открыть документ")
            })?;
            dialog.Show(Some(self.hwnd))?;
            let item = dialog.GetResult()?;
            let raw = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = PathBuf::from(OsString::from_wide(raw.as_wide()));
            CoTaskMemFree(Some(raw.0 as *const _));
            Ok::<_, windows::core::Error>(path)
        })();
        match result {
            Ok(path) => Some(path),
            Err(error) => {
                if error.code().0 as u32 != 0x8007_04c7 {
                    self.error("Ошибка диалога выбора файла", &error);
                }
                None
            }
        }
    }

    pub(super) fn command(&mut self, command: usize) {
        self.sync();
        if self.draft.is_some()
            && matches!(command, ADD_REVIEW | EDIT_REVIEW | REATTACH | DELETE_REVIEW | RESOLVE_REVIEW | UNDO | REDO | BOLD..=COL_DELETE | SHOW_SOURCE | MODES..=153)
        {
            self.status(Some("Примените черновик или отмените его."));
            return;
        }
        match command {
            OPEN | FOLDER if !self.wait => {
                if let Some(path) = self.choose(command == FOLDER, false, None)
                    && let Err(error) = self.open(path, 0)
                {
                    self.error("Не удалось открыть документ", &error);
                }
            }
            SAVE => {
                if let Some(i) = self.active {
                    self.save(i);
                }
            }
            SAVE_AS if !self.wait => self.save_as(),
            CLOSE => self.close_tab(),
            BACK => self.back(),
            FIND | HEADINGS => self.show_find(command == HEADINGS),
            FIND_NEXT | FIND_PREV => self.find(command == FIND_PREV),
            CLOSE_FIND => {
                self.find_open = false;
                self.layout();
                unsafe {
                    let _ = SetFocus(Some(self.editor));
                }
            }
            TREE if !self.wait => {
                if self.workspace.is_none() {
                    if let Some(path) = self.choose(true, false, None) {
                        self.folder(path);
                    }
                } else {
                    self.tree_open = !self.tree_open;
                    self.layout();
                }
            }
            REVIEWS => {
                self.review_open = !self.review_open;
                self.refresh_reviews();
                self.layout();
            }
            ZOOM_IN | ZOOM_OUT | ZOOM_RESET => {
                self.zoom = match command {
                    ZOOM_IN => (self.zoom + 10).min(250),
                    ZOOM_OUT => (self.zoom - 10).max(60),
                    _ => 100,
                };
                self.apply_zoom();
            }
            UNDO | REDO if self.mode() >= 2 => {
                if let Some(i) = self.active
                    && let Some(pos) = self.docs[i].undo(command == REDO)
                {
                    self.refresh(pos);
                }
            }
            THEME => {
                self.store_position();
                self.use_system = !self.use_system;
                self.theme();
                self.refresh_viewport();
            }
            MODES..=153 | SHOW_SOURCE if self.active.is_some() => {
                let target = if command == SHOW_SOURCE {
                    self.mode() | 1
                } else {
                    command - MODES
                };
                let show_selection = command == SHOW_SOURCE;
                let mut leaving = None;
                if self.mode() != target {
                    self.store_position();
                    let document = &mut self.docs[self.active.expect("document selected")];
                    if document.mode % 2 != target % 2 {
                        let position = document.viewport.clone();
                        if let Some((expected, previous)) = document.mode_round_trip.take()
                            && expected == position
                        {
                            document.viewport = previous;
                        }
                        leaving = Some(position);
                    }
                    document.mode = target;
                    self.refresh_viewport();
                }
                if show_selection {
                    send(self.editor, EM_SCROLLCARET, 0, 0);
                }
                unsafe {
                    let _ = SetFocus(Some(self.editor));
                }
                if let Some(position) = leaving {
                    let arrival = self.capture_viewport();
                    self.docs[self.active.expect("document selected")].mode_round_trip =
                        Some((arrival, position));
                }
            }
            ADD_REVIEW => self.start_review(false, false),
            EDIT_REVIEW => self.start_review(true, false),
            REATTACH => self.start_review(true, true),
            RESOLVE_REVIEW | DELETE_REVIEW => self.change_review(command),
            REVIEW_SAVE => self.save_review(),
            REVIEW_CANCEL => {
                self.draft = None;
                send(
                    self.editor,
                    EM_SETREADONLY,
                    usize::from(self.mode() < 2 || !self.projection_safe || self.draft.is_some()),
                    0,
                );
                self.layout();
                unsafe {
                    let _ = SetFocus(Some(self.editor));
                }
            }
            OPEN_LINK => self.follow_at(selection(self.editor).start),
            PARAGRAPH | LINE_BREAK if self.mode() == 2 && self.draft.is_none() => {
                let text = wide(if command == PARAGRAPH {
                    "\r"
                } else {
                    "\u{000b}"
                });
                send(self.editor, EM_REPLACESEL, 0, text.as_ptr() as isize);
                self.sync();
            }
            FORMAT | TABLE_TOOLS => {
                if let Some((button, _)) = self
                    .toolbar
                    .iter()
                    .find(|(btn, _)| unsafe { GetDlgCtrlID(*btn) as usize == command })
                {
                    self.popup(
                        *button,
                        if command == FORMAT {
                            FORMAT_ITEMS
                        } else {
                            TABLE_ITEMS
                        },
                        None,
                    );
                }
            }
            BOLD | ITALIC | STRIKE | CODE | LINK | HEADING | BULLET => self.format(command),
            ROW_ADD | ROW_DELETE | COL_ADD | COL_DELETE => self.table_edit(command),
            200 => {
                let mut controls = vec![self.editor];
                if self.active.is_some() && !self.wait {
                    controls.push(self.tab);
                }
                if self.tree_open {
                    controls.push(self.tree);
                }
                if self.review_open && self.active.is_some() {
                    controls.push(self.review_list);
                }
                if self.find_open {
                    controls.push(self.find_box);
                }
                let focus = unsafe { GetFocus() };
                let next = controls
                    .iter()
                    .position(|&hwnd| hwnd == focus)
                    .map_or(0, |p| (p + 1) % controls.len());
                unsafe {
                    let _ = SetFocus(Some(controls[next]));
                }
            }
            210 | 211 => {
                let focus = unsafe { GetFocus() };
                let list = self
                    .controls()
                    .chain([self.editor])
                    .filter(|&hwnd| unsafe {
                        IsWindowVisible(hwnd).as_bool()
                            && IsWindowEnabled(hwnd).as_bool()
                            && GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_TABSTOP.0 != 0
                    })
                    .collect::<Vec<_>>();
                if !list.is_empty() {
                    let step = if command == 210 { 1 } else { list.len() - 1 };
                    let next = list
                        .iter()
                        .position(|&hwnd| hwnd == focus)
                        .map_or(0, |i| (i + step) % list.len());
                    unsafe {
                        let _ = SetFocus(Some(list[next]));
                    }
                }
            }
            201 | 202 => {
                if let Some(i) = self.active {
                    let step = if command == 201 {
                        1
                    } else {
                        self.docs.len() - 1
                    };
                    self.switch((i + step) % self.docs.len());
                }
            }
            204 => self.popup(
                self.review_actions[3],
                &[
                    (REATTACH, "Перепривязать к выделению"),
                    (205, "К сохранённому заголовку"),
                    (DELETE_REVIEW, "Удалить комментарий"),
                ],
                None,
            ),
            205 => self.review_heading(),
            _ => {}
        }
    }

    fn apply_zoom(&self) {
        send(self.editor, EM_SETZOOM, self.zoom as usize, 100);
        set_text(self.zoom_status, &format!("{}%", self.zoom));
    }
}
