use super::*;
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    File,
    Folder,
    SaveAs,
    Image,
}

impl App {
    pub(super) fn open(&mut self, path: PathBuf, mode: usize) -> Result<()> {
        if self.draft.is_some() {
            return Err(tr(L::FinishDraft).into());
        }
        let path = fs::canonicalize(path)?;
        if path.is_dir() {
            if self.wait {
                return Err(tr(L::WaitNeedsFile).into());
            }
            return self.folder(path);
        }
        if let Some(index) = self
            .docs
            .iter()
            .position(|doc| doc.path() == Some(path.as_path()))
        {
            self.switch(index);
            return Ok(());
        }
        if self.wait && self.active.is_some() {
            return Ok(());
        }
        self.add_document(Document::open(path, mode)?);
        Ok(())
    }

    fn add_document(&mut self, doc: Document) {
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
    }

    fn folder(&mut self, path: PathBuf) -> Result<()> {
        let entries = folder_entries(&path)?;
        self.tree_open = true;
        self.outline_open = false;
        if self.nav_needs_room() {
            self.review_open = false;
        }
        set_text(
            self.outline,
            &format!(
                "{} · {}",
                tr(L::Files),
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            ),
        );
        self.workspace = Some(path.clone());
        self.set_tree(path, entries, &HashSet::new());
        self.layout();
        Ok(())
    }

    fn refresh_folder(&mut self) {
        let Some(path) = self.workspace.clone() else {
            return;
        };
        let entries = match folder_entries(&path) {
            Ok(entries) => entries,
            Err(error) => {
                self.error(tr(L::FolderError), &error);
                return;
            }
        };
        let expanded = self
            .tree_entries
            .iter()
            .filter(|entry| {
                entry.loaded
                    && send(
                        self.tree,
                        TVM_GETITEMSTATE,
                        entry.item.0 as usize,
                        TVIS_EXPANDED.0 as isize,
                    ) != 0
            })
            .map(|entry| entry.path.clone())
            .collect();
        self.set_tree(path, entries, &expanded);
    }

    fn set_tree(&mut self, path: PathBuf, entries: Vec<FolderEntry>, expanded: &HashSet<PathBuf>) {
        send(self.tree, TVM_DELETEITEM, 0, TVI_ROOT.0);
        self.tree_entries.clear();
        let root = self.tree_insert(TVI_ROOT, path, true);
        self.load_children(root, 0, entries);
        send(self.tree, TVM_EXPAND, TVE_EXPAND.0 as usize, root.0);
        // Adding children extends this list, so parents are always restored first.
        let mut next = 1;
        while let Some(entry) = self.tree_entries.get(next) {
            if expanded.contains(&entry.path)
                && let Ok(entries) = folder_entries(&entry.path)
            {
                let item = entry.item;
                self.load_children(item, next, entries);
                send(self.tree, TVM_EXPAND, TVE_EXPAND.0 as usize, item.0);
            }
            next += 1;
        }
    }

    fn tree_insert(&mut self, parent: HTREEITEM, path: PathBuf, folder: bool) -> HTREEITEM {
        let mut name = wide(
            &path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
        );
        let index = self.tree_entries.len();
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
        let item = HTREEITEM(send(
            self.tree,
            TVM_INSERTITEMW,
            0,
            &item as *const _ as isize,
        ));
        self.tree_entries.push(TreeEntry {
            item,
            path,
            folder,
            loaded: false,
        });
        item
    }

    pub(super) fn expand(&mut self, item: HTREEITEM, index: usize) {
        let Some(entry) = self.tree_entries.get(index) else {
            return;
        };
        if !entry.folder || entry.loaded {
            return;
        }
        match folder_entries(&entry.path) {
            Ok(entries) => self.load_children(item, index, entries),
            Err(error) => {
                self.error(tr(L::FolderError), &error);
            }
        }
    }

