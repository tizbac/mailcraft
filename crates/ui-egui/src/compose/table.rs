// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Layout of the table objects pasted into a message.

use std::sync::Arc;

use egui::text::{CCursor, CharIndex};
use egui::{
    Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2,
    text::{Galley, LayoutJob},
    vec2,
};
use mailcraft_model::rich::{CellAlignment, CellBorders, PastedTable, Style};
use mailcraft_model::signature::Picture;

use crate::theme::Tokens;

const PADDING: f32 = 4.0;
const MIN_ROW: f32 = 24.0;
/// The room between a cell's picture and its text.
const IMAGE_GAP: f32 = 4.0;

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
    /// The picture above its text, with the size it is drawn at.
    image: Option<(u32, Vec2)>,
    /// Its text, its lines' line breaks counted as characters.
    string: String,
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

    pub fn new(ctx: &egui::Context, table: &PastedTable, tokens: &Tokens, room: f32, pictures: &[Picture]) -> Self {
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
            let image = table
                .rows()
                .get(row)
                .and_then(|row| row.get(cell_index))
                .and_then(|cell| cell.image)
                .and_then(|id| pictures.iter().find(|p| p.id == id))
                .map(|p| super::size_of(ctx, p, (right - left - 2.0 * PADDING).max(8.0)));
            let extra = image.map_or(0.0, |size| size.y + IMAGE_GAP);
            let missing = (text.size().y + extra + 2.0 * PADDING - spanned.iter().sum::<f32>()).max(0.0);
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
                let image = table
                    .rows()
                    .get(row)
                    .and_then(|row| row.get(cell_index))
                    .and_then(|cell| cell.image)
                    .and_then(|id| pictures.iter().find(|p| p.id == id))
                    .map(|p| (p.id, super::size_of(ctx, p, (right - left - 2.0 * PADDING).max(8.0))));
                let extra = image.map_or(0.0, |(_, size)| size.y + IMAGE_GAP);
                let string = table
                    .rows()
                    .get(row)
                    .and_then(|row| row.get(cell_index))
                    .map(|cell| {
                        cell.lines.iter().map(|line| line.iter().map(|run| run.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
                    })
                    .unwrap_or_default();
                Some(Cell { rect, text, background, borders, row, column: cell_index, text_at: pos2(x, rect.top() + PADDING + extra), image, string })
            })
            .collect();
        Self { size: vec2(total * scale, heights.iter().sum()), cells, plain: None }
    }

    /// The row and the cell's place in it under `at`, measured from the table's own top left.
    pub fn cell_at(&self, at: Vec2) -> Option<(usize, usize)> {
        self.cells.iter().find(|cell| cell.rect.contains(at.to_pos2())).map(|cell| (cell.row, cell.column))
    }

    /// The pictures above the cells' text, each with where it is drawn when the table is at `at`.
    pub fn images(&self, at: Pos2) -> Vec<(Rect, u32)> {
        self.cells
            .iter()
            .filter_map(|cell| {
                let (id, size) = cell.image?;
                let origin = pos2(cell.rect.left() + PADDING, cell.rect.top() + PADDING);
                Some((Rect::from_min_size(origin + at.to_vec2(), size), id))
            })
            .collect()
    }

    /// The row, the cell and the character in it nearest to `at`, measured from the table's own
    /// top left.
    pub fn caret_at(&self, at: Vec2) -> Option<(usize, usize, usize)> {
        let point = at.to_pos2();
        let cell = self.cells.iter().find(|cell| cell.rect.contains(point))?;
        let index = cell.text.cursor_from_pos(point.to_vec2() - cell.text_at.to_vec2()).index.0;
        Some((cell.row, cell.column, index.min(cell.text.len())))
    }

    /// A cell's text, its lines' line breaks counted as characters.
    pub fn cell_text(&self, row: usize, column: usize) -> Option<String> {
        let cell = self.cells.iter().find(|cell| cell.row == row && cell.column == column)?;
        Some(cell.string.clone())
    }

    /// Where the word around character `at` of a cell begins and ends (a run of characters that
    /// are not spaces), for a double click; `None` when there is no such cell.
    pub fn cell_word_ends(&self, row: usize, column: usize, at: usize) -> Option<(usize, usize)> {
        let text = self.cell_text(row, column)?;
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return Some((0, 0));
        }
        let at = at.min(chars.len());
        let space = |c: char| c.is_whitespace();
        let mut from = at.min(chars.len().saturating_sub(1));
        while from > 0 && !space(chars.get(from).copied().unwrap_or(' ')) {
            from -= 1;
        }
        let mut to = at;
        while to < chars.len() && !space(chars.get(to).copied().unwrap_or(' ')) {
            to += 1;
        }
        Some((from, to))
    }

    /// The character of the cell at `row`, `column` nearest to `at` (measured from the table's
    /// own top left), for a drag inside it.
    pub fn caret_at_in(&self, row: usize, column: usize, at: Vec2) -> Option<usize> {
        let cell = self.cells.iter().find(|cell| cell.row == row && cell.column == column)?;
        let point = at.to_pos2().clamp(cell.rect.min, cell.rect.max);
        let index = cell.text.cursor_from_pos(point.to_vec2() - cell.text_at.to_vec2()).index.0;
        Some(index.min(cell.text.len()))
    }

    /// Where the caret stands for character `at` of the cell at `row`, `column`, measured from
    /// the table's own top left; a line as tall as the text there.
    pub fn caret_rect(&self, row: usize, column: usize, at: usize) -> Option<Rect> {
        let cell = self.cells.iter().find(|cell| cell.row == row && cell.column == column)?;
        let cursor = CCursor::new(at.min(cell.text.len()));
        let r = cell.text.pos_from_cursor(cursor);
        Some(r.translate(cell.text_at.to_vec2()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        painter: &Painter,
        at: Pos2,
        tokens: &Tokens,
        selected: Option<(usize, usize)>,
        caret: Option<(usize, usize, usize)>,
        selection: Option<(usize, usize, usize, usize)>,
    ) {
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
            // The text selected in this cell stands behind it, as the message's selection does.
            if let Some((row, column, from, to)) = selection
                && (row, column) == (cell.row, cell.column)
            {
                paint_cell_selection(painter, &cell.text, cell.text_at + at.to_vec2(), from, to, tokens.selection);
            }
            painter.with_clip_rect(rect).galley(cell.text_at + at.to_vec2(), cell.text.clone(), tokens.text);
            if let Some((row, column, caret_at)) = caret
                && (row, column) == (cell.row, cell.column)
                && let Some(line) = self.caret_rect(row, column, caret_at)
            {
                let line = line.translate(at.to_vec2());
                painter.line_segment([line.center_top(), line.center_bottom()], Stroke::new(1.5, tokens.accent_text));
            }
        }
    }
}

/// The selected characters of a cell's text, behind it.
fn paint_cell_selection(painter: &Painter, galley: &Galley, at: Pos2, from: usize, to: usize, fill: Color32) {
    let (a, b) = (from.min(to), from.max(to));
    let mut seen = 0usize;
    for row in &galley.rows {
        let n = row.char_count_including_newline().0;
        let (rs, re) = (seen, seen + n);
        seen = re;
        let (lo, hi) = (a.max(rs), b.min(re));
        if lo >= hi {
            continue;
        }
        let x0 = row.pos.x + row.x_offset(CharIndex(lo.saturating_sub(rs)));
        let x1 = row.pos.x + row.x_offset(CharIndex(hi.saturating_sub(rs)));
        let r = Rect::from_min_max(pos2(x0, row.min_y()), pos2(x1, row.max_y())).translate(at.to_vec2());
        painter.rect_filled(r, 0.0, fill);
    }
}
