use super::*;

pub(super) unsafe fn pointer(frame: HWND, message: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    if !matches!(
        message,
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MOUSEMOVE | WM_CAPTURECHANGED
    ) {
        return None;
    }
    unsafe {
        let state = GetWindowLongPtrW(frame, GWLP_USERDATA) as *const RefCell<App>;
        let mut app = state.as_ref()?.try_borrow_mut().ok()?;
        if message == WM_CAPTURECHANGED {
            app.task_down = None;
            return None;
        }
        if app.mode() != 2 || app.draft.is_some() || !app.projection_safe {
            let pressed = app.task_down.take().is_some();
            let editor = app.editor;
            drop(app);
            if pressed && GetCapture() == editor {
                let _ = ReleaseCapture();
            }
            return None;
        }
        let point = POINT {
            x: l.0 as i16 as i32,
            y: (l.0 >> 16) as i16 as i32,
        };
        let marker = || app.active.zip(app.task_hit(point));
        match message {
            WM_LBUTTONDOWN => {
                app.task_down = marker();
                if app.task_down.is_some() {
                    let editor = app.editor;
                    drop(app);
                    let _ = SetFocus(Some(editor));
                    SetCapture(editor);
                    return Some(LRESULT(0));
                }
            }
            WM_LBUTTONUP => {
                let hit = marker();
                if let Some(pressed) = app.task_down.take() {
                    let editor = app.editor;
                    drop(app);
                    if GetCapture() == editor {
                        let _ = ReleaseCapture();
                    }
                    if let Some((document, at)) = hit.filter(|target| *target == pressed) {
                        let _ = PostMessageW(
                            Some(frame),
                            WM_TASK,
                            WPARAM(at),
                            LPARAM(document as isize),
                        );
                    }
                    return Some(LRESULT(0));
                }
            }
            WM_MOUSEMOVE if w.0 & 1 == 0 && marker().is_some() => {
                if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cursor));
                }
                return Some(LRESULT(0));
            }
            _ => {}
        }
        None
    }
}

impl App {
    pub(super) fn task_hit(&self, point: POINT) -> Option<usize> {
        let at = hit(self.editor, point)?;
        [at, at.saturating_sub(1)].into_iter().find(|&start| {
            let Some((i, local)) = self.view.region_range(start..start + 1) else {
                return false;
            };
            if local.start != 0
                || self.parsed.regions[i]
                    .list
                    .as_ref()
                    .and_then(|item| item.task)
                    .is_none()
            {
                return false;
            }
            let a = char_point(self.editor, start);
            let b = char_point(self.editor, start + 1);
            point.x >= a.x && point.x < b.x && point.y >= a.y && point.y < a.y + self.dip(21)
        })
    }

    pub(super) fn toggle_task(&mut self, at: usize) {
        if self.mode() < 2 || self.draft.is_some() || !self.projection_safe {
            return;
        }
        self.flush_edit();
        self.parse_source_edits();
        let Some(i) = self.active else { return };
        let source = &self.docs[i].text;
        let caret = self.view.source_at(&self.parsed, at);
        let item = if self.mode() == 3 {
            self.parsed
                .regions
                .iter()
                .rev()
                .filter_map(|r| r.list.as_ref())
                .find(|item| (item.source.start..=item.source.end).contains(&caret))
        } else {
            self.view
                .region_range(at..at)
                .and_then(|(i, _)| self.parsed.regions[i].list.as_ref())
        };
        if let Some((span, replacement)) = item.and_then(|item| item.task_patch(source)) {
            self.patch(span, replacement, caret);
        }
    }

    pub(super) fn continue_list(&mut self) -> bool {
        let selected = selection(self.editor);
        let Some(i) = self.active else {
            return false;
        };
        let Some((region, local)) = self.view.region_range(selected.clone()) else {
            return false;
        };
        if !selected.is_empty() || !self.projection_safe {
            return false;
        }
        let region = &self.parsed.regions[region];
        let source = &self.docs[i].text;
        let caret = if local.start == region.text.encode_utf16().count() {
            region.source.end
        } else {
            self.view.source_at(&self.parsed, selected.start)
        };
        if let Some((range, replacement, caret)) = region.continue_list(source, caret) {
            self.patch(range, &replacement, caret);
            true
        } else {
            false
        }
    }
}