    fn load_children(&mut self, item: HTREEITEM, index: usize, entries: Vec<FolderEntry>) {
        self.tree_entries[index].loaded = true;
        if entries.is_empty() {
            let info = TVITEMW {
                mask: TVIF_CHILDREN,
                hItem: item,
                cChildren: TVITEMEXW_CHILDREN(0),
                ..Default::default()
            };
            send(self.tree, TVM_SETITEMW, 0, &info as *const _ as isize);
        }
        for entry in entries {
            self.tree_insert(item, entry.path, entry.directory);
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
                    self.error(tr(L::OpenError), &error);
                }
            }
            DragFinish(drop);
        }
    }

    pub(super) fn tab_name(&self, index: usize) -> String {
        let doc = &self.docs[index];
        format!("{}{}", doc.name(), if doc.dirty() { " *" } else { "" })
    }

    fn title(&self) {
        if let Some(index) = self.active {
            self.update_tab_title(index);
        } else {
            set_text(self.hwnd, "pane-md");
        }
    }

    fn update_tab_title(&self, index: usize) {
        let name = self.tab_name(index);
        let mut title = wide(&name);
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(title.as_mut_ptr()),
            ..Default::default()
        };
        send(self.tab, TCM_SETITEMW, index, &item as *const _ as isize);
        if self.active == Some(index) {
            set_text(self.hwnd, &format!("{name} — pane-md"));
        }
    }

    pub(super) fn store_position(&mut self) {
        self.flush_edit();
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

    pub(super) fn restore_viewport(&mut self, viewport: &Viewport) {
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
        if self.draft.is_some() {
            self.status(Some(tr(L::FinishDraft)));
            send(self.tab, TCM_SETCURSEL, self.active.unwrap(), 0);
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
        self.review_expanded = false;
        self.review_all = false;
    }

    pub(super) fn refresh(&mut self, position: usize) {
        self.source_parse_dirty = false;
        if let Some(i) = self.active {
            let source = &self.docs[i].text;
            self.parsed = Markdown::parse(source);
            let trailing = self
                .parsed
                .owners
                .last()
                .and_then(|owner| source.get(owner.source.end..));
            let empty_eof = source.trim().is_empty()
                || position == source.len()
                    && trailing.is_some_and(|tail| {
                        tail.trim().is_empty()
                            && pane_md::normalize_newlines(tail, "\n").contains("\n\n")
                    });
            if self.mode() == 2 && empty_eof {
                self.parsed.push_empty_paragraph(source.len());
            }
            send(self.tab, TCM_SETCURSEL, i, 0);
        } else {
            self.parsed = Markdown::default();
        }
        self.rebuild_editor(position);
        self.refresh_reviews();
        self.refresh_outline();
        self.title();
        self.update_controls();
    }

    fn update_controls(&mut self) {
        self.layout();
        self.bar.update(self.active.is_some(), self.display_state());
        self.status(None);
    }

    pub(super) fn refresh_outline(&self) {
        if !self.outline_open {
            return;
        }
        set_text(self.outline, tr(L::Outline));
        let current = self.current_position();
        let mut nearest = None;
        let displayed = unsafe { IsWindowVisible(self.outline_list).as_bool() };
        if displayed {
            send(self.outline_list, WM_SETREDRAW, 0, 0);
        }
        send(self.outline_list, LB_RESETCONTENT, 0, 0);
        for region in self.parsed.regions.iter().filter(|r| r.level > 0) {
            let label = wide(&format!(
                "{}{}",
                "   ".repeat((region.level - 1) as usize),
                region.text
            ));
            let row = send(self.outline_list, LB_ADDSTRING, 0, label.as_ptr() as isize);
            if row >= 0 && region.source.start <= current {
                nearest = Some(row as usize);
            }
        }
        if let Some(row) = nearest {
            send(self.outline_list, LB_SETCURSEL, row, 0);
        }
        if displayed {
            send(self.outline_list, WM_SETREDRAW, 1, 0);
            unsafe {
                let _ = InvalidateRect(Some(self.outline_list), None, true);
            }
        }
    }

    pub(super) fn jump_outline(&mut self) {
        self.flush_edit();
        self.parse_source_edits();
        let row = send(self.outline_list, LB_GETCURSEL, 0, 0);
        if let Ok(index) = usize::try_from(row)
            && let Some(region) = self
                .parsed
                .regions
                .iter()
                .filter(|r| r.level > 0)
                .nth(index)
        {
            self.jump_source(region.source.start);
        }
    }

    fn render_data(&mut self) -> (String, Projection) {
        let style = self.appearance.style(self.dark);
        let Some(i) = self.active else {
            self.diagrams = Diagrams::default();
            return (style.rtf() + "}", Projection::default());
        };
        if self.mode() % 2 == 1 {
            let view = Projection::source(&self.docs[i].text);
            let rtf = source_rtf(&view, &self.parsed, &style, self.docs[i].show_reviews);
            (rtf, view)
        } else {
            prepare_diagrams(
                &mut self.diagrams,
                self.parsed
                    .regions
                    .iter()
                    .filter(|region| region.mermaid)
                    .map(|region| region.text.as_str()),
                style.dark,
                self.hwnd,
            );
            render(
                &self.parsed,
                &style,
                10_800,
                self.docs[i].path().and_then(Path::parent),
                &self.diagrams,
                &self.remote_images,
                self.docs[i].show_reviews,
            )
        }
    }

    pub(super) fn refresh_diagrams(&mut self) {
        let diagrams = self.diagrams.receive();
        let (images, failed) = self.remote_images.receive();
        if !(diagrams || images || failed) || !self.mode().is_multiple_of(2) {
            return;
        }
        let Some(i) = self.active else { return };
        self.store_position();
        let viewport = self.docs[i].viewport.clone();
        self.rebuild_editor(viewport.selection.start);
        self.restore_viewport(&viewport);
        self.status(failed.then(|| tr(L::RemoteImageUnavailable)));
    }

    fn rebuild_editor(&mut self, position: usize) {
        let (rtf, view) = self.render_data();
        self.view = view;
        self.update_ruler();
        send(self.editor, WM_SETREDRAW, 0, 0);
        send(self.editor, EM_SETREADONLY, 0, 0);
        let loaded = stream(self.editor, &rtf);
        let text = buffer(self.editor);
        if self.view.text.last() == Some(&13)
            && text == self.view.text[..self.view.text.len() - 1]
            && self.mode().is_multiple_of(2)
        {
            self.view.text.pop();
        }
        self.projection_safe = loaded && text == self.view.text;
        if !self.projection_safe && self.active.is_some() {
            eprintln!(
                "pane-md: RichEdit projection differs ({} native, {} expected)",
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
            if warnings == 0 && self.view.diagram_fallbacks == 0 && self.view.diagram_pending == 0 {
                return String::new();
            }
            let path = doc.path().map_or_else(|| doc.name(), Path::to_string_lossy);
            let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
            let family = if doc.mode.is_multiple_of(2) {
                tr(L::RichFamily)
            } else {
                tr(L::SourceFamily)
            };
            let stage = if doc.mode >= 2 {
                tr(L::Edit)
            } else if doc.show_reviews {
                tr(L::Review)
            } else {
                tr(L::View)
            };
            let mut text = format!("{family} · {stage}    {path}");
            if self.view.diagram_fallbacks != 0 {
                text.push_str(&format!(
                    "    {}: {}",
                    tr(L::MermaidFallback),
                    self.view.diagram_fallbacks
                ));
            }
            if self.view.diagram_pending != 0 {
                text.push_str(&format!(
                    "    {}: {}",
                    tr(L::MermaidPending),
                    self.view.diagram_pending
                ));
            }
            if warnings > 0 {
                format!("{text}    {}: {warnings}", tr(L::BrokenReviews))
            } else {
                text
            }
        });
        let default = if self.active.is_some() && !self.projection_safe && self.mode() >= 2 {
            tr(L::ProjectionError)
        } else {
            info.as_deref().unwrap_or("")
        };
        set_text(self.status, override_text.unwrap_or(default));
    }

    pub(super) fn flush_edit(&mut self) {
        if self.sync_pending {
            self.sync();
        }
    }

    pub(super) fn sync(&mut self) {
        self.sync_pending = false;
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
                .and_then(|units| String::from_utf16(units).ok())
                .filter(|text| {
                    !text
                        .chars()
                        .any(|ch| matches!(ch, '\u{fff9}'..='\u{fffc}' | '\u{0007}' | '\0'))
                });
            let inline = self
                .view
                .region_range(old.clone())
                .zip(inserted.as_deref())
                .and_then(|((region, selection), text)| {
                    let item = &self.parsed.regions[region];
                    if self.view.diagram_regions.contains(&region)
                        || text.is_empty() && item.removes_inline_run(&selection)
                    {
                        return None;
                    }
                    item.patch(source, selection, text)
                });
            if inline.is_none()
                && let Some(text) = inserted
                && let Some(edit) =
                    self.visual_replacement(old.clone(), &text.replace('\u{000b}', "\n"))
            {
                self.patch(edit.range, &edit.text, edit.caret.start);
                return;
            }
            inline
        };
        if let Some((range, text)) = patch {
            let position = range.start + text.len();
            let live_edit = self.mode() == 2
                && apply_plain_edit(&mut self.parsed, &mut self.view, source, old, &range, &text);
            self.docs[i].patch(range, &text, true);
            if self.mode() == 3 {
                self.source_parse_dirty = true;
            }
            if live_edit {
                self.view.text = after;
            } else if self.mode() == 3 {
                self.view = Projection::source(&self.docs[i].text);
                self.update_ruler();
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
            if self.outline_open || self.mode() == 3 {
                unsafe {
                    SetTimer(Some(self.hwnd), REFRESH_SIDEBARS, 250, None);
                }
            }
        } else {
            self.rebuild_editor(previous_position);
            set_scroll(self.editor, scroll);
            self.status(Some(tr(L::EditSourceHint)));
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
        if self.docs[index].path().is_none() {
            return self.save_as(index);
        }
        match self.docs[index].save() {
            Ok(()) => {
                self.update_tab_title(index);
                self.status(Some(tr(L::Saved)));
                true
            }
            Err(error) => {
                self.error(tr(L::SaveError), &error);
                false
            }
        }
    }

    fn save_as(&mut self, index: usize) -> bool {
        let name = self.docs[index].suggested_filename();
        let Some(path) = self.choose(Pick::SaveAs, Some(&name)) else {
            return false;
        };
        let previous = self.docs[index].path().map(Path::to_owned);
        match self.docs[index].save_as(path) {
            Ok(()) => {
                self.history
                    .saved_as(&self.docs[index], previous.as_deref());
                if self.active == Some(index) {
                    self.store_position();
                    self.refresh_viewport();
                } else {
                    self.update_tab_title(index);
                }
                if let Some(root) = &self.workspace
                    && self.docs[index]
                        .path()
                        .is_some_and(|path| path.starts_with(root))
                {
                    self.refresh_folder();
                }
                self.status(Some(tr(L::Saved)));
                true
            }
            Err(error) => {
                self.error(tr(L::SaveAsError), &error);
                false
            }
        }
    }

    pub(super) fn confirm_close(&mut self, index: usize) -> bool {
        if !self.docs[index].dirty() {
            return true;
        }
        let text = tr(L::ConfirmSave).replacen("{file}", &self.docs[index].name(), 1);
        let result = unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(wide(&text).as_ptr()),
                w!("pane-md"),
                MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON1,
            )
        };
        match result {
            IDYES => self.save(index),
            IDNO => true,
            _ => false,
        }
    }

    pub(super) fn close_tab(&mut self, i: usize) {
        if self.draft.is_some() {
            self.status(Some(tr(L::FinishDraft)));
            return;
        }
        if self.wait {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return;
        }
        if i >= self.docs.len() || !self.confirm_close(i) {
            return;
        }
        let previous = self.active;
        self.docs.remove(i);
        send(self.tab, TCM_DELETEITEM, i, 0);
        if let Some(active) = previous
            && active != i
        {
            let index = active - usize::from(active > i);
            self.active = Some(index);
            send(self.tab, TCM_SETCURSEL, index, 0);
            return;
        }
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

    fn choose(&self, pick: Pick, name: Option<&str>) -> Option<PathBuf> {
        let directory = pick == Pick::Folder;
        let save = pick == Pick::SaveAs;
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
            if save {
                dialog.SetDefaultExtension(w!("md"))?;
            }
            if let Some(name) = name {
                dialog.SetFileName(PCWSTR(wide(name).as_ptr()))?;
            }
            if pick == Pick::Image {
                use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
                let label = wide(tr(L::ImageFiles));
                let patterns = crate::images::EXTENSIONS
                    .iter()
                    .map(|ext| format!("*.{ext}"))
                    .collect::<Vec<_>>()
                    .join(";");
                let patterns = wide(&patterns);
                dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                    pszName: PCWSTR(label.as_ptr()),
                    pszSpec: PCWSTR(patterns.as_ptr()),
                }])?;
            }
            let title = wide(tr(match pick {
                Pick::File => L::OpenDocument,
                Pick::Folder => L::OpenFolder,
                Pick::SaveAs => L::SaveAsDialog,
                Pick::Image => L::InsertImage,
            }));
            dialog.SetTitle(PCWSTR(title.as_ptr()))?;
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
                    self.error(tr(L::DialogError), &error);
                }
                None
            }
        }
    }

    pub(super) fn insert_image(&mut self, path: &Path) -> Result<()> {
        if self.mode() != 2 || !self.projection_safe || self.draft.is_some() {
            return Err(tr(L::UnsupportedEdit).into());
        }
        let index = self.active.ok_or(tr(L::OpenFirst))?;
        let selected = selection(self.editor);
        let region = self
            .view
            .region_range(selected.start..selected.start)
            .map(|(index, _)| index);
        let doc = &self.docs[index];
        let document = doc.path().ok_or(tr(L::SaveBeforeRelative))?;
        let edit = crate::images::insertion(document, &doc.text, &self.parsed, region, path)?;
        self.patch(edit.range, &edit.text, edit.caret.start);
        Ok(())
    }

    pub(super) fn command(&mut self, command: usize) {
        self.flush_edit();
        if self.draft.is_some()
            && matches!(command, NEW_NOTE | SHOW_REPLACE | REPLACE_ONE | REPLACE_ALL | INSERT_IMAGE | HEADING_FIRST..=HEADING_LAST | NUMBERED_LIST | MAKE_TASK | TOGGLE_TASK | LINT_NEXT | LINT_PREV | VIEW_RICH | VIEW_SOURCE | OPEN | FOLDER | BACK | FORWARD_LINK | CLOSE | ADD_REVIEW | EDIT_REVIEW | REATTACH | DELETE_REVIEW | RESOLVE_REVIEW | UNDO | REDO | BOLD..=COL_DELETE | SHOW_SOURCE | MODES..=153 | FAMILY..=EDITING | REVIEW_ALL | REVIEW_COLLAPSE | CUT_SELECTION)
        {
            self.status(Some(tr(L::FinishDraft)));
            return;
        }
        if matches!(
            command,
            REVIEWS | ADD_REVIEW | EDIT_REVIEW | REATTACH | RESOLVE_REVIEW | DELETE_REVIEW | 205
        ) {
            self.parse_source_edits();
        }
        match command {
            NEW_NOTE if !self.wait => {
                self.add_document(Document::blank());
                unsafe {
                    let _ = SetFocus(Some(self.editor));
                }
            }
            OPEN | FOLDER if !self.wait => {
                if let Some(path) = self.choose(
                    if command == FOLDER {
                        Pick::Folder
                    } else {
                        Pick::File
                    },
                    None,
                ) && let Err(error) = self.open(path, 0)
                {
                    self.error(tr(L::OpenError), &error);
                }
            }
            SAVE => {
                if self.draft.is_some() {
                    self.save_review();
                    if self.draft.is_some() {
                        return;
                    }
                }
                if let Some(i) = self.active {
                    self.save(i);
                }
            }
            SAVE_AS if !self.wait => {
                if let Some(i) = self.active {
                    self.save_as(i);
                }
            }
            CLOSE => {
                if let Some(i) = self.active {
                    self.close_tab(i);
                }
            }
            BACK | FORWARD_LINK => self.walk_link_history(command == FORWARD_LINK),
            FIND | HEADINGS => self.show_find(command == HEADINGS),
            SHOW_REPLACE => self.show_replace(),
            REPLACE_ONE | REPLACE_ALL => self.replace_matches(command == REPLACE_ALL),
            FIND_OPTIONS if !self.find_headings => self.search_options_menu(),
            MATCH_CASE => self.search_options.case_sensitive = !self.search_options.case_sensitive,
            WHOLE_WORD => self.search_options.whole_word = !self.search_options.whole_word,
            FIND_NEXT | FIND_PREV => self.find(command == FIND_PREV),
            LINT_NEXT | LINT_PREV => {
                self.parse_source_edits();
                self.lint_jump(command == LINT_PREV);
            }
            CLOSE_FIND => {
                self.find_open = false;
                self.replace_open = false;
                self.layout();
                unsafe {
                    let _ = SetFocus(Some(self.editor));
                }
            }
            REFRESH_FILES if !self.wait => self.refresh_folder(),
            TREE if !self.wait => {
                if self.workspace.is_none() {
                    if let Some(path) = self.choose(Pick::Folder, None)
                        && let Err(error) = self.open(path, 0)
                    {
                        self.error(tr(L::FolderError), &error);
                    }
                } else {
                    let hidden = self.review_open && self.nav_needs_room();
                    self.tree_open = !self.tree_open || self.outline_open || hidden;
                    self.outline_open = false;
                    if self.tree_open && self.nav_needs_room() {
                        self.review_open = false;
                    }
                    let root = self.workspace.as_ref().unwrap();
                    set_text(
                        self.outline,
                        &format!(
                            "{} · {}",
                            tr(L::Files),
                            root.file_name()
                                .unwrap_or(root.as_os_str())
                                .to_string_lossy()
                        ),
                    );
                    self.layout();
                }
            }
            OUTLINE if !self.wait && self.active.is_some() => {
                let hidden = self.review_open && self.nav_needs_room();
                self.outline_open = !self.outline_open || hidden;
                self.tree_open = false;
                if self.outline_open && self.nav_needs_room() {
                    self.review_open = false;
                }
                self.parse_source_edits();
                self.refresh_outline();
                self.layout();
            }
            REVIEWS => {
                if self.draft.is_none() {
                    if self.active.is_some() && !self.show_review_marks() {
                        self.review_open = true;
                        self.change_mode(self.mode() % 2, true);
                    } else {
                        self.review_open = !self.review_open;
                        self.layout();
                    }
                }
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
            THEME | THEME_LIGHT | THEME_DARK => {
                self.store_position();
                self.theme_choice = command;
                settings::write_preference("Theme", command as u32);
                self.refresh_menu();
                self.theme();
                self.refresh_viewport();
            }
            RICH_FONT..=RESET_APPEARANCE => self.choose_appearance(command),
            LANG_RU | LANG_EN => {
                self.store_position();
                i18n::set_language(command == LANG_EN);
                settings::write_preference("Language", u32::from(command == LANG_EN));
                self.localize_controls();
                self.refresh_viewport();
                for index in 0..self.docs.len() {
                    self.update_tab_title(index);
                }
            }
            DISPLAY if self.active.is_some() => {
                self.popup(
                    self.bar.mode,
                    &[
                        (READ, tr(L::View)),
                        (COMMENTS, tr(L::Review)),
                        (EDITING, tr(L::Edit)),
                    ],
                    None,
                );
            }
            READ | COMMENTS | EDITING if self.active.is_some() => {
                if command == COMMENTS {
                    self.review_open = true;
                }
                self.change_mode(
                    self.mode() % 2 + if command == EDITING { 2 } else { 0 },
                    command != READ,
                );
                self.layout();
            }
            FAMILY | VIEW_RICH | VIEW_SOURCE | MODES..=153 | SHOW_SOURCE
                if self.active.is_some() =>
            {
                let target = match command {
                    FAMILY => self.mode() ^ 1,
                    VIEW_RICH => self.mode() & !1,
                    VIEW_SOURCE => self.mode() | 1,
                    SHOW_SOURCE => self.mode() | 1,
                    _ => command - MODES,
                };
                self.change_mode(target, self.show_review_marks() || target >= 2);
                if command == SHOW_SOURCE {
                    send(self.editor, EM_SCROLLCARET, 0, 0);
                    let visible = self.capture_viewport();
                    if let Some((expected, _)) =
                        self.docs[self.active.unwrap()].mode_round_trip.as_mut()
                    {
                        *expected = visible;
                    }
                }
            }
            REVIEW_ALL if self.active.is_some() => {
                self.parse_source_edits();
                if !self.review_all {
                    self.review_open = true;
                    if !self.show_review_marks() {
                        self.change_mode(self.mode() % 2, true);
                    }
                }
                self.review_all = !self.review_all;
                self.review_expanded = false;
                self.layout();
                if self.review_all {
                    unsafe {
                        let _ = SetFocus(Some(self.review_list));
                    }
                }
            }
            REVIEW_COLLAPSE => {
                self.review_expanded = false;
                self.layout_review_panel();
                unsafe {
                    let _ = SetFocus(Some(
                        self.review_cards
                            .first()
                            .map(|card| card.body)
                            .unwrap_or(self.review_all_button),
                    ));
                }
            }
            CARD_BASE..=495 if self.draft.is_none() => {
                if let Some(&index) = self.card_rows.get(command - CARD_BASE)
                    && let Some(row) = self.review_indices.iter().position(|&i| i == index)
                {
                    send(self.review_list, LB_SETCURSEL, row, 0);
                    self.review_select(true);
                }
            }
            REVIEW_DETAILS if self.review_record().is_some() && self.draft.is_none() => {
                self.review_expanded = true;
                self.review_all = false;
                self.layout_review_panel();
                unsafe {
                    let _ = SetFocus(Some(self.review_text));
                }
            }
            CARD_MENU_BASE..=595 if self.draft.is_none() => {
                self.review_card_menu(command - CARD_MENU_BASE);
            }
            COPY_SELECTION => {
                send(self.editor, WM_COPY, 0, 0);
            }
            CUT_SELECTION => self.cut_selection(),
            ADD_REVIEW => self.start_review(false, false),
            EDIT_REVIEW => self.start_review(true, false),
            REATTACH => self.start_review(true, true),
            RESOLVE_REVIEW | DELETE_REVIEW => self.change_review(command),
            REVIEW_SAVE => self.save_review(),
            REVIEW_COMMIT => {
                let comment = self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.format.is_none());
                self.save_review();
                if comment
                    && self.draft.is_none()
                    && let Some(index) = self.active
                {
                    self.save(index);
                }
            }
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
            TOGGLE_TASK => self.toggle_task(selection(self.editor).start),
            LINE_BREAK if self.mode() == 2 && self.draft.is_none() && self.projection_safe => {
                let change = self.active.and_then(|document| {
                    let (region, local) = self.view.region_range(selection(self.editor))?;
                    self.parsed.regions[region].patch(&self.docs[document].text, local, "\u{000b}")
                });
                if let Some((range, replacement)) = change {
                    let caret = range.start + replacement.len();
                    self.patch(range, &replacement, caret);
                } else {
                    self.status(Some(tr(L::EditSourceHint)));
                }
            }
            PARAGRAPH if self.mode() == 2 && self.draft.is_none() => {
                if self.continue_list() {
                    return;
                }
                let text = wide("\r");
                send(self.editor, EM_REPLACESEL, 0, text.as_ptr() as isize);
                self.sync();
            }
            BOLD
            | ITALIC
            | STRIKE
            | CODE
            | LINK
            | HEADING
            | BULLET
            | NUMBERED_LIST
            | MAKE_TASK
            | HEADING_FIRST..=HEADING_LAST => self.format(command),
            INSERT_IMAGE if self.mode() == 2 && self.projection_safe => {
                if self.active.is_some_and(|i| self.docs[i].path().is_none()) {
                    self.status(Some(tr(L::SaveBeforeRelative)));
                    return;
                }
                if let Some(path) = self.choose(Pick::Image, None)
                    && let Err(error) = self.insert_image(&path)
                {
                    self.error(tr(L::InsertImage), error.as_ref());
                }
            }
            ROW_ADD | ROW_DELETE | COL_ADD | COL_DELETE => self.table_edit(command),
            200 => {
                let mut controls = vec![self.editor];
                if self.active.is_some() && !self.wait {
                    controls.push(self.tab);
                }
                if unsafe { IsWindowVisible(self.tree).as_bool() } {
                    controls.push(self.tree);
                }
                if unsafe { IsWindowVisible(self.outline_list).as_bool() } {
                    controls.push(self.outline_list);
                }
                if let Some(control) = [self.draft_box, self.review_list, self.review_actions[0]]
                    .into_iter()
                    .find(|&hwnd| unsafe {
                        IsWindowVisible(hwnd).as_bool() && IsWindowEnabled(hwnd).as_bool()
                    })
                {
                    controls.push(control);
                }
                if self.find_open {
                    controls.push(self.find_box);
                    if self.replace_open && self.mode() == 3 && self.draft.is_none() {
                        controls.push(self.replace_box);
                    }
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
                    (REATTACH, tr(L::Reattach)),
                    (205, tr(L::SavedHeading)),
                    (RESOLVE_REVIEW, tr(L::ToggleResolved)),
                ],
                None,
            ),
            205 => self.review_heading(),
            _ => {}
        }
    }

    pub(super) fn display_state(&self) -> usize {
        if self.mode() >= 2 {
            EDITING
        } else if self.show_review_marks() {
            COMMENTS
        } else {
            READ
        }
    }

    fn refresh_menu(&self) {
        if let Ok(menu) = menu_bar(self.theme_choice) {
            unsafe {
                let old = GetMenu(self.hwnd);
                if SetMenu(self.hwnd, Some(menu)).is_ok() {
                    self.theme_menu();
                    if !old.is_invalid() {
                        let _ = DestroyMenu(old);
                    }
                } else {
                    let _ = DestroyMenu(menu);
                }
            }
        }
    }

    fn localize_controls(&mut self) {
        self.refresh_menu();
        set_text(self.open_tab, tr(L::OpenTab));
        set_text(self.bar.rich, tr(L::RichFamily));
        set_text(self.bar.source, tr(L::SourceFamily));
        for card in &self.review_cards {
            set_text(card.menu, tr(L::More));
        }
        set_text(self.empty, tr(L::EmptyEditor));
        if !self.outline_open
            && let Some(root) = self.workspace.as_ref()
        {
            set_text(
                self.outline,
                &format!(
                    "{} · {}",
                    tr(L::Files),
                    root.file_name()
                        .unwrap_or(root.as_os_str())
                        .to_string_lossy()
                ),
            );
        }
        for (hwnd, name) in
            self.review_actions
                .into_iter()
                .zip([L::AddComment, L::ReviewEdit, L::Delete, L::More])
        {
            set_text(hwnd, tr(name));
        }
        set_text(self.review_collapse, tr(L::Collapse));
        for (hwnd, name) in self
            .draft_actions
            .into_iter()
            .zip([L::SaveComment, L::Cancel])
        {
            set_text(hwnd, tr(name));
        }
        for (hwnd, name) in
            self.find_actions
                .into_iter()
                .zip([L::Back, L::Next, L::SearchOptions, L::Close])
        {
            set_text(hwnd, tr(name));
        }
        for (hwnd, name) in self
            .replace_actions
            .into_iter()
            .zip([L::ReplaceOne, L::ReplaceAll])
        {
            set_text(hwnd, tr(name));
        }
        self.update_find_hint();
        self.card_rows.clear();
    }

    pub(super) fn change_mode(&mut self, target: usize, marks: bool) {
        let Some(index) = self.active else { return };
        let family_change = self.mode() % 2 != target % 2;
        let marks_change = self.show_review_marks() != marks;
        if family_change || marks_change {
            self.store_position();
            let document = &mut self.docs[index];
            let leaving = document.viewport.clone();
            if family_change
                && let Some((expected, previous)) = document.mode_round_trip.take()
                && expected == leaving
            {
                document.viewport = previous;
            }
            document.mode = target;
            document.show_reviews = marks;
            self.refresh_viewport();
            if family_change {
                let arrival = self.capture_viewport();
                self.docs[index].mode_round_trip = Some((arrival, leaving));
            }
        } else if self.mode() != target {
            self.docs[index].mode = target;
            self.parse_source_edits();
            self.refresh_outline();
            send(
                self.editor,
                EM_SETREADONLY,
                usize::from(target < 2 || !self.projection_safe),
                0,
            );
            self.update_controls();
        }
        unsafe {
            let _ = SetFocus(Some(self.editor));
        }
    }

    fn apply_zoom(&self) {
        send(self.editor, EM_SETZOOM, self.zoom as usize, 100);
        set_text(self.zoom_buttons[1], &format!("{}%", self.zoom));
        unsafe {
            let _ = InvalidateRect(Some(self.zoom_buttons[1]), None, false);
        }
    }

    pub(super) fn parse_source_edits(&mut self) {
        if self.source_parse_dirty
            && let Some(index) = self.active
        {
            self.parsed = Markdown::parse(&self.docs[index].text);
            self.source_parse_dirty = false;
            self.refresh_reviews();
            self.layout();
        }
    }
}
