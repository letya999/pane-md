use pane_md::markdown::TableEdit;

use super::*;

impl App {
    pub(super) fn refresh_reviews(&mut self) {
        let displayed = unsafe { IsWindowVisible(self.review_list).as_bool() };
        if displayed {
            send(self.review_list, WM_SETREDRAW, 0, 0);
        }
        send(self.review_list, LB_RESETCONTENT, 0, 0);
        self.review_indices = (0..self.parsed.records.len()).collect();
        self.review_indices
            .sort_by_key(|&i| match &self.parsed.records[i].binding {
                Binding::Attached { line, column, .. } => (*line, *column),
                _ => (usize::MAX, i),
            });
        let mut selected = None;
        for (i, &record_index) in self.review_indices.iter().enumerate() {
            let record = &self.parsed.records[record_index];
            let location = match &record.binding {
                Binding::Attached { line, .. } => format!("{} {line}", tr(L::Line)),
                Binding::Missing => tr(L::ReviewUnattached).to_owned(),
                Binding::Ambiguous => tr(L::ReviewAmbiguous).to_owned(),
            };
            let state = if record.review.status == Status::Resolved {
                tr(L::ResolvedSuffix)
            } else {
                ""
            };
            let quote = record.review.quote.replace(['\r', '\n', '\t'], " ");
            let line = wide(&format!("{location}{state}   «{quote}»"));
            send(self.review_list, LB_ADDSTRING, 0, line.as_ptr() as isize);
            if self.active_review.as_ref() == Some(&record.review.id) {
                selected = Some(i);
            }
        }
        if let Some(i) = selected {
            send(self.review_list, LB_SETCURSEL, i, 0);
        }
        if displayed {
            send(self.review_list, WM_SETREDRAW, 1, 0);
            unsafe {
                let _ = InvalidateRect(Some(self.review_list), None, true);
            }
        }
        self.card_rows.clear();
        self.review_select(false);
        if self.active_review.is_none() {
            self.review_expanded = false;
        }
        self.layout_review_panel();
    }

    pub(super) fn review_select(&mut self, jump: bool) {
        let native = send(self.review_list, LB_GETCURSEL, 0, 0);
        let record = usize::try_from(native)
            .ok()
            .and_then(|n| self.review_indices.get(n))
            .and_then(|&i| self.parsed.records.get(i));
        self.active_review = record.map(|r| r.review.id.clone());
        if jump {
            self.review_expanded = false;
            self.review_all = false;
        }
        let preview = record.map_or(String::from(tr(L::EmptyReview)), |record| {
            let state = match record.binding {
                Binding::Missing => tr(L::UnattachedHint),
                Binding::Ambiguous => tr(L::AmbiguousHint),
                Binding::Attached { .. } if record.review.status == Status::Resolved => {
                    tr(L::ReviewResolved)
                }
                _ => tr(L::ReviewOpen),
            };
            let heading = if record.review.anchor.heading.is_empty() {
                ""
            } else {
                "\r\n"
            };
            format!(
                "«{}»{heading}{}\r\n{state} · {}\r\n\r\n{}",
                super::cards::excerpt(&record.review.quote, 150),
                record.review.anchor.heading,
                super::cards::review_date(record.review.updated_at),
                pane_md::normalize_newlines(&record.review.text, "\r\n")
            )
        });
        set_text(self.review_text, &preview);
        unsafe {
            let _ = ShowScrollBar(self.review_text, SB_VERT, record.is_some());
        }
        for (i, button) in self.review_actions.iter().enumerate() {
            unsafe {
                let _ = EnableWindow(
                    *button,
                    if i == 0 {
                        self.active.is_some()
                    } else {
                        record.is_some()
                    },
                );
            }
        }
        if jump {
            if let Some(span) =
                record.and_then(|r| self.view.review_range(&self.parsed, &r.binding))
            {
                select(self.editor, span, true);
                self.status(None);
            }
            self.layout_review_panel();
            self.invalidate_ruler();
        }
    }

    pub(super) fn review_hit(&mut self, at: usize) {
        if !self.show_review_marks()
            || !self.projection_safe
            || self.draft.is_some()
            || self.link_at(at).is_some()
        {
            return;
        }
        let position = self.review_indices.iter().position(|&index| {
            self.view
                .review_range(&self.parsed, &self.parsed.records[index].binding)
                .is_some_and(|span| span.contains(&at))
        });
        if let Some(position) = position {
            self.review_open = true;
            send(self.review_list, LB_SETCURSEL, position, 0);
            self.review_select(true);
            self.layout();
        }
    }

    pub(super) fn review_record(&self) -> Option<&review::Record> {
        let id = self.active_review.as_ref()?;
        self.parsed.records.iter().find(|r| &r.review.id == id)
    }

