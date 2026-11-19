// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! A message as pages a printer can take: its heading (who sent it, to whom, when), then its text
//! and pictures the way the reading pane shows them — headings, bold and italic, coloured words,
//! lists, quotes, rules, pictures and tables — set on A4 and written as a PDF.
//!
//! The blocks are the ones the reading pane is drawn from ([`mailcraft_html`]). Nothing here
//! fetches a picture, runs a style sheet or follows a link: a picture written into the message is
//! printed, one on the web only when the caller hands its bytes over, and one with no bytes at all
//! is printed as its alt text. Hidden sender text never reaches these blocks at all.
//!
//! The fonts are the caller's (the window's own): a word is set in the first face that has all of
//! its letters, so a message in a script the first face lacks still prints.

use std::sync::Arc;

#[cfg(test)]
mod tests;

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::text::{Font, TextDirection};
use mailcraft_html::{Align, Block, BlockKind, Cell, Marker, Rgb, Sides, Span, Table};

/// A font file the pages may be set in.
#[derive(Clone)]
pub struct Face {
    pub data: Arc<Vec<u8>>,
    /// Its index in a TrueType collection: 0 for a plain file.
    pub index: u32,
}

impl Face {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Face {
        Face { data: Arc::new(bytes.into()), index: 0 }
    }
}

/// The faces a message is set in: the body's regular and bold, the monospace of code, and further
/// faces tried, in order, for letters those lack (the system's own, a script's).
#[derive(Clone, Default)]
pub struct Fonts {
    pub regular: Option<Face>,
    pub bold: Option<Face>,
    pub mono: Option<Face>,
    pub fallbacks: Vec<Face>,
}

