use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn review_date(timestamp: Option<u64>) -> String {
    let local = timestamp.and_then(|ms| unsafe {
        let ticks = ms
            .checked_mul(10_000)?
            .checked_add(116_444_736_000_000_000)?;
        let file = FILETIME {
            dwLowDateTime: ticks as u32,
            dwHighDateTime: (ticks >> 32) as u32,
        };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        FileTimeToSystemTime(&file, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local).ok()?;
        Some(local)
    });
    local.map_or_else(
        || tr(L::UnknownDate).to_owned(),
        |date| {
            format!(
                "{} {:02}:{:02}",
                if i18n::english() {
                    format!("{:04}-{:02}-{:02}", date.wYear, date.wMonth, date.wDay)
                } else {
                    format!("{:02}.{:02}.{:04}", date.wDay, date.wMonth, date.wYear)
                },
                date.wHour,
                date.wMinute
            )
        },
    )
}

pub(super) fn excerpt(text: &str, max: usize) -> String {
    let mut result = text
        .split_whitespace()
        .flat_map(|word| word.chars().chain([' ']))
        .take(max + 1)
        .collect::<String>();
    if result.trim_end().chars().count() > max {
        result.truncate(result.char_indices().nth(max).unwrap().0);
        result.truncate(result.trim_end().len());
        result.push('…');
    } else {
        result.truncate(result.trim_end().len());
    }
    result
}

impl App {
    pub(super) fn show_review_marks(&self) -> bool {
        self.active.is_some_and(|i| self.docs[i].show_reviews)
    }

    pub(super) fn reviews_visible(&self) -> bool {
        self.active.is_some()
            && (self.draft.is_some() || self.show_review_marks() && self.review_open)
    }

    pub(super) fn layout_review_panel(&mut self) {
        let r = self.review_bounds;
        let dpi = self.dpi;
        let d = |n| super::layout::scale(n, dpi);
        let x = r.left + d(12);
        let width = r.right - x - d(12);
        let visible = self.reviews_visible();
        let draft = visible && self.draft.is_some();
        let inline = self.inline_draft();
        let expanded = visible && !draft && self.review_expanded && self.review_record().is_some();
        let list = visible && !draft && self.review_all;
        let quiet = visible && !list && !draft && !expanded && self.review_indices.is_empty();
        let header = match self.draft.as_ref() {
            Some(draft) if draft.format.is_some() => tr(L::LinkAddress),
            Some(draft) if draft.existing.is_none() => tr(L::NewReview),
            _ => tr(L::Comments),
        };
        set_text(self.review_label, header);
        move_to(
            self.review_label,
            x,
            r.top + d(11),
            width - d(60),
            d(22),
            visible,
        );
        move_to(
            self.review_actions[0],
            r.right - d(68),
            r.top + d(7),
            d(29),
            d(29),
            visible && !draft,
        );
        set_text(self.review_all_button, tr(L::Collapse));
        move_to(
            self.review_all_button,
            x,
            r.bottom - d(31),
            d(70),
            d(26),
            list,
        );
        move_to(
            self.review_list,
            x,
            r.top + d(46),
            width,
            r.bottom - r.top - d(82),
            list,
        );
        move_to(
            self.review_text,
            x,
            r.top + d(47),
            width,
            r.bottom - r.top - if quiet { d(53) } else { d(89) },
            expanded || quiet,
        );
        let action_width = (width - d(9)) / 4;
        for (i, button) in [
            self.review_collapse,
            self.review_actions[1],
            self.review_actions[2],
            self.review_actions[3],
        ]
        .into_iter()
        .enumerate()
        {
            move_to(
                button,
                x + i as i32 * (action_width + d(3)),
                r.bottom - d(33),
                action_width,
                d(27),
                expanded,
            );
        }
        if draft && !inline {
            let area = RECT {
                left: x,
                right: x + width,
                top: r.top + d(48),
                bottom: r.bottom - d(6),
            };
            self.place_draft(area, false);
        } else if !draft {
            self.hide_draft();
        }
        self.place_review_card();
    }