    pub(super) fn capture_selection(&self) -> Option<(usize, String, review::Anchor)> {
        let span = self.projection_safe.then(|| selection(self.editor))?;
        if span.is_empty() {
            return None;
        }
        let mut parts = self.view.review_parts(&self.parsed, span);
        let first = parts.next()?;
        let last = parts.next_back().unwrap_or_else(|| first.clone());
        review::capture_span(&self.parsed, first, last)
    }

    pub(super) fn start_review(&mut self, existing: bool, reattach: bool) {
        if self.active.is_none() {
            return;
        }
        let original = if existing {
            let Some(record) = self.review_record() else {
                return;
            };
            Some(record.review.clone())
        } else {
            None
        };
        let capture = if !existing || reattach {
            let Some(capture) = self.capture_selection() else {
                self.status(Some(tr(L::SelectForReview)));
                return;
            };
            Some(capture)
        } else {
            None
        };
        let text = original.as_ref().map_or("", |r| r.text.as_str());
        self.review_open = true;
        send(self.editor, EM_SETREADONLY, 1, 0);
        self.draft = Some(Draft {
            existing: original.as_ref().map(|r| r.id.clone()),
            capture,
            format: None,
        });
        set_text(self.draft_box, &pane_md::normalize_newlines(text, "\r\n"));
        self.layout();
        unsafe {
            let _ = SetFocus(Some(self.draft_box));
        }
    }

    pub(super) fn save_review(&mut self) {
        let Some(index) = self.active else { return };
        let text = pane_md::normalize_newlines(get_text(self.draft_box).trim(), "\n");
        if let Some((region, local)) = self.draft.as_ref().and_then(|draft| draft.format.clone()) {
            let source = &self.docs[index].text;
            match self.parsed.change_link(source, region, local, &text) {
                Ok(patch) => {
                    self.draft = None;
                    self.patch(patch.range, &patch.text, patch.caret.start);
                    unsafe {
                        let _ = SetFocus(Some(self.editor));
                    }
                }
                Err(error) => self.status(Some(crate::i18n::format_error(error))),
            }
            return;
        }
        if text.trim().is_empty() {
            self.status(Some(tr(L::EnterReview)));
            return;
        }
        let Some(draft) = self.draft.take() else {
            return;
        };
        let existing = draft
            .existing
            .as_ref()
            .and_then(|id| self.parsed.records.iter().find(|r| &r.review.id == id));
        let source = &self.docs[index].text;
        let patch = match (existing, draft.capture) {
            (Some(record), Some((after, quote, anchor))) => {
                let mut updated = record.review.clone();
                updated.quote = quote;
                updated.anchor = anchor;
                updated.text = text;
                updated.updated_at = Some(super::cards::now());
                let (range, replacement) = review::relocate(source, record, after, &updated);
                (range, replacement, updated.id)
            }
            (Some(record), None) => {
                if record.review.text == text {
                    self.command(REVIEW_CANCEL);
                    return;
                }
                let mut updated = record.review.clone();
                updated.text = text;
                updated.updated_at = Some(super::cards::now());
                (record.source.clone(), updated.encode(), updated.id)
            }
            (None, Some((after, quote, anchor))) => {
                let review = Review {
                    v: 1,
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    quote,
                    anchor,
                    text,
                    status: Status::Open,
                    updated_at: Some(super::cards::now()),
                };
                (
                    after..after,
                    review::insertion(source, after, &review),
                    review.id,
                )
            }
            (None, None) => return,
        };
        self.active_review = Some(patch.2);
        self.docs[index].show_reviews = true;
        let position = self.current_position();
        self.patch(patch.0, &patch.1, position);
        self.review_select(true);
        self.status(Some(tr(L::ReviewChanged)));
    }

