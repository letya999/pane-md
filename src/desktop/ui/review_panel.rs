use super::*;

impl App {
    pub(super) fn refresh_reviews(&mut self) {
        send(self.review_list, WM_SETREDRAW, 0, 0);
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
                Binding::Attached { line, .. } => format!("Строка {line}"),
                Binding::Missing => "Без привязки".to_owned(),
                Binding::Ambiguous => "Неоднозначно".to_owned(),
            };
            let state = if record.review.status == Status::Resolved {
                " · решён"
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
        if let Some(i) = selected.or_else(|| (!self.review_indices.is_empty()).then_some(0)) {
            send(self.review_list, LB_SETCURSEL, i, 0);
        }
        send(self.review_list, WM_SETREDRAW, 1, 0);
        unsafe {
            let _ = InvalidateRect(Some(self.review_list), None, true);
        }
        set_text(
            self.review_label,
            &format!("Комментарии  ({})", self.review_indices.len()),
        );
        self.review_select(false);
    }

    pub(super) fn review_select(&mut self, jump: bool) {
        let native = send(self.review_list, LB_GETCURSEL, 0, 0);
        let record = usize::try_from(native)
            .ok()
            .and_then(|n| self.review_indices.get(n))
            .and_then(|&i| self.parsed.records.get(i));
        self.active_review = record.map(|r| r.review.id.clone());
        let preview = record.map_or(String::from("Выделите текст в документе и нажмите «Добавить»."),
            |record| {
                let state = match record.binding {
                    Binding::Missing => "Привязка потерялась. Выделите текст и выберите «Перепривязать к выделению».",
                    Binding::Ambiguous => "Найдено несколько совпадений. Выберите нужное выделение.",
                    Binding::Attached { .. } if record.review.status == Status::Resolved => "Решён",
                    _ => "Открыт",
                };
                let heading = if record.review.anchor.heading.is_empty() { "" } else { "\r\n" };
                format!("«{}»{heading}{}\r\n{state}\r\n\r\n{}", record.review.quote,
                    record.review.anchor.heading, notepad_md::normalize_newlines(&record.review.text, "\r\n"))
            });
        set_text(self.review_text, &preview);
        set_text(
            self.review_actions[2],
            if record.is_some_and(|r| r.review.status == Status::Resolved) {
                "Открыть"
            } else {
                "Решить"
            },
        );
        for (i, button) in self.review_actions.iter().enumerate() {
            unsafe {
                let _ = EnableWindow(
                    *button,
                    if i == 0 {
                        self.mode() % 2 == 0
                    } else {
                        record.is_some()
                    },
                );
            }
        }
        if jump && let Some(record) = record {
            match &record.binding {
                Binding::Attached {
                    region, selection, ..
                } if self.mode() % 2 == 0 => {
                    select(
                        self.editor,
                        self.view.native_at(*region, selection.start)
                            ..self.view.native_at(*region, selection.end),
                        true,
                    );
                    self.status(None);
                }
                Binding::Attached { bytes, .. } => {
                    let start = self.view.visible_at(&self.parsed, bytes.start);
                    let end = self.view.visible_at(&self.parsed, bytes.end);
                    select(self.editor, start..end, true);
                }
                _ => {}
            }
        }
    }