    fn inline_draft(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| draft.format.is_none() && draft.existing.is_some())
    }

    fn hide_draft(&self) {
        for hwnd in [self.draft_quote, self.draft_box]
            .into_iter()
            .chain(self.draft_actions)
        {
            move_to(hwnd, 0, 0, 0, 0, false);
        }
    }

    fn place_draft(&mut self, rect: RECT, inline: bool) {
        let d = |n| super::layout::scale(n, self.dpi);
        let width = rect.right - rect.left;
        let x = rect.left + d(10);
        let body_top = rect.top + d(if inline { 34 } else { 6 });
        if inline {
            if let Some(record) = self.review_record() {
                set_text(
                    self.draft_quote,
                    &format!("«{}»", excerpt(&record.review.quote, 80)),
                );
            }
            move_to(
                self.draft_quote,
                x,
                rect.top + d(10),
                width - d(20),
                d(22),
                true,
            );
        } else {
            move_to(self.draft_quote, 0, 0, 0, 0, false);
        }
        move_to(
            self.draft_box,
            x,
            body_top,
            width - d(20),
            rect.bottom - body_top - d(49),
            true,
        );
        let format = self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.format.is_some());
        set_text(
            self.draft_actions[0],
            tr(if format { L::Apply } else { L::SaveComment }),
        );
        move_to(
            self.draft_actions[1],
            rect.right - d(183),
            rect.bottom - d(37),
            d(77),
            d(27),
            true,
        );
        move_to(
            self.draft_actions[0],
            rect.right - d(101),
            rect.bottom - d(37),
            d(91),
            d(27),
            true,
        );
    }

    pub(super) fn place_review_card(&mut self) {
        let r = self.review_bounds;
        let d = |n| super::layout::scale(n, self.dpi);
        let mut next = r.top + d(51);
        let mut draft_rect = None;
        let mut rows = Vec::new();
        let inline = self.inline_draft();
        if self.reviews_visible()
            && (inline || self.draft.is_none() && !self.review_expanded && !self.review_all)
        {
            let active = self.active_review.as_deref();
            let entries = self.review_indices.iter().copied();
            for index in entries {
                let record = &self.parsed.records[index];
                let editing = inline && active == Some(record.review.id.as_str());
                let anchor = self
                    .view
                    .review_range(&self.parsed, &record.binding)
                    .map(|span| self.document_top + char_point(self.editor, span.start).y);
                if !editing && anchor.is_some_and(|y| y < self.document_top || y >= r.bottom) {
                    continue;
                }
                let height = d(if editing { 156 } else { 82 });
                if next + height > r.bottom - d(8) || rows.len() == CARD_SLOTS {
                    continue;
                }
                let rect = RECT {
                    left: r.left + d(12),
                    right: r.right - d(12),
                    top: next,
                    bottom: next + height,
                };
                next += height + d(9);
                if editing {
                    draft_rect = Some(rect);
                } else {
                    rows.push((index, rect));
                }
            }
            if inline && draft_rect.is_none() {
                // A draft must remain reachable when its anchor has scrolled out of view.
                rows.clear();
                draft_rect = Some(RECT {
                    left: r.left + d(12),
                    right: r.right - d(12),
                    top: r.top + d(51),
                    bottom: (r.top + d(207)).min(r.bottom),
                });
            }
        }

        let old_draft = self.draft_frame;
        self.draft_frame = draft_rect;
        if let Some(rect) = draft_rect {
            self.place_draft(rect, true);
        }
        if old_draft != draft_rect {
            for rect in old_draft.into_iter().chain(draft_rect) {
                unsafe {
                    let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
                }
            }
        }

        let mut shown = 0;
        for (slot, (index, rect)) in rows.iter().copied().enumerate() {
            if slot == self.review_cards.len() {
                let instance =
                    HINSTANCE(unsafe { GetWindowLongPtrW(self.hwnd, GWLP_HINSTANCE) } as *mut _);
                let Ok(card) = super::cells::ReviewCard::new(self.hwnd, instance, slot, self.font)
                else {
                    break;
                };
                self.review_cards.push(card);
            }
            let card = &self.review_cards[slot];
            if self.card_rows.get(slot) != Some(&index) {
                let review = &self.parsed.records[index].review;
                set_text(
                    card.body,
                    &format!(
                        "{} · {}",
                        excerpt(&review.quote, 80),
                        excerpt(&review.text, 141)
                    ),
                );
            }
            card.place(rect, self.dpi);
            unsafe {
                let _ = EnableWindow(card.body, !inline);
                let _ = EnableWindow(card.menu, !inline);
                let _ = InvalidateRect(Some(card.body), None, false);
            }
            shown += 1;
        }
        for card in self.review_cards.iter().skip(shown) {
            card.hide();
        }
        self.card_rows = rows
            .into_iter()
            .take(shown)
            .map(|(index, _)| index)
            .collect();
    }

    pub(super) fn paint_review_draft(&self, dc: HDC) {
        if let Some(rect) = self.draft_frame {
            super::atoms::rounded(
                dc,
                rect,
                self.dip(12),
                self.palette.canvas,
                self.palette.action,
            );
            super::atoms::fill(
                dc,
                &RECT {
                    right: rect.left + self.dip(3),
                    top: rect.top + self.dip(6),
                    bottom: rect.bottom - self.dip(6),
                    ..rect
                },
                self.palette.action,
            );
        }
    }

    pub(super) fn review_card_menu(&mut self, slot: usize) {
        let Some(&index) = self.card_rows.get(slot) else {
            return;
        };
        let Some(row) = self.review_indices.iter().position(|&i| i == index) else {
            return;
        };
        send(self.review_list, LB_SETCURSEL, row, 0);
        self.review_select(false);
        self.invalidate_ruler();
        if let Some(span) = self
            .review_record()
            .and_then(|record| self.view.review_range(&self.parsed, &record.binding))
        {
            select(self.editor, span, true);
        }
        let anchor = self.review_cards[slot].menu;
        self.popup(
            anchor,
            &[
                (EDIT_REVIEW, tr(L::ReviewEdit)),
                (REVIEW_DETAILS, tr(L::FullComment)),
                (RESOLVE_REVIEW, tr(L::ToggleResolved)),
                (REATTACH, tr(L::Reattach)),
                (205, tr(L::SavedHeading)),
                (DELETE_REVIEW, tr(L::Delete)),
            ],
            None,
        );
    }
}
