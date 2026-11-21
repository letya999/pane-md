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
        let hint = wide(if headings {
            "Найти заголовок"
        } else {
            "Найти в документе"
        });
        send(self.find_box, EM_SETCUEBANNER, 1, hint.as_ptr() as isize);
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

    pub(super) fn find(&self, backwards: bool) {
        let query = get_text(self.find_box).to_lowercase();
        if self.find_headings {
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
                self.status(Some("Заголовков не найдено"));
            }
            return;
        }
        if query.is_empty() {
            return;
        }
        let mut lower = String::new();
        let mut offsets = Vec::new();
        let mut offset = 0;
        for unit in char::decode_utf16(self.view.text.iter().copied()) {
            let ch = unit.unwrap_or(char::REPLACEMENT_CHARACTER);
            for part in ch.to_lowercase() {
                offsets.resize(offsets.len() + part.len_utf8(), offset);
                lower.push(part);
            }
            offset += ch.len_utf16();
        }
        offsets.push(offset);
        let matches = || {
            lower
                .match_indices(&query)
                .map(|(at, s)| offsets[at]..offsets[at + s.len()])
        };
        let current = selection(self.editor);
        let found = if backwards {
            matches()
                .take_while(|span| span.end <= current.start)
                .last()
                .or_else(|| matches().last())
        } else {
            matches()
                .find(|span| span.start >= current.end)
                .or_else(|| matches().next())
        };
        if let Some(span) = found {
            select(self.editor, span, true);
            self.status(None);
        } else {
            self.status(Some("Совпадений нет"));
        }
    }

    pub(super) fn follow_at(&mut self, at: usize) {
        if !self.projection_safe || self.mode() % 2 == 1 {
            return;
        }
        let Some(raw) = self.view.link(at).map(str::to_owned) else {
            return;
        };
        if raw.starts_with("image:") {
            self.status(Some("Удалённые изображения не загружаются автоматически."));
        } else if let Err(error) = self.follow(&raw) {
            self.status(Some(&error.to_string()));
        }
    }

    fn follow(&mut self, raw: &str) -> Result<()> {
        if raw.contains(['\r', '\n', '\0']) {
            return Err("Некорректная ссылка".into());
        }
        if let Some((scheme, _)) = raw.split_once(':')
            && scheme
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
        {
            let lower = raw.to_ascii_lowercase();
            if !(lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("mailto:"))
            {
                return Err("Этот тип ссылки не открывается".into());
            }
            let code = unsafe {
                ShellExecuteW(
                    Some(self.hwnd),
                    w!("open"),
                    PCWSTR(wide(raw).as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                )
            };
            if code.0 as isize <= 32 {
                return Err("Windows не удалось открыть ссылку".into());
            }
            return Ok(());
        }
        let (file, fragment) = raw.split_once('#').unwrap_or((raw, ""));
        let fragment = decode_url(fragment)?;
        let index = self.active.ok_or("Сначала откройте документ")?;
        let from = self.docs[index].path.clone();
        let position = self.capture_viewport();
        if !file.is_empty() {
            let local = PathBuf::from(decode_url(file)?);
            if local.has_root() || !markdown_path(&local) {
                return Err("Поддерживаются относительные ссылки на Markdown-файлы".into());
            }
            self.open(
                from.parent()
                    .ok_or("Не удалось определить каталог документа")?
                    .join(local),
                0,
            )?;
        }
        if !fragment.is_empty() && !self.jump_anchor(&fragment) {
            self.history.push((from, position));
            return Err(format!("Якорь #{fragment} не найден").into());
        }
        self.history.push((from, position));
        Ok(())
    }

    pub(super) fn jump_anchor(&self, fragment: &str) -> bool {
        if let Some(name) = fragment.strip_prefix("fnref-") {
            let note = format!("#fn-{name}");
            if let Some((span, _)) = self.view.links.iter().find(|(_, url)| *url == note) {
                select(self.editor, span.clone(), true);
                return true;
            }
        }
        if let Some(region) = self.parsed.regions.iter().find(|r| r.slug == fragment) {
            self.jump_source(region.source.start);
            true
        } else {
            false
        }
    }

    pub(super) fn back(&mut self) {
        if let Some((path, position)) = self.history.pop() {
            if self.open(path, 0).is_ok() {
                self.restore_viewport(&position);
            }
        }
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
        let mut items = vec![(206, "Копировать"), (207, "Выделить всё")];
        if self.mode() >= 2 {
            items.extend([(208, "Вырезать"), (209, "Вставить"), (UNDO, "Отменить")]);
        }
        if self.mode() % 2 == 0 {
            items.extend([
                (ADD_REVIEW, "Добавить комментарий"),
                (OPEN_LINK, "Открыть ссылку"),
                (SHOW_SOURCE, "Показать в исходнике"),
            ]);
        }
        match self.popup_choice(self.editor, &items, location) {
            Some(206) => {
                send(self.editor, WM_COPY, 0, 0);
            }
            Some(207) => select(self.editor, 0..self.view.text.len(), false),
            Some(208) => {
                send(self.editor, WM_CUT, 0, 0);
            }
            Some(209) => {
                send(self.editor, WM_PASTE, 0, 0);
            }
            Some(command) => self.command(command),
            _ => {}
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
                let _ = AppendMenuW(menu, MF_STRING, *id, PCWSTR(wide(text).as_ptr()));
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
    let mut out = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex = |byte: u8| char::from(byte).to_digit(16).map(|x| x as u8);
            let hi = bytes
                .next()
                .and_then(hex)
                .ok_or("Некорректное экранирование ссылки")?;
            let lo = bytes
                .next()
                .and_then(hex)
                .ok_or("Некорректное экранирование ссылки")?;
            out.push(hi * 16 + lo);
        } else {
            out.push(byte);
        }
    }
    if out.contains(&0) {
        return Err("Некорректная ссылка".into());
    }
    Ok(String::from_utf8(out)?)
}

