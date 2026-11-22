use super::atoms::{fill, rounded, text};
use super::*;

pub(super) fn scale(n: i32, dpi: i32) -> i32 {
    n * dpi / 96
}

const TWO_PANES: i32 = 790;
const EDGE: i32 = 36;

#[derive(Clone, Copy)]
pub(super) struct Workspace {
    pub header: i32,
    pub left: i32,
    pub right: i32,
    bottom: i32,
    nav: bool,
    reviews: bool,
}

impl Workspace {
    fn measure(
        width: i32,
        height: i32,
        dpi: i32,
        tabs: bool,
        nav: bool,
        reviews: bool,
        standalone: bool,
    ) -> Self {
        let d = |n| scale(n, dpi);
        let nav = nav && (!reviews || width >= d(TWO_PANES));
        let left = if nav {
            (width * 18 / 100).clamp(d(150), d(288))
        } else if standalone {
            0
        } else {
            d(EDGE)
        };
        let right = width
            - if reviews {
                if width < d(TWO_PANES) {
                    d(218)
                } else {
                    (width / 4).clamp(d(236), d(350))
                }
            } else if standalone {
                0
            } else {
                d(EDGE)
            };
        Self {
            header: if tabs { d(35) } else { 0 },
            left,
            right,
            bottom: height - d(29),
            nav,
            reviews,
        }
    }
}

impl App {
    fn workspace(&self) -> Workspace {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        Workspace::measure(
            rect.right,
            rect.bottom,
            self.dpi,
            !self.wait && self.active.is_some(),
            !self.wait && (self.tree_open || self.outline_open),
            self.reviews_visible(),
            self.wait,
        )
    }

    pub(super) fn nav_is_visible(&self) -> bool {
        self.workspace().nav
    }

