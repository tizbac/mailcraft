// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Layout of the table objects pasted into a message.

use std::sync::Arc;

use egui::text::CCursor;
use egui::{
    Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2,
    text::{Galley, LayoutJob},
    vec2,
};
use mailcraft_model::rich::{CellAlignment, CellBorders, PastedTable, Style};

use crate::theme::Tokens;

const PADDING: f32 = 4.0;
const MIN_ROW: f32 = 24.0;

struct Cell {
    rect: Rect,
    text: Arc<Galley>,
    background: Option<Color32>,
    borders: CellBorders,
    /// Its row and its place in that row, as the model names it.
    row: usize,
    column: usize,
    /// Where its text starts, measured from the table's own top left.
    text_at: Pos2,
}

pub(super) struct TableLayout {
    pub size: Vec2,
    cells: Vec<Cell>,
    plain: Option<Arc<Galley>>,
}

impl TableLayout {
    pub fn plain(ctx: &egui::Context, table: &PastedTable, tokens: &Tokens, room: f32) -> Self {
        let mut job = LayoutJob::default();
        job.wrap.max_width = room;
        // Plain text is drawn in the window's own fonts (#582).
        job.append(&table.plain(), 0.0, super::text_format(&Style::default(), true, tokens, &mut crate::fonts::Chooser::none()));
        let text = ctx.fonts_mut(|f| f.layout_job(job));
        Self { size: text.size(), cells: Vec::new(), plain: Some(text) }
    }

    pub fn new(ctx: &egui::Context, table: &PastedTable, tokens: &Tokens, room: f32) -> Self {
        let columns = usize::from(table.columns());
        let mut occupied = vec![0; columns];
        let mut widths = vec![24.0_f32; columns];
        let mut heights = vec![MIN_ROW; table.rows().len()];
        let mut cells = Vec::new();
        // One chooser for the table: a cell's family is looked up once, and what isn't here yet is
        // asked for once, at the end (#1493).
        let mut fonts = crate::fonts::Chooser::new(ctx);
        for (row_index, row) in table.rows().iter().enumerate() {
            let mut column = 0;
            for (cell_index, cell) in row.iter().enumerate() {
                while occupied.get(column).is_some_and(|end| *end > row_index) {
                    column += 1;
                }
                if column >= columns {
                    break;
                }
                let end_column = (column + usize::from(cell.colspan)).min(columns);
                let end_row = (row_index + usize::from(cell.rowspan)).min(table.rows().len());
                if end_column <= column || end_row <= row_index {
                    continue;
                }
                let mut job = LayoutJob {
                    halign: match cell.alignment {
                        CellAlignment::Left => egui::Align::Min,
                        CellAlignment::Center => egui::Align::Center,
                        CellAlignment::Right => egui::Align::Max,
                    },
                    ..Default::default()
                };
                let background =
                    cell.background.as_deref().and_then(mailcraft_model::rich::safe_color).and_then(|color| Color32::from_hex(&color).ok());
                let mut format = |style: &Style| {
                    let style = style.clone().clean();
                    let mut format = super::text_format(&style, false, tokens, &mut fonts);
                    let paper = if format.background != Color32::TRANSPARENT { format.background } else { background.unwrap_or(tokens.surface) };
                    let ink = if style.link.is_some() {
                        tokens.accent_text
                    } else {
                        style.color.as_deref().and_then(|c| Color32::from_hex(c).ok()).unwrap_or(tokens.text)
                    };
                    format.color = if tokens.high_contrast {
                        crate::reading::colors::readable_strong(ink, paper)
                    } else {
                        crate::reading::colors::readable(ink, paper)
                    };
                    format.underline.color = format.color;
                    format
                };
                for (i, line) in cell.lines.iter().enumerate() {
                    if i > 0 {
                        job.append("\n", 0.0, format(&Style::default()));
                    }
                    for run in line {
                        job.append(&run.text, 0.0, format(&run.style));
                    }
                }
                let wanted = ctx.fonts_mut(|f| f.layout_job(job.clone()).size().x).min(440.0) + 2.0 * PADDING;
                let per_column = wanted / (end_column - column) as f32;
                for (slot, width) in occupied.iter_mut().zip(&mut widths).take(end_column).skip(column) {
                    *slot = (*slot).max(end_row);
                    *width = width.max(per_column);
                }
                cells.push((row_index, cell_index, end_row, column, end_column, job, background, cell.borders));
                column = end_column;
            }
        }
        fonts.finish(ctx);
        let total = widths.iter().sum::<f32>();
        let scale = (room / total).min(1.0);
        widths.iter_mut().for_each(|w| *w *= scale);
        let edges = |sizes: &[f32]| {
            std::iter::once(0.0)
                .chain(sizes.iter().scan(0.0, |sum, size| {
                    *sum += size;
                    Some(*sum)
                }))
                .collect::<Vec<_>>()
        };
        let xs = edges(&widths);
        let mut laid = Vec::new();
        for (row, cell_index, end_row, column, end_column, mut job, background, borders) in cells {
            let (Some(&left), Some(&right)) = (xs.get(column), xs.get(end_column)) else { continue };
            job.wrap.max_width = (right - left - 2.0 * PADDING).max(1.0);
            let text = ctx.fonts_mut(|f| f.layout_job(job));
            let Some(spanned) = heights.get_mut(row..end_row) else { continue };
            let missing = (text.size().y + 2.0 * PADDING - spanned.iter().sum::<f32>()).max(0.0);
            if let Some(last) = spanned.last_mut() {
                *last += missing;
            }
            laid.push((row, cell_index, end_row, left, right, text, background, borders));
        }
        let ys = edges(&heights);
        let cells = laid
            .into_iter()
            .filter_map(|(row, cell_index, end_row, left, right, text, background, borders)| {
                let rect = Rect::from_min_max(pos2(left, *ys.get(row)?), pos2(right, *ys.get(end_row)?));
                let x = match text.job.halign {
                    egui::Align::Min => rect.left() + PADDING,
                    egui::Align::Center => rect.center().x,
                    egui::Align::Max => rect.right() - PADDING,
                };
                Some(Cell { rect, text, background, borders, row, column: cell_index, text_at: pos2(x, rect.top() + PADDING) })
            })
            .collect();
        Self { size: vec2(total * scale, heights.iter().sum()), cells, plain: None }
    }