    pub(super) fn change_review(&mut self, command: usize) {
        let Some(record) = self.review_record() else {
            return;
        };
        let range = record.source.clone();
        let mut updated = record.review.clone();
        let position = self.current_position();
        if command == DELETE_REVIEW {
            let answer = unsafe {
                MessageBoxW(
                    Some(self.hwnd),
                    PCWSTR(wide(tr(L::ConfirmDelete)).as_ptr()),
                    w!("pane-md"),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            };
            if answer != IDYES {
                return;
            }
            self.patch(range, "", position);
        } else {
            updated.status = if updated.status == Status::Open {
                Status::Resolved
            } else {
                Status::Open
            };
            updated.updated_at = Some(super::cards::now());
            self.patch(range, &updated.encode(), position);
        }
    }

    pub(super) fn review_heading(&self) {
        if let Some(record) = self.review_record() {
            if let Some(region) = self
                .parsed
                .regions
                .iter()
                .find(|r| r.level > 0 && r.heading == record.review.anchor.heading)
            {
                self.jump_source(region.source.start);
            } else {
                self.status(Some(tr(L::MissingHeading)));
            }
        }
    }

    pub(super) fn format(&mut self, command: usize) {
        if self.mode() != 2 || !self.projection_safe {
            return;
        }
        let selected = selection(self.editor);
        let Some((region, local)) = self.view.region_range(selected) else {
            return;
        };
        if self.view.diagram_regions.contains(&region) {
            self.status(Some(tr(L::MermaidSourceHint)));
            return;
        }
        if command == LINK {
            match self.parsed.link_edit(region, local.clone()) {
                Ok(edit) => {
                    let target = edit.target().map(str::to_owned);
                    self.link_draft(region, local, target.as_deref());
                }
                Err(error) => self.status(Some(crate::i18n::format_error(error))),
            }
            return;
        }
        let region = &self.parsed.regions[region];
        let i = self.active.expect("editor document");
        let source = &self.docs[i].text;
        use pane_md::markdown::{BlockFormat, InlineFormat};
        let block = match command {
            HEADING => Some(BlockFormat::Heading(2)),
            HEADING_FIRST..=HEADING_LAST => {
                Some(BlockFormat::Heading((1 + command - HEADING_FIRST) as u8))
            }
            BULLET => Some(BlockFormat::Bullet),
            NUMBERED_LIST => Some(BlockFormat::Numbered),
            MAKE_TASK => Some(BlockFormat::Task),
            _ => None,
        };
        if let Some(block) = block {
            match region.block_format(source, block) {
                Ok(patch) => self.patch(patch.range, &patch.text, patch.caret.start),
                Err(error) => self.status(Some(crate::i18n::format_error(error))),
            }
            return;
        }
        let style = match command {
            BOLD => InlineFormat::Bold,
            ITALIC => InlineFormat::Italic,
            STRIKE => InlineFormat::Strike,
            _ => InlineFormat::Code,
        };
        match region.inline_format(source, local, style) {
            Ok(patch) => self.patch(patch.range, &patch.text, patch.caret.start),
            Err(error) => self.status(Some(crate::i18n::format_error(error))),
        }
    }

    fn link_draft(&mut self, region: usize, local: Range<usize>, url: Option<&str>) {
        self.draft = Some(Draft {
            existing: None,
            capture: None,
            format: Some((region, local)),
        });
        self.review_open = true;
        send(self.editor, EM_SETREADONLY, 1, 0);
        set_text(self.draft_box, url.unwrap_or(""));
        self.layout();
        unsafe {
            let _ = SetFocus(Some(self.draft_box));
        }
        send(self.draft_box, EM_SETSEL, 0, -1);
        self.status(Some(tr(if url.is_some() {
            L::LinkEditHint
        } else {
            L::LinkApplyHint
        })));
    }

    pub(super) fn table_edit(&mut self, command: usize) {
        if self.mode() != 2 || !self.projection_safe {
            return;
        }
        let Some((region, _)) = self.view.region_range(selection(self.editor)) else {
            return;
        };
        let edit = match command {
            ROW_ADD => TableEdit::AddRow,
            ROW_DELETE => TableEdit::DeleteRow,
            COL_ADD => TableEdit::AddColumn,
            _ => TableEdit::DeleteColumn,
        };
        self.apply_table_edit(region, edit);
    }

    pub(super) fn move_table_cell(&mut self, backwards: bool) -> bool {
        if self.mode() != 2 || self.draft.is_some() {
            return false;
        }
        self.flush_edit();
        if !self.projection_safe {
            return false;
        }
        let at = selection(self.editor).start;
        let Some((region, _)) = self.view.region_range(at..at) else {
            return false;
        };
        if self.parsed.regions[region].cell.is_none() {
            return false;
        }
        if let Some(target) = self.parsed.table_neighbor(region, backwards) {
            self.focus_table_cell(target);
        } else if backwards {
            self.command(211);
        } else {
            self.apply_table_edit(region, TableEdit::AddRow);
        }
        true
    }

    fn apply_table_edit(&mut self, region: usize, edit: TableEdit) {
        let Some(i) = self.active else { return };
        match self.parsed.edit_table(&self.docs[i].text, region, edit) {
            Ok(patch) => {
                self.patch(patch.range, &patch.text, patch.caret.start);
                if let Some(target) = patch.focus_region {
                    self.focus_table_cell(target);
                }
            }
            Err(error) => self.status(Some(crate::i18n::format_error(error))),
        }
    }

    fn focus_table_cell(&self, region: usize) {
        if let Some(cell) = self.view.region.get(region) {
            select(self.editor, cell.start..cell.start, true);
            unsafe {
                let _ = SetFocus(Some(self.editor));
            }
        }
    }
}