    pub(super) fn nav_needs_room(&self) -> bool {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rect);
        }
        rect.right < self.dip(TWO_PANES) && self.draft.is_none()
    }

    pub(super) fn layout(&mut self) {
        let w = self.workspace();
        let dpi = self.dpi;
        let d = |n| scale(n, dpi);
        let active = self.active.is_some();
        let tabs = !self.wait && active;
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let width = client.right;
        let max_tabs = (width - d(41)).max(0);
        if tabs {
            move_to(self.tab, 0, 0, max_tabs, w.header, true);
            let mut last = RECT::default();
            let mut edge = max_tabs;
            if send(
                self.tab,
                TCM_GETITEMRECT,
                self.docs.len() - 1,
                &mut last as *mut _ as isize,
            ) != 0
                && last.right + d(4) < max_tabs
            {
                edge = last.right + d(4);
                move_to(self.tab, 0, 0, edge, w.header, true);
            }
            move_to(self.open_tab, edge, d(2), d(34), d(30), true);
        } else {
            move_to(self.tab, 0, 0, 0, 0, false);
            move_to(self.open_tab, 0, 0, 0, 0, false);
        }

        self.bar.place(w, dpi);
        let nav_edge = if w.nav { w.left } else { d(EDGE) };
        move_to(
            self.outline,
            d(12),
            w.header + d(11),
            w.left - d(54),
            d(24),
            w.nav,
        );
        set_text(
            self.outline_toggle,
            tr(match (w.nav, self.outline_open) {
                (true, true) => L::HideOutline,
                (true, false) => L::HideFiles,
                _ => L::ShowOutline,
            }),
        );
        set_text(
            self.review_toggle,
            tr(if w.reviews {
                L::HideComments
            } else {
                L::ShowComments
            }),
        );
        move_to(
            self.outline_toggle,
            nav_edge - d(33),
            w.header + d(7),
            d(29),
            d(29),
            active && !self.wait,
        );
        move_to(
            self.review_toggle,
            width - d(33),
            w.header + d(7),
            d(28),
            d(29),
            active && !self.wait,
        );
        for button in [
            self.outline_toggle,
            self.review_toggle,
            self.review_actions[0],
        ] {
            unsafe {
                let _ = EnableWindow(button, self.draft.is_none());
            }
        }
        let mut y = w.header + d(43);
        move_to(
            self.tree,
            d(5),
            y,
            w.left - d(10),
            w.bottom - y,
            w.nav && !self.outline_open,
        );
        move_to(
            self.outline_list,
            d(5),
            y,
            w.left - d(10),
            w.bottom - y,
            w.nav && self.outline_open,
        );

        let ruler = if active && self.mode() % 2 == 1 {
            d(47)
        } else {
            0
        };
        let left = if w.nav { w.left } else { 0 };
        let right = if w.reviews { w.right } else { width };
        let editorial_width = right - left;
        let search = self.find_open && active;
        let search_width = (editorial_width - d(232)).clamp(d(56), d(440));
        let find_x = left + d(8);
        move_to(self.find_box, find_x, y, search_width, d(28), search);
        let mut buttons_x = find_x + search_width + d(4);
        for (i, (button, width)) in self
            .find_actions
            .into_iter()
            .zip([42, 42, 58, 58])
            .enumerate()
        {
            move_to(button, buttons_x, y, d(width), d(28), search);
            if i == 2 {
                unsafe {
                    let _ = EnableWindow(button, !self.find_headings);
                }
            }
            buttons_x += d(width + 2);
        }
        if search {
            y += d(34);
        }
        let replacing = search && self.replace_open && self.mode() == 3 && self.draft.is_none();
        move_to(self.replace_box, find_x, y, search_width, d(28), replacing);
        for (i, button) in self.replace_actions.into_iter().enumerate() {
            move_to(
                button,
                find_x + search_width + d(4 + i as i32 * 102),
                y,
                d(100),
                d(28),
                replacing,
            );
        }
        if replacing {
            y += d(34);
        }
        let central = (editorial_width - ruler).max(0);
        let body = (w.bottom - y).max(0);
        let placement = (left + ruler, y, central, body, active);
        if self.editor_placement != placement {
            self.editor_placement = placement;
            move_to(self.editor, left + ruler, y, central, body, active);
        }
        let metrics = (central, body, dpi);
        if self.editor_metrics != metrics {
            self.editor_metrics = metrics;
            let gutter = if ruler > 0 {
                d(13)
            } else {
                ((central - d(880)) / 2).max(d(24).min(central / 10))
            };
            margins(self.editor, central, body, gutter, dpi);
        }
        move_to(
            self.empty,
            left + d(24),
            y + d(42),
            central - d(48),
            d(44),
            !active,
        );
        self.content_width = central;
        self.document_top = y;
        self.review_bounds = RECT {
            left: w.right,
            right: width,
            top: w.header,
            bottom: w.bottom,
        };
        self.layout_review_panel();

        move_to(
            self.status,
            d(12),
            w.bottom + d(5),
            width - d(136),
            d(20),
            true,
        );
        let mut zx = width - d(117);
        let [out, reset, into] = self.zoom_buttons;
        for (button, size) in [(reset, 48), (out, 30), (into, 30)] {
            move_to(button, zx, w.bottom + d(1), d(size), d(27), active);
            zx += d(size);
        }
        unsafe {
            let _ = RedrawWindow(
                Some(self.hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN,
            );
        }
    }

    fn ruler_rect(&self) -> Option<RECT> {
        let (x, y, _, height, visible) = self.editor_placement;
        (visible && self.mode() % 2 == 1).then(|| RECT {
            left: x - self.dip(47),
            right: x,
            top: y,
            bottom: y + height,
        })
    }

    pub(super) fn invalidate_ruler(&self) {
        if let Some(rect) = self.ruler_rect() {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
            }
        }
    }

    pub(super) fn update_ruler(&mut self) {
        self.source_lines.clear();
        if self.mode() % 2 == 1 {
            self.source_lines.push(0);
            self.source_lines.extend(
                self.view
                    .text
                    .iter()
                    .enumerate()
                    .filter_map(|(at, &unit)| (unit == 13).then_some(at + 1)),
            );
        }
        self.invalidate_ruler();
    }

    pub(super) fn paint_layout(&self, dc: HDC, dirty: RECT) {
        let w = self.workspace();
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        fill(dc, &dirty, self.palette.frame);
        let line = |x, y, right, bottom| {
            fill(
                dc,
                &RECT {
                    left: x,
                    top: y,
                    right,
                    bottom,
                },
                self.palette.line,
            )
        };
        if w.header > 0 {
            line(0, w.header - 1, client.right, w.header);
        }
        let band = w.header + self.dip(43);
        line(0, band - 1, client.right, band);
        line(0, w.bottom, client.right, w.bottom + 1);
        if w.nav {
            line(w.left - 1, w.header, w.left, w.bottom);
        }
        if w.reviews {
            line(w.right, w.header, w.right + 1, w.bottom);
            self.paint_review_draft(dc);
        }
        if let Some(r) = self.ruler_rect()
            && r.top < dirty.bottom
            && dirty.top < r.bottom
        {
            self.paint_ruler(dc, r);
        }
    }

    fn paint_ruler(&self, dc: HDC, rect: RECT) {
        let p = self.appearance.style(self.dark).palette;
        fill(dc, &rect, p.canvas);
        let row_height = (self.appearance.source.half_points * self.dpi * self.zoom / 14_400)
            .clamp(6, self.dip(16));
        let small_font = (row_height < self.dip(14))
            .then(|| super::atoms::ui_font(row_height))
            .filter(|font| !font.is_invalid());
        let font = small_font.unwrap_or(self.font);
        let saved_dc = unsafe {
            let saved = SaveDC(dc);
            if saved != 0 {
                IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
            }
            saved
        };
        let first = send(self.editor, EM_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize;
        let total = send(self.editor, EM_GETLINECOUNT, 0, 0).max(0) as usize;
        let focused = self
            .review_record()
            .filter(|_| self.show_review_marks())
            .and_then(|record| self.view.review_range(&self.parsed, &record.binding))
            .zip(self.active_review.as_ref())
            .and_then(|(span, id)| {
                let row = self
                    .review_indices
                    .iter()
                    .position(|&i| &self.parsed.records[i].review.id == id)?;
                Some((
                    self.source_lines
                        .partition_point(|&at| at <= span.start)
                        .saturating_sub(1),
                    row + 1,
                ))
            });
        for visual in first..total {
            let offset = send(self.editor, EM_LINEINDEX, visual, 0);
            let Ok(offset) = usize::try_from(offset) else {
                break;
            };
            let y = rect.top + char_point(self.editor, offset).y;
            if y >= rect.bottom {
                break;
            }
            let Ok(line) = self.source_lines.binary_search(&offset) else {
                continue;
            };
            if y + row_height <= rect.top {
                continue;
            }
            text(
                dc,
                font,
                p.muted,
                &(line + 1).to_string(),
                RECT {
                    top: y,
                    bottom: (y + row_height).min(rect.bottom),
                    right: rect.right - self.dip(19),
                    ..rect
                },
                DT_SINGLELINE | DT_RIGHT,
            );
            if let Some((_, row)) = focused.filter(|&(line_no, _)| line == line_no) {
                let marker = RECT {
                    left: rect.right - self.dip(18),
                    right: rect.right,
                    top: y,
                    bottom: y + row_height,
                };
                rounded(dc, marker, self.dip(6), p.highlight, p.highlight);
                text(
                    dc,
                    font,
                    p.accent,
                    &row.to_string(),
                    marker,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
            }
        }
        unsafe {
            if saved_dc != 0 {
                let _ = RestoreDC(dc, saved_dc);
            }
            if let Some(font) = small_font {
                let _ = DeleteObject(font.into());
            }
        }
    }
}