    /// The row and the cell's place in it under `at`, measured from the table's own top left.
    pub fn cell_at(&self, at: Vec2) -> Option<(usize, usize)> {
        self.cells.iter().find(|cell| cell.rect.contains(at.to_pos2())).map(|cell| (cell.row, cell.column))
    }

    /// The row, the cell and the character in it nearest to `at`, measured from the table's own
    /// top left.
    pub fn caret_at(&self, at: Vec2) -> Option<(usize, usize, usize)> {
        let point = at.to_pos2();
        let cell = self.cells.iter().find(|cell| cell.rect.contains(point))?;
        let index = cell.text.cursor_from_pos(point.to_vec2() - cell.text_at.to_vec2()).index.0;
        Some((cell.row, cell.column, index.min(cell.text.len())))
    }

    pub fn paint(&self, painter: &Painter, at: Pos2, tokens: &Tokens, selected: Option<(usize, usize)>, caret: Option<(usize, usize, usize)>) {
        if let Some(text) = &self.plain {
            painter.galley(at, text.clone(), tokens.text);
            return;
        }
        for cell in &self.cells {
            let rect = cell.rect.translate(at.to_vec2());
            if let Some(background) = cell.background {
                painter.rect_filled(rect, 0.0, background);
            }
            if selected == Some((cell.row, cell.column)) {
                painter.rect_filled(rect, 0.0, tokens.selection);
            }
            // Each side the owner kept is a line of its own; a cell with all of them is the old
            // whole frame.
            let line = Stroke::new(1.0, tokens.border);
            if cell.borders.all() {
                painter.rect_stroke(rect, 0.0, line, StrokeKind::Inside);
            } else {
                if cell.borders.top {
                    painter.line_segment([rect.left_top(), rect.right_top()], line);
                }
                if cell.borders.right {
                    painter.line_segment([rect.right_top(), rect.right_bottom()], line);
                }
                if cell.borders.bottom {
                    painter.line_segment([rect.left_bottom(), rect.right_bottom()], line);
                }
                if cell.borders.left {
                    painter.line_segment([rect.left_top(), rect.left_bottom()], line);
                }
            }
            let x = match cell.text.job.halign {
                egui::Align::Min => rect.left() + PADDING,
                egui::Align::Center => rect.center().x,
                egui::Align::Max => rect.right() - PADDING,
            };
            painter.with_clip_rect(rect).galley(pos2(x, rect.top() + PADDING), cell.text.clone(), tokens.text);
            if let Some((row, column, caret_at)) = caret
                && (row, column) == (cell.row, cell.column)
            {
                let pos = cell.text.pos_from_cursor(CCursor::new(caret_at.min(cell.text.len()))).min;
                let foot = pos2(at.x + cell.text_at.x + pos.x, at.y + cell.text_at.y + pos.y);
                let height = cell.text.rows.first().map_or(14.0, |row| row.height());
                painter.line_segment([foot, pos2(foot.x, foot.y + height)], Stroke::new(1.5, tokens.accent_text));
            }
        }
    }
}
