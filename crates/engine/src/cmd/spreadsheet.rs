// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Excel's "XML Spreadsheet" clipboard data (SpreadsheetML 2003) as the HTML table a message
//! takes: Windows puts it beside "HTML Format" when a range is copied, and some copies carry no
//! usable HTML at all. Only what a pasted grid needs is read — the worksheet's table, rows,
//! cells, their `ss:Index` places, merged cells and their text — under finite bounds. Nothing in
//! it runs, and no formula is evaluated.

use quick_xml::Reader;
use quick_xml::events::Event;

/// The most XML taken in (the clipboard's own limit is the same as a paste's).
pub const MAX_BYTES: usize = 4 << 20;
/// Most rows and columns a table made of it may have: what a message's table holds.
const MAX_ROWS: usize = mailcraft_model::rich::MAX_PASTED_ROWS;
const MAX_COLUMNS: usize = mailcraft_model::rich::MAX_PASTED_COLUMNS;
/// The longest one cell's text kept, in bytes.
const MAX_CELL: usize = 64 * 1024;

/// The HTML table a SpreadsheetML document holds, or `None` when it holds no worksheet table (or
/// is no such document, or is larger than is taken). Text is escaped; the types are kept as the
/// numbers or words the cells show.
pub fn to_html(xml: &str) -> Option<String> {
    if xml.len() > MAX_BYTES || xml.len() < 16 {
        return None;
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut sheet = Sheet::default();
    loop {
        let event = reader.read_event().ok()?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "Table" => sheet.in_table = true,
                    "Row" if sheet.in_table => sheet.open_row(attribute(&e, "Index")),
                    "Cell" if sheet.in_row => sheet.open_cell(&e),
                    "Data" if sheet.in_cell => sheet.data.clear(),
                    _ => {}
                }
                if empty {
                    match name.as_ref() {
                        "Cell" if sheet.in_row => sheet.close_cell(),
                        "Row" if sheet.in_table => sheet.close_row()?,
                        _ => {}
                    }
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "Cell" if sheet.in_row => sheet.close_cell(),
                "Row" if sheet.in_table => sheet.close_row()?,
                "Table" => sheet.in_table = false,
                _ => {}
            },
            Event::Text(t) if sheet.in_cell => {
                let text = t.xml_content(quick_xml::XmlVersion::Implicit1_0);
                if sheet.data.len().saturating_add(text.len()) <= MAX_CELL {
                    sheet.data.push_str(&text);
                }
            }
            Event::CData(t) if sheet.in_cell => {
                let text = t.into_inner();
                if sheet.data.len().saturating_add(text.len()) <= MAX_CELL {
                    sheet.data.push_str(&text);
                }
            }
            // (quick-xml hands an entity over by itself: `&amp;` is `amp`, and `&#65;` is a char.)
            Event::GeneralRef(r) if sheet.in_cell => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.as_ref() {
                        "amp" => "&".to_owned(),
                        "lt" => "<".to_owned(),
                        "gt" => ">".to_owned(),
                        "quot" => "\"".to_owned(),
                        "apos" => "'".to_owned(),
                        _ => String::new(),
                    },
                };
                if sheet.data.len().saturating_add(text.len()) <= MAX_CELL {
                    sheet.data.push_str(&text);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    (!sheet.html.is_empty()).then(|| format!("<table>{}</table>", sheet.html))
}

/// An attribute's value by its local name (`ss:Index` read as `Index`).
#[allow(deprecated)]
fn attribute(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == name).and_then(|a| a.unescape_value().ok()).map(|v| v.into_owned())
}

#[derive(Default)]
struct Sheet {
    html: String,
    in_table: bool,
    in_row: bool,
    in_cell: bool,
    /// The next row and column (0-based) an element without `ss:Index` goes in.
    row: usize,
    column: usize,
    /// The cells of the row being written, and the most it has.
    columns: usize,
    data: String,
}

impl Sheet {
    fn open_row(&mut self, index: Option<String>) {
        // (`ss:Index` is 1-based; a row without one comes after the last.)
        let at = index.and_then(|i| i.trim().parse::<usize>().ok()).and_then(|i| i.checked_sub(1)).unwrap_or(self.row);
        if at >= MAX_ROWS {
            return;
        }
        // The rows the `ss:Index` skips are empty ones in the sheet.
        while self.row < at {
            self.html.push_str("<tr></tr>");
            self.row = self.row.saturating_add(1);
        }
        self.row = at;
        self.column = 0;
        self.columns = 0;
        self.in_row = true;
        self.html.push_str("<tr>");
    }

    fn open_cell(&mut self, e: &quick_xml::events::BytesStart<'_>) {
        let across = attribute(e, "MergeAcross").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0).min(MAX_COLUMNS);
        let down = attribute(e, "MergeDown").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0).min(MAX_ROWS);
        let at = attribute(e, "Index").and_then(|i| i.trim().parse::<usize>().ok()).and_then(|i| i.checked_sub(1)).unwrap_or(self.column);
        if at >= MAX_COLUMNS {
            return;
        }
        // The cells skipped by `ss:Index` are empty ones.
        self.row_fill(at);
        self.html.push_str("<td");
        if across > 0 {
            self.html.push_str(&format!(" colspan=\"{}\"", across + 1));
        }
        if down > 0 {
            self.html.push_str(&format!(" rowspan=\"{}\"", down + 1));
        }
        self.html.push('>');
        self.in_cell = true;
        self.data.clear();
        self.column = at.saturating_add(across).saturating_add(1);
        self.columns = self.columns.max(self.column);
    }

    /// Empty cells up to `at` (0-based), for a cell whose `ss:Index` skips some.
    fn row_fill(&mut self, at: usize) {
        while self.column < at && self.column < MAX_COLUMNS {
            self.html.push_str("<td></td>");
            self.column = self.column.saturating_add(1);
        }
        self.columns = self.columns.max(self.column);
    }

    fn close_cell(&mut self) {
        if !self.in_cell {
            return;
        }
        self.in_cell = false;
        escape_into(&self.data, &mut self.html);
        self.html.push_str("</td>");
    }

    fn close_row(&mut self) -> Option<()> {
        if !self.in_row {
            return Some(());
        }
        self.close_cell();
        self.html.push_str("</tr>");
        self.in_row = false;
        self.row = self.row.saturating_add(1);
        if self.row >= MAX_ROWS {
            return None;
        }
        Some(())
    }
}

/// `text` with what HTML reads as markup escaped.
fn escape_into(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' | '\n' => out.push(' '),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
#[path = "spreadsheet_tests.rs"]
mod tests;