    pub(super) fn align_reviews(&mut self) {
        if !self.review_open
            || self.draft.is_some()
            || self.mode() % 2 == 1
            || self.review_indices.len() < 2
            || !self.projection_safe
        {
            return;
        }
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.editor, &mut rect);
        }
        let native = send(self.review_list, LB_GETCURSEL, 0, 0);
        if native >= 0
            && let Some(&i) = self.review_indices.get(native as usize)
        {
            match self.parsed.records[i].binding {
                Binding::Missing | Binding::Ambiguous => return,
                Binding::Attached { region, .. } => {
                    let point = char_point(self.editor, self.view.region[region].start);
                    if (0..rect.bottom).contains(&point.y) {
                        return;
                    }
                }
            }
        }
        for (index, &i) in self.review_indices.iter().enumerate() {
            if let Binding::Attached { region, .. } = self.parsed.records[i].binding {
                let y = char_point(self.editor, self.view.region[region].start).y;
                if (0..rect.bottom).contains(&y) {
                    send(self.review_list, LB_SETCURSEL, index, 0);
                    self.review_select(false);
                    break;
                }
            }
        }
    }

    pub(super) fn place_review_card(&self) {
        let r = self.review_bounds;
        let preferred = self
            .review_record()
            .and_then(|record| {
                if self.mode() % 2 == 1 {
                    return None;
                }
                let Binding::Attached {
                    region, selection, ..
                } = &record.binding
                else {
                    return None;
                };
                let at = self.view.native_at(*region, selection.start);
                Some(self.document_top + char_point(self.editor, at).y)
            })
            .unwrap_or(r.top);
        let top = preferred.clamp(r.top, (r.bottom - self.dip(150)).max(r.top));
        move_to(
            self.review_text,
            r.left,
            top,
            r.right - r.left,
            r.bottom - top,
            self.review_open && self.active.is_some() && self.draft.is_none(),
        );
    }

    pub(super) fn review_hit(&mut self, at: usize) {
        if self.mode() % 2 == 1
            || !self.projection_safe
            || self.draft.is_some()
            || self.view.link(at).is_some()
        {
            return;
        }
        let position = self.review_indices.iter().position(|&index| {
            let record = &self.parsed.records[index];
            if let Binding::Attached {
                region, selection, ..
            } = &record.binding
            {
                (self.view.native_at(*region, selection.start)
                    ..self.view.native_at(*region, selection.end))
                    .contains(&at)
            } else {
                false
            }
        });
        if let Some(position) = position {
            self.review_open = true;
            send(self.review_list, LB_SETCURSEL, position, 0);
            self.review_select(true);
            self.layout();
        }
    }

    fn review_record(&self) -> Option<&review::Record> {
        let id = self.active_review.as_ref()?;
        self.parsed.records.iter().find(|r| &r.review.id == id)
    }

    pub(super) fn capture_selection(&self) -> Option<(usize, String, review::Anchor)> {
        if self.mode() % 2 == 1 || !self.projection_safe {
            return None;
        }
        let (index, local) = self.view.region_range(selection(self.editor))?;
        review::capture(&self.parsed, index, local)
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
                self.status(Some(
                    "Выделите слово, фразу или текст одной ячейки в WYSIWYG.",
                ));
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
        set_text(
            self.draft_box,
            &notepad_md::normalize_newlines(text, "\r\n"),
        );
        self.layout();
        unsafe {
            let _ = SetFocus(Some(self.draft_box));
        }
    }

    pub(super) fn save_review(&mut self) {
        let Some(index) = self.active else { return };
        let text = notepad_md::normalize_newlines(get_text(self.draft_box).trim(), "\n");
        if text.trim().is_empty() {
            self.status(Some("Введите текст."));
            return;
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.format.is_some())
            && text.contains(['\r', '\n', '<', '>'])
        {
            self.status(Some("Введите адрес ссылки одной строкой."));
            return;
        }
        let Some(draft) = self.draft.take() else {
            return;
        };
        if let Some((range, label)) = draft.format {
            let url = text
                .replace(' ', "%20")
                .replace('(', "%28")
                .replace(')', "%29");
            let replacement =
                label.map_or_else(|| url.clone(), |label| format!("[{label}]({url})"));
            let at = range.start;
            self.patch(range, &replacement, at);
            unsafe {
                let _ = SetFocus(Some(self.editor));
            }
            return;
        }
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
                let old = record.source.clone();
                let new = review::insertion(source, after, &updated);
                if after <= old.start {
                    (
                        after..old.end,
                        format!("{new}{}", &source[after..old.start]),
                        updated.id,
                    )
                } else if after >= old.end {
                    (
                        old.start..after,
                        format!("{}{new}", &source[old.end..after]),
                        updated.id,
                    )
                } else {
                    (old, updated.encode(), updated.id)
                }
            }
            (Some(record), None) => {
                let mut updated = record.review.clone();
                updated.text = text;
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
        let position = self.current_position(false);
        self.patch(patch.0, &patch.1, position);
        self.review_select(true);
        self.status(Some("Комментарий в буфере. Ctrl+S сохраняет файл."));
    }

    pub(super) fn change_review(&mut self, command: usize) {
        let Some(record) = self.review_record() else {
            return;
        };
        let range = record.source.clone();
        let mut updated = record.review.clone();
        let position = self.current_position(false);
        if command == DELETE_REVIEW {
            let answer = unsafe {
                MessageBoxW(
                    Some(self.hwnd),
                    w!("Удалить выбранный комментарий?"),
                    w!("notepad-md"),
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
                self.status(Some("Сохранённый заголовок не найден"));
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
        let region = &self.parsed.regions[region];
        let i = self.active.expect("editor document");
        let source = &self.docs[i].text;
        if matches!(command, HEADING | BULLET) {
            let while_editing = !region.source.is_empty()
                && region.kind != "table"
                && region.indent == 0
                && matches!(region.kind, "paragraph" | "heading")
                && !source[region.source.clone()].contains(['\r', '\n']);
            if !while_editing {
                self.status(Some("Выберите однострочный абзац или заголовок."));
                return;
            }
            let raw = &source[region.source.clone()];
            let existing = raw.bytes().take_while(|&b| b == b'#').count();
            let existing = if existing <= 6 && raw.as_bytes().get(existing) == Some(&b' ') {
                existing + 1
            } else {
                0
            };
            let start = region.source.start;
            let marker = if command == HEADING { "## " } else { "- " };
            let marker = if raw.starts_with(marker) { "" } else { marker };
            self.patch(start..start + existing, marker, start + marker.len());
            return;
        }
        if command == LINK {
            let mut offset = 0;
            for run in &region.runs {
                let start = offset;
                offset += run.units.len();
                if local.start < start || local.start >= offset {
                    continue;
                }
                if let Some(url) = &run.link {
                    let label = region.source_selection(start..offset);
                    let range = label.and_then(|label| {
                        if !source[..label.start].ends_with('[') {
                            return None;
                        }
                        let target = label.end + 2;
                        let suffix = source[label.end..].strip_prefix("](")?.strip_prefix(url)?;
                        (suffix.starts_with(')') || suffix.starts_with(char::is_whitespace))
                            .then_some(target..target + url.len())
                    });
                    if let Some(range) = range {
                        let url = url.clone();
                        self.link_draft(range, None, &url);
                    } else {
                        self.status(Some("Адрес этой ссылки можно изменить в MD Source Edit."));
                    }
                    return;
                }
            }
        }
        let Some(span) = region.source_selection(local.clone()) else {
            self.status(Some("Выделите текст для форматирования."));
            return;
        };
        if region.patch(source, local, "").is_none() {
            self.status(Some("Этот диапазон можно форматировать в MD Source Edit."));
            return;
        }
        let raw = &source[span.clone()];
        if command == LINK {
            let raw = raw.to_owned();
            self.link_draft(span, Some(raw), "");
            return;
        }
        let marker = match command {
            BOLD => "**",
            ITALIC => "*",
            STRIKE => "~~",
            _ => "`",
        };
        if raw.trim() != raw || (command == CODE && raw.contains('`')) {
            self.status(Some("Выделите текст без пробелов по краям."));
            return;
        }
        let start = span.start;
        let styled = region.runs.iter().any(|run| {
            let includes = run
                .units
                .iter()
                .any(|unit| unit.as_ref().is_some_and(|unit| unit.start == span.start));
            includes
                && match command {
                    BOLD => run.style.bold,
                    ITALIC => run.style.italic,
                    STRIKE => run.style.strike,
                    _ => run.style.code,
                }
        });
        if styled && source[..start].ends_with(marker) && source[span.end..].starts_with(marker) {
            let bytes = start - marker.len()..span.end + marker.len();
            let raw = raw.to_owned();
            self.patch(bytes.clone(), &raw, bytes.start);
        } else {
            let replacement = format!("{marker}{raw}{marker}");
            self.patch(span, &replacement, start + marker.len());
        }
    }

    fn link_draft(&mut self, range: Range<usize>, label: Option<String>, url: &str) {
        self.draft = Some(Draft {
            existing: None,
            capture: None,
            format: Some((range, label)),
        });
        self.review_open = true;
        send(self.editor, EM_SETREADONLY, 1, 0);
        set_text(self.draft_box, url);
        self.layout();
        unsafe {
            let _ = SetFocus(Some(self.draft_box));
        }
        self.status(Some("Введите адрес ссылки и нажмите «Применить»."));
    }

    pub(super) fn table_edit(&mut self, command: usize) {
        let Some(i) = self.active else { return };
        if self.mode() != 2 || !self.projection_safe {
            return;
        }
        let Some((region, _)) = self.view.region_range(selection(self.editor)) else {
            return;
        };
        let r = &self.parsed.regions[region];
        let Some((row, cell)) = r.row.zip(r.cell) else {
            self.status(Some("Выберите ячейку таблицы."));
            return;
        };
        let Some(Layout::Table { rows, alignment, source }) = self.parsed.layout.iter().find(|block| {
            matches!(block, Layout::Table { rows, .. } if rows.get(row).and_then(|r| r.get(cell)) == Some(&region))
        }) else { return };
        let edit = match command {
            ROW_ADD => notepad_md::markdown::TableEdit::AddRow,
            ROW_DELETE => notepad_md::markdown::TableEdit::DeleteRow,
            COL_ADD => notepad_md::markdown::TableEdit::AddColumn,
            _ => notepad_md::markdown::TableEdit::DeleteColumn,
        };
        let range = source.clone();
        let replacement = notepad_md::markdown::table_patch(
            &self.docs[i].text,
            range.clone(),
            rows.len(),
            alignment.len(),
            row,
            cell,
            edit,
        );
        if let Some(replacement) = replacement {
            self.patch(range.clone(), &replacement, range.start);
        } else {
            self.status(Some(
                "Эту структуру таблицы можно изменить в MD Source Edit.",
            ));
        }
    }
}