pub(super) const FORMAT_ITEMS: &[(usize, &str)] = &[
    (BOLD, "Полужирный\tCtrl+B"),
    (ITALIC, "Курсив\tCtrl+I"),
    (STRIKE, "Зачёркнутый"),
    (CODE, "Код"),
    (LINK, "Ссылка…"),
    (HEADING, "Заголовок H2"),
    (BULLET, "Пункт списка"),
];

pub(super) const TABLE_ITEMS: &[(usize, &str)] = &[
    (ROW_ADD, "Добавить строку ниже"),
    (ROW_DELETE, "Удалить строку"),
    (COL_ADD, "Добавить столбец справа"),
    (COL_DELETE, "Удалить столбец"),
];

pub(super) fn menu_bar() -> Result<HMENU> {
    let sections: &[(&str, &[(usize, &str)])] = &[
        (
            "&Файл",
            &[
                (OPEN, "Открыть файл…\tCtrl+O"),
                (FOLDER, "Открыть папку…\tCtrl+Shift+O"),
                (SAVE, "Сохранить\tCtrl+S"),
                (SAVE_AS, "Сохранить как…\tCtrl+Shift+S"),
                (CLOSE, "Закрыть вкладку\tCtrl+W"),
            ],
        ),
        (
            "&Правка",
            &[
                (UNDO, "Отменить\tCtrl+Z"),
                (REDO, "Повторить\tCtrl+Y"),
                (FIND, "Найти…\tCtrl+F"),
                (FIND_NEXT, "Следующее совпадение\tF3"),
                (FIND_PREV, "Предыдущее совпадение\tShift+F3"),
            ],
        ),
        (
            "&Вид",
            &[
                (TREE, "Панель папки"),
                (REVIEWS, "Панель комментариев"),
                (HEADINGS, "Найти заголовок…"),
                (SHOW_SOURCE, "Показать в исходнике"),
                (BACK, "Назад по ссылке\tAlt+←"),
                (ZOOM_IN, "Увеличить\tCtrl++"),
                (ZOOM_OUT, "Уменьшить\tCtrl+-"),
                (ZOOM_RESET, "Масштаб 100%\tCtrl+0"),
                (THEME, "Переключить системную / светлую тему"),
            ],
        ),
        ("&Формат", FORMAT_ITEMS),
        ("&Таблица", TABLE_ITEMS),
        (
            "&Ревью",
            &[
                (ADD_REVIEW, "Добавить к выделению\tCtrl+Alt+M"),
                (EDIT_REVIEW, "Править выбранный комментарий"),
                (RESOLVE_REVIEW, "Решить / открыть снова"),
                (REATTACH, "Перепривязать к выделению"),
                (205, "К сохранённому заголовку"),
                (DELETE_REVIEW, "Удалить комментарий"),
            ],
        ),
    ];
    unsafe {
        let menu = CreateMenu()?;
        for (title, items) in sections {
            let popup = CreatePopupMenu()?;
            for (id, name) in *items {
                AppendMenuW(popup, MF_STRING, *id, PCWSTR(wide(name).as_ptr()))?;
            }
            AppendMenuW(
                menu,
                MF_POPUP,
                popup.0 as usize,
                PCWSTR(wide(title).as_ptr()),
            )?;
        }
        Ok(menu)
    }
}