/// The MailCraft faces: the ones the window draws with — Inter for the body, its semi-bold for
/// what is bold, JetBrains Mono for code.
pub fn bundled_fonts() -> Fonts {
    Fonts {
        regular: Some(Face::new(include_bytes!("../../../assets/fonts/Inter-Regular.ttf"))),
        bold: Some(Face::new(include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"))),
        mono: Some(Face::new(include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"))),
        fallbacks: vec![
            Face::new(include_bytes!("../../../assets/fonts/SourceSans3-Regular.ttf")),
            Face::new(include_bytes!("../../../assets/fonts/NotoSansGeorgian-Regular.ttf")),
        ],
    }
}

/// The paper, in PDF points (72 to the inch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub width: f32,
    pub height: f32,
    /// The room kept free round the text: top, bottom and each side.
    pub margin: f32,
}

impl Paper {
    /// A4: 210 × 297 mm.
    pub const A4: Paper = Paper { width: 595.28, height: 841.89, margin: 50.0 };
    /// US Letter: 8.5 × 11 in.
    pub const LETTER: Paper = Paper { width: 612.0, height: 792.0, margin: 50.0 };

    /// A sheet pages can be set on: finite, and with room left for text between the margins.
    pub fn is_sound(&self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.margin.is_finite()
            && self.width >= 100.0
            && self.height >= 100.0
            && self.margin >= 0.0
            && self.margin * 2.0 + 20.0 <= self.width.min(self.height)
    }
}

impl Default for Paper {
    fn default() -> Paper {
        Paper::A4
    }
}

/// A picture's bytes and what it is (`image/png`); an empty kind means "work it out from the bytes".
pub type PictureBytes = (Arc<Vec<u8>>, String);

/// One message to print.
pub struct Printout<'a> {
    pub subject: &'a str,
    /// Who sent it, as the reading pane says it.
    pub from: &'a str,
    pub to: &'a str,
    pub cc: &'a str,
    /// When, as the reading pane shows it.
    pub date: &'a str,
    /// The names of its files, in order.
    pub attachments: &'a [String],
    /// The body as display blocks: the ones the reading pane shows.
    pub blocks: &'a [Block],
    /// The bytes for a picture the message calls for, by the address it gives (`cid:…` or a web
    /// one). Never fetched here: the caller has them only when the owner let them be brought.
    pub picture: Option<&'a dyn Fn(&str) -> Option<PictureBytes>>,
    /// What stands at the foot of every page, besides its number: empty for nothing.
    pub footer: &'a str,
    pub paper: Paper,
}

/// Why the pages couldn't be written.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

/// The size of a message's body, in points: what the window draws it at.
const BODY: f32 = 10.5;

/// The slant of an italic word in a face that has no italic of its own: the tangent of 12°, since
/// a shear takes the ratio, not the angle.
const ITALIC_SLANT: f32 = 0.2126;

/// The message as a PDF: its heading, its text, its pictures and tables, each page numbered at its
/// foot.
pub fn pdf(out: &Printout<'_>, fonts: &Fonts) -> Result<Vec<u8>, Error> {
    if !out.paper.is_sound() {
        return Err(Error("the paper is not one pages can be set on".into()));
    }
    let faces = chosen_faces(fonts);
    if faces.is_empty() {
        return Err(Error("no fonts are at hand to print with".into()));
    }
    let mut flow = Flow::new(&faces, out.paper, out.picture);
    flow.heading(out);
    flow.blocks(out.blocks);
    if flow.pages.iter().all(Vec::is_empty) {
        return Err(Error("there was nothing to print".into()));
    }
    paint(flow.pages, &faces, out.paper, out.footer)
}

/// The faces in the order the layout uses them: 0 regular, 1 bold, 2 mono, then the fallbacks.
fn chosen_faces(fonts: &Fonts) -> Vec<Face> {
    let mut faces = Vec::new();
    if let Some(regular) = fonts.regular.clone().or_else(|| fonts.fallbacks.first().cloned()) {
        faces.push(regular.clone());
        faces.push(fonts.bold.clone().unwrap_or_else(|| regular.clone()));
        faces.push(fonts.mono.clone().unwrap_or_else(|| regular.clone()));
    }
    faces.extend(fonts.fallbacks.iter().cloned());
    faces
}

/// How a word of the text is set.
#[derive(Clone)]
struct Word {
    text: String,
    face: usize,
    size: f32,
    color: [u8; 3],
    line_height: f32,
    rtl: bool,
    underline: bool,
    strike: bool,
    italic: bool,
    background: Option<[u8; 3]>,
}

/// A word and where it stands in its line.
#[derive(Clone)]
struct Placed {
    word: Word,
    x: f32,
    width: f32,
}

/// One thing to draw on a page, in points from its top left.
enum Op {
    Text { at: Point, size: f32, face: usize, color: [u8; 3], text: String, rtl: bool, italic: bool },
    Line { from: Point, to: Point, width: f32, color: [u8; 3] },
    Fill { rect: Rect, color: [u8; 3] },
    Image { at: Point, size: Size, data: Arc<Vec<u8>>, kind: String },
}

fn shift(op: &mut Op, dx: f32, dy: f32) {
    match op {
        Op::Text { at, .. } => {
            at.x += dx;
            at.y += dy;
        }
        Op::Line { from, to, .. } => {
            from.x += dx;
            from.y += dy;
            to.x += dx;
            to.y += dy;
        }
        Op::Fill { rect, .. } => {
            if let Some(moved) = Rect::from_xywh(rect.left() + dx, rect.top() + dy, rect.width(), rect.height()) {
                *rect = moved;
            }
        }
        Op::Image { at, .. } => {
            at.x += dx;
            at.y += dy;
        }
    }
}

/// CSS pixels to PDF points.
fn px(px: u8) -> f32 {
    f32::from(px) * 0.75
}

/// How wide one letter of a face is. A face is read each time it is measured: reading one is only
/// looking at its tables, and the words are short.
fn advance(face: &Face, size: f32, ch: char) -> f32 {
    match ttf_parser::Face::parse(&face.data, face.index) {
        Ok(parsed) => parsed
            .glyph_index(ch)
            .and_then(|id| parsed.glyph_hor_advance(id))
            .map_or(size * 0.5, |a| f32::from(a) / f32::from(parsed.units_per_em().max(1)) * size),
        Err(_) => size * 0.5,
    }
}

fn covers(face: &Face, ch: char) -> bool {
    ch.is_whitespace() || ttf_parser::Face::parse(&face.data, face.index).is_ok_and(|parsed| parsed.glyph_index(ch).is_some())
}

fn text_width(faces: &[Face], face: usize, size: f32, text: &str) -> f32 {
    match faces.get(face) {
        Some(face) => text.chars().fold(0.0, |sum, ch| sum + advance(face, size, ch)),
        None => size * 0.5 * text.chars().count() as f32,
    }
}

/// The face that has every letter of `text`, starting with `preferred`; when none does, the
/// preferred one, as a screen falls back.
fn face_for(faces: &[Face], text: &str, preferred: usize) -> usize {
    let Some(preferred_face) = faces.get(preferred) else { return 0 };
    if text.chars().all(|ch| covers(preferred_face, ch)) {
        return preferred;
    }
    for (at, face) in faces.iter().enumerate() {
        if at != preferred && text.chars().all(|ch| covers(face, ch)) {
            return at;
        }
    }
    preferred
}

/// The running column of pages a message is set in.
struct Flow<'a> {
    faces: Vec<Face>,
    paper: Paper,
    pages: Vec<Vec<Op>>,
    y: f32,
    source: Option<&'a dyn Fn(&str) -> Option<PictureBytes>>,
    /// Pixels of picture this printout has room for, counted as it is set: a message can name more
    /// pictures than any printer could take.
    pictures: u64,
}

impl<'a> Flow<'a> {
    fn new(faces: &[Face], paper: Paper, source: Option<&'a dyn Fn(&str) -> Option<PictureBytes>>) -> Flow<'a> {
        Flow { faces: faces.to_vec(), paper, pages: vec![Vec::new()], y: paper.margin, source, pictures: 0 }
    }

    fn left(&self) -> f32 {
        self.paper.margin
    }

    fn width(&self) -> f32 {
        self.paper.width - 2.0 * self.paper.margin
    }

    /// The lowest a line may reach on this page: the foot keeps room for the page number.
    fn floor(&self) -> f32 {
        self.paper.height - self.paper.margin - 16.0
    }

    fn need(&mut self, height: f32) {
        if self.y + height > self.floor() {
            self.pages.push(Vec::new());
            self.y = self.paper.margin;
        }
    }

    fn current(&mut self) -> &mut Vec<Op> {
        let at = self.pages.len().saturating_sub(1);
        &mut self.pages[at]
    }

    fn heading(&mut self, out: &Printout<'_>) {
        let subject = if out.subject.trim().is_empty() { "(no subject)" } else { out.subject.trim() };
        self.put_words(self.words_of(subject, 15.5, 1, false, [0x1a, 0x1a, 0x1a]), self.width(), Align::Left, self.left());
        self.y += 4.0;
        for (name, value) in [("From", out.from), ("To", out.to), ("Cc", out.cc), ("Date", out.date)] {
            if value.trim().is_empty() {
                continue;
            }
            self.put_words(self.words_of(&format!("{name}: {value}"), 9.5, 0, false, [0x55, 0x55, 0x55]), self.width(), Align::Left, self.left());
        }
        if !out.attachments.is_empty() {
            let names: Vec<String> = out.attachments.iter().take(20).cloned().collect();
            let more = out.attachments.len().saturating_sub(names.len());
            let line =
                if more > 0 { format!("Attachments: {} ({} more)", names.join(", "), more) } else { format!("Attachments: {}", names.join(", ")) };
            self.put_words(self.words_of(&line, 9.5, 0, false, [0x55, 0x55, 0x55]), self.width(), Align::Left, self.left());
        }
        self.y += 6.0;
        let (rule_left, rule_right, rule_y) = (self.left(), self.left() + self.width(), self.y);
        self.current().push(Op::Line {
            from: Point::from_xy(rule_left, rule_y),
            to: Point::from_xy(rule_right, rule_y),
            width: 0.6,
            color: [0x99, 0x99, 0x99],
        });
        self.y += 12.0;
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for block in blocks.iter().take(3000) {
            self.block(block, 0);
        }
    }

    fn block(&mut self, block: &Block, depth: u8) {
        let quote = f32::from(block.quote.min(8)) * 10.0;
        let indent = quote + px(block.inset[0]);
        let width = (self.width() - indent).max(20.0);
        let left = self.left() + indent;
        let gap = if block.gap > 0 { px(block.gap) } else { f32::from(block.space.min(3)) * BODY };
        if gap > 0.0 {
            self.y += gap.min(48.0);
        }
        if self.y >= self.floor() {
            self.need(1.0);
        }
        if block.quote > 0
            && let Some(rect) = Rect::from_xywh(self.left() + quote - 5.0, self.y, 2.0, 9.0)
        {
            self.current().push(Op::Fill { rect, color: [0xc8, 0xc8, 0xc8] });
        }
        let height = block.line_height.map_or(BODY * 1.4, |h| h.of(BODY).clamp(BODY, BODY * 2.5));
        match &block.kind {
            BlockKind::Rule => {
                self.need(10.0);
                let line = &block.line;
                let at = self.y + 4.0;
                self.current().push(Op::Line {
                    from: Point::from_xy(left, at),
                    to: Point::from_xy(left + width, at),
                    width: (px(line.width) * 0.6).clamp(0.4, 6.0),
                    color: line.color.map_or([0xaa, 0xaa, 0xaa], |Rgb(r, g, b)| [r, g, b]),
                });
                self.y += 10.0;
            }
            BlockKind::Image { src, width: asked, height: tall, .. } => {
                self.image(block, src, *asked, *tall, left, width);
            }
            BlockKind::Table(table) => {
                if depth < 3 {
                    self.table(table, left, width, depth);
                } else {
                    // (A table inside a table inside a table: its cells' text, without the frame.)
                    for row in &table.rows {
                        for cell in &row.cells {
                            self.blocks(&cell.blocks);
                        }
                    }
                }
            }
            BlockKind::ListItem { marker, depth } => {
                let mark = match marker {
                    Marker::Bullet => "•".to_string(),
                    Marker::Number(n) => format!("{n}."),
                    Marker::Label(text) => text.clone(),
                    Marker::None => String::new(),
                };
                let hang = f32::from(*depth) * 14.0;
                if mark.is_empty() {
                    self.put_words(self.styled_words(block, BODY, height), width - hang, block.align, left + hang);
                } else {
                    let mark_word = Word {
                        text: mark.clone(),
                        face: face_for(&self.faces, &mark, 0),
                        size: BODY,
                        color: [0x33, 0x33, 0x33],
                        line_height: height,
                        rtl: block.rtl,
                        underline: false,
                        strike: false,
                        italic: false,
                        background: None,
                    };
                    let mark_w = text_width(&self.faces, mark_word.face, BODY, &mark) + 6.0;
                    let mut lines = self.wrap(self.styled_words(block, BODY, height), (width - hang - mark_w).max(20.0));
                    if let Some(first) = lines.first_mut() {
                        for placed in first {
                            placed.x += mark_w;
                        }
                    }
                    self.need(height);
                    let mark_at = Point::from_xy(left + hang, self.y + BODY);
                    self.current().push(Op::Text {
                        at: mark_at,
                        size: mark_word.size,
                        face: mark_word.face,
                        color: mark_word.color,
                        text: mark_word.text,
                        rtl: false,
                        italic: false,
                    });
                    self.put(lines, width - hang, block.align, left + hang);
                }
            }
            BlockKind::Preformatted => {
                let text = block.spans.iter().map(|s| s.text.as_str()).collect::<String>();
                for line in text.split('\n').take(1000) {
                    let face = face_for(&self.faces, line, 2);
                    let word = Word {
                        text: line.to_string(),
                        face,
                        size: BODY,
                        color: [0x22, 0x22, 0x22],
                        line_height: height,
                        rtl: false,
                        underline: false,
                        strike: false,
                        italic: false,
                        background: None,
                    };
                    let w = text_width(&self.faces, face, BODY, line);
                    self.put(vec![vec![Placed { word, x: 0.0, width: w }]], width, Align::Left, left);
                }
            }
            _ => {
                let heading = match block.kind {
                    BlockKind::Heading(level) => Some(level),
                    _ => None,
                };
                let mut words = self.styled_words(block, BODY, height);
                if let Some(level) = heading {
                    let size = match level {
                        1 => 16.0,
                        2 => 14.0,
                        3 => 12.5,
                        4 => 11.5,
                        5 => 10.5,
                        _ => BODY,
                    };
                    for word in &mut words {
                        word.size = size;
                        word.line_height = size * 1.4;
                    }
                }
                self.put(self.wrap(words, width), width, block.align, left);
            }
        }
    }

    fn styled_words(&self, block: &Block, base: f32, line_height: f32) -> Vec<Word> {
        let mut words = Vec::new();
        // Whether the words so far end with a gap: a span can begin or end with spaces, and the
        // space between two spans is as much part of the line as the space inside one.
        let mut open = false;
        for span in &block.spans {
            let word = self.word_of(span, base, line_height, block.rtl);
            let pieces: Vec<&str> = span.text.split_whitespace().collect();
            let leading = span.text.chars().next().is_some_and(char::is_whitespace);
            if pieces.is_empty() {
                open = open || span.text.chars().any(char::is_whitespace);
                continue;
            }
            for (i, piece) in pieces.iter().enumerate() {
                if i > 0 || open || (leading && !words.is_empty()) {
                    words.push(Word { text: " ".to_string(), ..word.clone() });
                }
                words.push(Word { text: piece.to_string(), ..word.clone() });
            }
            open = span.text.chars().last().is_some_and(char::is_whitespace);
        }
        words
    }

    fn words_of(&self, text: &str, size: f32, face: usize, rtl: bool, color: [u8; 3]) -> Vec<Word> {
        let word = Word {
            text: String::new(),
            face: face_for(&self.faces, text, face),
            size,
            color,
            line_height: size * 1.4,
            rtl,
            underline: false,
            strike: false,
            italic: false,
            background: None,
        };
        let mut words = Vec::new();
        for (i, piece) in text.split_whitespace().enumerate() {
            if i > 0 {
                words.push(Word { text: " ".to_string(), ..word.clone() });
            }
            words.push(Word { text: piece.to_string(), ..word.clone() });
        }
        words
    }

    fn word_of(&self, span: &Span, base: f32, line_height: f32, rtl: bool) -> Word {
        let preferred = if span.code {
            2
        } else if span.bold {
            1
        } else {
            0
        };
        let size = span.size.map_or(base, px).clamp(6.0, 30.0);
        let link = span.link.is_some();
        Word {
            text: String::new(),
            face: face_for(&self.faces, &span.text, preferred),
            size,
            color: if link { [0x1a, 0x5c, 0xc8] } else { span.color.map_or([0x1a, 0x1a, 0x1a], |Rgb(r, g, b)| [r, g, b]) },
            line_height: line_height.max(size),
            rtl,
            underline: span.underline || (link && !span.bare),
            strike: span.strike,
            italic: span.italic,
            background: span.background.map(|Rgb(r, g, b)| [r, g, b]),
        }
    }

    /// The lines a run of words makes in `room`.
    fn wrap(&self, words: Vec<Word>, room: f32) -> Vec<Vec<Placed>> {
        let mut lines: Vec<Vec<Placed>> = Vec::new();
        let mut line: Vec<Placed> = Vec::new();
        let mut used = 0.0;
        for mut word in words {
            if word.text.is_empty() {
                continue;
            }
            // A slanted word needs a little more room than its upright letters: its top leans right.
            let slant = if word.italic && !word.text.trim().is_empty() { word.size * ITALIC_SLANT } else { 0.0 };
            let mut w = text_width(&self.faces, word.face, word.size, &word.text) + slant;
            if !line.is_empty() && used + w > room {
                lines.push(std::mem::take(&mut line));
                used = 0.0;
                if word.text.trim().is_empty() {
                    continue;
                }
            }
            if w > room && room > 10.0 {
                // (A word longer than the room: broken between letters, as a browser does.)
                let Some(face) = self.faces.get(word.face) else { continue };
                let mut chunk = String::new();
                let mut chunk_w = 0.0;
                for ch in word.text.chars() {
                    let a = advance(face, word.size, ch);
                    if chunk_w + a > room && !chunk.is_empty() {
                        let piece = Word { text: std::mem::take(&mut chunk), ..word.clone() };
                        lines.push(vec![Placed { word: piece, x: 0.0, width: chunk_w + slant }]);
                        chunk_w = 0.0;
                    }
                    chunk.push(ch);
                    chunk_w += a;
                }
                word.text = chunk;
                w = chunk_w + slant;
            }
            // The words come with their own spaces between them, so nothing is added here.
            line.push(Placed { x: used, width: w, word });
            used += w;
            if used > room {
                lines.push(std::mem::take(&mut line));
                used = 0.0;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn put_words(&mut self, words: Vec<Word>, room: f32, align: Align, left: f32) {
        let wrapped = self.wrap(words, room);
        self.put(wrapped, room, align, left);
    }

    fn put(&mut self, lines: Vec<Vec<Placed>>, room: f32, align: Align, left: f32) {
        for line in lines {
            let height = line.iter().map(|p| p.word.line_height).fold(BODY * 1.4, f32::max);
            self.need(height);
            let line_width = line.last().map_or(0.0, |p| p.x + p.width);
            let offset = match align {
                Align::Left => 0.0,
                Align::Center => ((room - line_width) / 2.0).max(0.0),
                Align::Right => (room - line_width).max(0.0),
            };
            for placed in line {
                let word = placed.word;
                let x = left + offset + placed.x;
                let baseline = self.y + word.size;
                if let Some(color) = word.background
                    && let Some(rect) = Rect::from_xywh(x, baseline - word.size, placed.width, word.size * 1.25)
                {
                    self.current().push(Op::Fill { rect, color });
                }
                self.current().push(Op::Text {
                    at: Point::from_xy(x, baseline),
                    size: word.size,
                    face: word.face,
                    color: word.color,
                    text: word.text,
                    rtl: word.rtl,
                    italic: word.italic,
                });
                if word.underline {
                    let y = baseline + 1.6;
                    self.current().push(Op::Line {
                        from: Point::from_xy(x, y),
                        to: Point::from_xy(x + placed.width, y),
                        width: 0.5,
                        color: word.color,
                    });
                }
                if word.strike {
                    let y = baseline - word.size * 0.3;
                    self.current().push(Op::Line {
                        from: Point::from_xy(x, y),
                        to: Point::from_xy(x + placed.width, y),
                        width: 0.5,
                        color: word.color,
                    });
                }
            }
            self.y += height;
        }
    }

    fn image(&mut self, block: &Block, src: &str, asked_w: Option<u16>, asked_h: Option<u16>, left: f32, width: f32) {
        let Some((data, kind)) = picture_of(block, src, self.source) else {
            // No bytes: what the picture says it is stands in for it.
            self.put(self.wrap(self.styled_words(block, BODY, BODY * 1.4), width), width, block.align, left);
            return;
        };
        let Some((w, h)) = picture_size(&data, asked_w, asked_h) else {
            self.put(self.wrap(self.styled_words(block, BODY, BODY * 1.4), width), width, block.align, left);
            return;
        };
        // CSS pixels to points; nothing wider than the column, nothing taller than a page.
        let (mut w, mut h) = (w * 0.75, h * 0.75);
        if w > width {
            h *= width / w;
            w = width;
        }
        let room = self.floor() - self.paper.margin;
        if h > room {
            w *= room / h;
            h = room;
        }
        if w <= 0.5 || h <= 0.5 {
            return;
        }
        // Four pixels to the point of the room it is drawn in is as much as a printer sees; the
        // pages hold only so much picture in all, and the rest stands in as its words.
        let room = u64::from((w * h * 16.0).clamp(0.0, u32::MAX as f32).round() as u32);
        if self.pictures.saturating_add(room) > MAX_PICTURE_PIXELS_TOTAL {
            self.put(self.wrap(self.styled_words(block, BODY, BODY * 1.4), width), width, block.align, left);
            return;
        }
        self.pictures = self.pictures.saturating_add(room);
        self.need(h + 6.0);
        let x = match block.align {
            Align::Left => left,
            Align::Center => left + (width - w) / 2.0,
            Align::Right => left + width - w,
        };
        let Some(size) = Size::from_wh(w, h) else { return };
        let at = Point::from_xy(x, self.y);
        self.current().push(Op::Image { at, size, data, kind });
        self.y += h + 6.0;
    }

    fn table(&mut self, table: &Table, left: f32, width: f32, depth: u8) {
        let columns = usize::from(table.columns.min(64)).max(1);
        let room = match table.width {
            Some(mailcraft_html::Length::Px(px)) => f32::from(px) * 0.75,
            Some(mailcraft_html::Length::Percent(p)) => width * f32::from(p.min(100)) / 100.0,
            None => width,
        }
        .clamp(20.0, width);
        let column = room / columns as f32;
        // How many rows each column is still covered by, from a cell that spans several.
        let mut busy = vec![0u32; columns];
        for row in table.rows.iter().take(500) {
            let mut cells: Vec<(usize, f32, Vec<Op>, f32, Sides)> = Vec::new();
            let mut at = 0usize;
            for cell in row.cells.iter().take(columns * 4) {
                while at < columns && busy[at] > 0 {
                    at += 1;
                }
                if at >= columns {
                    break;
                }
                let span = usize::from(cell.colspan.max(1)).min(columns - at);
                let span_rows = u32::from(cell.rowspan.clamp(1, 50));
                for held in busy[at..at + span].iter_mut() {
                    *held = (*held).max(span_rows);
                }
                let cell_w = (column * span as f32 - 6.0).max(10.0);
                let (ops, height) = self.cell(cell, cell_w, depth);
                cells.push((at, cell_w, ops, height, cell.border));
                at += span;
            }
            if cells.is_empty() {
                continue;
            }
            let tallest = cells.iter().map(|c| c.3).fold(0.0f32, f32::max) + 8.0;
            self.need(tallest);
            let y0 = self.y;
            for (start, cell_w, ops, _, sides) in cells {
                let x = left + column * start as f32;
                let w = cell_w + 6.0;
                if table.outline || table.border || sides.any() {
                    let color = [0x99, 0x99, 0x99];
                    let thick = 0.5;
                    if table.border || sides.top {
                        self.current().push(Op::Line { from: Point::from_xy(x, y0), to: Point::from_xy(x + w, y0), width: thick, color });
                    }
                    if table.border || sides.bottom {
                        self.current().push(Op::Line {
                            from: Point::from_xy(x, y0 + tallest - 2.0),
                            to: Point::from_xy(x + w, y0 + tallest - 2.0),
                            width: thick,
                            color,
                        });
                    }
                    if table.border || sides.left {
                        self.current().push(Op::Line { from: Point::from_xy(x, y0), to: Point::from_xy(x, y0 + tallest - 2.0), width: thick, color });
                    }
                    if table.border || sides.right {
                        self.current().push(Op::Line {
                            from: Point::from_xy(x + w, y0),
                            to: Point::from_xy(x + w, y0 + tallest - 2.0),
                            width: thick,
                            color,
                        });
                    }
                }
                for mut op in ops {
                    shift(&mut op, x + 3.0, y0 + 4.0);
                    self.current().push(op);
                }
            }
            for c in &mut busy {
                *c = c.saturating_sub(1);
            }
            self.y += tallest;
        }
        self.y += 4.0;
    }

    /// A cell's blocks laid out in a room of its own: what to draw, and how tall it is.
    fn cell(&mut self, cell: &Cell, width: f32, depth: u8) -> (Vec<Op>, f32) {
        let mut inner =
            Flow { faces: self.faces.clone(), paper: self.paper, pages: vec![Vec::new()], y: 0.0, source: self.source, pictures: self.pictures };
        // A cell never breaks onto a new page: its paper is as tall as it needs to be.
        inner.paper.height = 1000.0;
        inner.paper.margin = 0.0;
        for block in cell.blocks.iter().take(200) {
            inner.block(block, depth + 1);
        }
        self.pictures = inner.pictures;
        let height = inner.y;
        let mut ops = inner.pages.into_iter().next().unwrap_or_default();
        if let Some(Rgb(r, g, b)) = cell.background
            && let Some(rect) = Rect::from_xywh(0.0, 0.0, width, height.max(1.0))
        {
            ops.push(Op::Fill { rect, color: [r, g, b] });
        }
        (ops, height)
    }
}

/// A block's picture: the bytes written into the message, or the caller's for its address.
fn picture_of(block: &Block, src: &str, source: Option<&dyn Fn(&str) -> Option<PictureBytes>>) -> Option<PictureBytes> {
    if let BlockKind::Image { data: Some(data), .. } = &block.kind {
        return Some((Arc::new(data.to_vec()), String::new()));
    }
    source.and_then(|source| source(src))
}

/// The picture's size in CSS pixels: what it is, or what the message asks for.
fn picture_size(data: &[u8], asked_w: Option<u16>, asked_h: Option<u16>) -> Option<(f32, f32)> {
    let natural = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if natural.0 > 0 && natural.1 > 0 {
        return Some((natural.0 as f32, natural.1 as f32));
    }
    match (asked_w, asked_h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((f32::from(w), f32::from(h))),
        _ => None,
    }
}

fn paint(pages: Vec<Vec<Op>>, faces: &[Face], paper: Paper, footer: &str) -> Result<Vec<u8>, Error> {
    let mut document = Document::new();
    let fonts: Vec<Option<Font>> = faces.iter().map(|face| Font::new(krilla::Data::from(Arc::clone(&face.data)), face.index)).collect();
    let total = pages.len();
    for (number, ops) in pages.into_iter().enumerate() {
        let Some(settings) = PageSettings::from_wh(paper.width, paper.height) else { continue };
        let mut page = document.start_page_with(settings);
        let mut surface = page.surface();
        for op in ops {
            match op {
                Op::Text { at, size, face, color, text, rtl, italic } => {
                    let Some(Some(font)) = fonts.get(face) else { continue };
                    surface.set_fill(Some(Fill { paint: rgb::Color::new(color[0], color[1], color[2]).into(), ..Default::default() }));
                    if italic {
                        // (A slant for a face that has no italic of its own: sheared about the word's
                        // own baseline, so it stays where it was put. krilla's `from_skew` takes the
                        // shear, an angle would lean the word off the page.)
                        surface.push_transform(&Transform::from_translate(0.0, at.y));
                        surface.push_transform(&Transform::from_skew(-ITALIC_SLANT, 0.0));
                        surface.push_transform(&Transform::from_translate(0.0, -at.y));
                        surface.draw_text(at, font.clone(), size, &text, false, direction(rtl));
                        surface.pop();
                        surface.pop();
                        surface.pop();
                    } else {
                        surface.draw_text(at, font.clone(), size, &text, false, direction(rtl));
                    }
                }
                Op::Line { from, to, width, color } => {
                    let mut builder = PathBuilder::new();
                    builder.move_to(from.x, from.y);
                    builder.line_to(to.x, to.y);
                    if let Some(path) = builder.finish() {
                        surface.set_fill(None);
                        surface.set_stroke(Some(Stroke { paint: rgb::Color::new(color[0], color[1], color[2]).into(), width, ..Default::default() }));
                        surface.draw_path(&path);
                    }
                }
                Op::Fill { rect, color } => {
                    let mut builder = PathBuilder::new();
                    builder.push_rect(rect);
                    if let Some(path) = builder.finish() {
                        surface.set_stroke(None);
                        surface.set_fill(Some(Fill { paint: rgb::Color::new(color[0], color[1], color[2]).into(), ..Default::default() }));
                        surface.draw_path(&path);
                    }
                }
                Op::Image { at, size, data, kind } => {
                    let _ = kind;
                    if let Some(image) = decode(data, (size.width(), size.height())) {
                        surface.push_transform(&Transform::from_translate(at.x, at.y));
                        surface.draw_image(image, size);
                        surface.pop();
                    }
                }
            }
        }
        if let Some(Some(font)) = fonts.first() {
            let text =
                if footer.trim().is_empty() { format!("{} / {total}", number + 1) } else { format!("{} · {} / {total}", footer.trim(), number + 1) };
            surface.set_fill(Some(Fill { paint: rgb::Color::new(0x77, 0x77, 0x77).into(), ..Default::default() }));
            surface.draw_text(Point::from_xy(paper.margin, paper.height - 22.0), font.clone(), 8.5, &text, false, TextDirection::LeftToRight);
        }
        surface.finish();
        page.finish();
    }
    document.finish().map_err(|why| Error(format!("the pages couldn't be written: {why:?}")))
}

fn direction(rtl: bool) -> TextDirection {
    if rtl { TextDirection::RightToLeft } else { TextDirection::LeftToRight }
}

/// The largest picture a page can hold, in bytes, in a side, and in pixels all told: nothing
/// printed is bigger than this, and a message or a server can name a picture of any size at all.
const MAX_PICTURE_BYTES: usize = 24 * 1024 * 1024;
const MAX_PICTURE_SIDE: u32 = 4_096;
const MAX_PICTURE_PIXELS: u64 = 16_000_000;
/// Most pixels of picture one printout holds: a message can name more pictures than any printer
/// could take, and a page is made of the bytes a caller hands over.
const MAX_PICTURE_PIXELS_TOTAL: u64 = 12_000_000;

/// Any picture the reader can open, as RGBA for the PDF, with no more pixels than the room it is
/// drawn in needs (a picture's decoder is somebody else's code reading hostile bytes: should it
/// panic, the picture is left out rather than taken as a crash). `box` is the room, in points.
fn decode(data: Arc<Vec<u8>>, room: (f32, f32)) -> Option<krilla::image::Image> {
    if data.len() > MAX_PICTURE_BYTES {
        return None;
    }
    let opened = std::panic::catch_unwind(|| {
        let reader = image::ImageReader::new(std::io::Cursor::new(data.as_ref())).with_guessed_format().ok()?;
        let (side, tall) = reader.into_dimensions().ok()?;
        if side == 0 || tall == 0 || side > MAX_PICTURE_SIDE || tall > MAX_PICTURE_SIDE || u64::from(side) * u64::from(tall) > MAX_PICTURE_PIXELS {
            return None;
        }
        let reader = image::ImageReader::new(std::io::Cursor::new(data.as_ref())).with_guessed_format().ok()?;
        reader.decode().ok()
    })
    .ok()??;
    let rgba = opened.into_rgba8();
    // Four pixels to the point is more than a printer can use; more than that is thrown away.
    let wanted = ((room.0 * 4.0).clamp(1.0, 1_000_000.0) as u32, (room.1 * 4.0).clamp(1.0, 1_000_000.0) as u32);
    let rgba = if rgba.width() > wanted.0 || rgba.height() > wanted.1 { image::imageops::thumbnail(&rgba, wanted.0, wanted.1) } else { rgba };
    let (width, height) = (rgba.width(), rgba.height());
    if width == 0 || height == 0 {
        return None;
    }
    Some(krilla::image::Image::from_rgba8(rgba.into_vec(), width, height))
}
