// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing in rich text: the selection in the message being written, typing, formatting and
//! undo inside the message (`mailcraft_model::rich`).
//!
//! These are pure commands: they change the message being written, never the mailbox, so they
//! are not on the mailbox's undo history. The message has its own (`compose.undo`). While a
//! signature is open in File › Signatures they work on it instead ([`Session::doc`]).
//! Positions count characters of the body's text, a paragraph break being one.

use mailcraft_model::rich::{
    Align, CellAlignment, CellText, Font, Format, ListKind, MAX_PASTED_COLUMNS, MAX_PASTED_ROWS, MAX_SIZE, MIN_SIZE, Paragraph, PastedTable, Run,
    Style, TableCell, safe_color,
};
use mailcraft_model::{ModelError, RichBody, RichText};
use serde_json::{Value, json};

use super::{bad, fail, needs_compose};
use crate::Doc;
use crate::session::{enum_code, parse_enum};
use crate::{BodyFormat, CmdError, CmdResult, CommandSpec, Compose, Session, p};

const RANGE: &str = r#"{"from"?: number, "to"?: number} (default: the selection)"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("compose.select", "Select Text", "Message", select).pure().params(r#"{"from": number, "to"?: number} (to: the caret, default from)"#).when_writing(needs_doc),
        CommandSpec::new("compose.type", "Type", "Message", type_text)
            .pure()
            .params(r#"{"text": string, "autoList"?: bool} — over the selection, in the formatting at the caret; a line break starts a paragraph (of the same kind; a line break alone in an empty list item ends the list there). "autoList" (the keyboard's, with a space): when the paragraph so far is `* `, `- `, `1. ` or `1) ` in an HTML message and File › Options has Automatic bulleted and numbered lists on, it becomes a list item without the marker, an undo step of its own."#)
            .when_writing(needs_doc),
        CommandSpec::new("compose.replaceText", "Replace Text at Position", "Message", replace_text)
            .pure()
            .params(r#"{"from": number, "to": number, "expected": string, "text": string} — replace the expected text at [from, to) as one undo step, preserving the first character's style. Each string is on one line and at most 255 characters. The caret moves to the end of the replacement, and a stale range or different text leaves the body alone"#)
            .when_writing(needs_doc),
        CommandSpec::new("compose.delete", "Delete Text", "Message", delete)
            .pure()
            .params(r#"{"from"?: number, "to"?: number, "forward"?: bool} (default: the selection, else the character before the caret, or after it with forward)"#)
            .when_writing(needs_doc),
        CommandSpec::new("compose.undo", "Undo Typing", "Message", undo).pure().when_writing(|s| match s.doc() {
            None => Some("no message is being written"),
            Some((b, _)) if !b.can_undo() => Some("nothing to undo in the message"),
            Some(_) => None,
        }),
        CommandSpec::new("compose.redo", "Redo Typing", "Message", redo).pure().when_writing(|s| match s.doc() {
            None => Some("no message is being written"),
            Some((b, _)) if !b.can_redo() => Some("nothing to redo in the message"),
            Some(_) => None,
        }),
        CommandSpec::new("format.bold", "Bold", "Message › Basic Text", bold).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("format.italic", "Italic", "Message › Basic Text", italic).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("format.underline", "Underline", "Message › Basic Text", underline).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("format.size", "Font Size", "Message › Basic Text", size)
            .pure()
            .params(r#"{"size": number (points, 8–36) | null (the message's own size), "from"?: number, "to"?: number}"#)
            .when_writing(needs_html),
        CommandSpec::new("format.color", "Font Color", "Message › Basic Text", color)
            .pure()
            .params(r##"{"value": "#rrggbb" | null (the message's own colour), "from"?: number, "to"?: number}"##)
            .when_writing(needs_html),
        CommandSpec::new("format.highlight", "Text Highlight Color", "Message › Basic Text", highlight)
            .pure()
            .params(r##"{"value": "#rrggbb" | null (no highlight), "from"?: number, "to"?: number}"##)
            .when_writing(needs_html),
        CommandSpec::new("insert.symbol", "Symbol", "Insert › Symbols", symbol)
            .pure()
            .params(r#"{"char": string (one character)} | {"code": number | "U+20AC"} — put in at the caret, over the selection; a control or format character, a private-use or non-character code point, or white space other than the no-break space is refused"#)
            .when_writing(needs_doc),
        CommandSpec::new("format.font", "Font", "Message › Basic Text", font)
            .pure()
            .params(r#"{"value": string (a font family, as it is named on this computer — the window offers what is installed; "ui.fonts" over the control port lists them; a family that isn't installed is drawn in the window's own font), or null (the message's own font), "from"?: number, "to"?: number}"#)
            .when_writing(needs_html),
        CommandSpec::new("format.bullets", "Bullets", "Message › Basic Text", bullets).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("format.numbering", "Numbering", "Message › Basic Text", numbering).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("format.align", "Align Text", "Message › Basic Text", align)
            .pure()
            .params(r#"{"value": "left"|"center"|"right", "from"?: number, "to"?: number} — every ordinary paragraph the range (default: the selection) touches; a list item stays left. Kept when sent (text-align) and on a paste, but not for a blank line; a plain-text message has none. Answers {"value"}: the alignment where the range starts. The Commands list doesn't offer it, having no way to say which"#)
            .when_writing(needs_html),
        CommandSpec::new("format.clear", "Clear All Formatting", "Message › Basic Text", clear).pure().params(RANGE).when_writing(needs_html),
        CommandSpec::new("compose.link", "Link", "Message › Include", link)
            .pure()
            .params(r#"{"url"?: string (any URI scheme but javascript, vbscript, data, file and cid, e.g. https, mailto, tel, ms-settings; none: remove the link), "from"?: number, "to"?: number}"#)
            .when_writing(needs_html),
        CommandSpec::new("format.indent", "Increase Indent", "Message › Basic Text", indent)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — the list items there a level deeper (eight levels)"#)
            .when_writing(needs_html),
        CommandSpec::new("format.outdent", "Decrease Indent", "Message › Basic Text", outdent)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — the list items there a level out; from the outermost level, out of the list"#)
            .when_writing(needs_html),
        CommandSpec::new("edit.copy", "Copy", "Message › Clipboard", copy)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — answers the selection as {"html", "text"} for the clipboard (HTML only in an HTML message); changes nothing"#)
            .when_writing(needs_doc),
        CommandSpec::new("edit.cut", "Cut", "Message › Clipboard", cut)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — answers the selection as edit.copy does, and takes it out of the text"#)
            .when_writing(needs_doc),
        CommandSpec::new("edit.paste", "Paste", "Message › Clipboard", paste)
            .pure()
            .params(r#"{"html"?: string, "spreadsheet"?: string (Excel's "XML Spreadsheet" clipboard data, made into a table), "text"?: string, "pictures"?: [{"url": string, "data": base64, "file"?: string, "alt"?: string}] (a signature: the pictures of its pasted HTML, downloaded beforehand, each put where its `url` stands in the HTML)} — over the selection; supported HTML formatting includes bold, italic, underline, font, size, colours, links, lists and a paragraph's indent and alignment. The HTML composer and a signature keep bounded tables as selectable objects. Tables flatten to text in plain-text messages. HTML that comes to no text gives way to `text`; text alone that is a spreadsheet's Copy (two or more rows of lines with the same number of tab-separated cells) becomes a table in an HTML message. Nothing to paste changes nothing"#)
            .when_writing(needs_doc),
        CommandSpec::new("table.cellEdit", "Edit Cell", "Insert › Tables", cell_edit)
            .pure()
            .params(r#"{"from": number (the table's place in the text), "row": number, "cell": number (both 0-based), "at"?: number (the character in the cell; default 0)} — the owner types in that cell: what follows is table.cellType, table.cellErase and table.cellEnter till table.cellLeave"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellSelect", "Select in Cell", "Insert › Tables", cell_select)
            .pure()
            .params(r#"{"from": number (the table's place in the text), "row": number, "cell": number (both 0-based), "anchor": number, "at": number} — select the characters of that cell between two places in it (both clamped to what it holds)"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellSelectAll", "Select the Cell", "Insert › Tables", cell_select_all)
            .pure()
            .when_writing(needs_html),
        CommandSpec::new("table.cellCopy", "Copy in Cell", "Insert › Tables", cell_copy)
            .pure()
            .params(r#"{} — the selected characters of the cell being edited, as {"html", "text"} for the clipboard"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellCut", "Cut in Cell", "Insert › Tables", cell_cut)
            .params(r#"{} — as table.cellCopy, and takes it out of the cell"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellType", "Type in Cell", "Insert › Tables", cell_type)
            .params(r#"{"text": string} — put in at the caret in the cell being edited, a `\n` starting a new line"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellPaste", "Paste in Cell", "Insert › Tables", cell_paste)
            .params(r#"{"html"?: string, "text"?: string} — paste into the cell being edited: its HTML formatting (bold, italic, underline, colours, fonts, sizes, links) is kept, and its paragraphs are its lines; with no HTML, `text` is put in the cell's own style"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellPicture", "Picture in Cell", "Insert › Tables", cell_picture)
            .params(r#"{"data": base64, "file"?: string, "alt"?: string} — put a picture above the text of the cell being edited (the cell's text, when there is some, stands under it)"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellErase", "Erase in Cell", "Insert › Tables", cell_erase)
            .params(r#"{"forward"?: bool (the character after the caret; else the one before), "to"?: number (erase from the caret to there)}"#)
            .when_writing(needs_html),
        CommandSpec::new("table.cellEnter", "New Line in Cell", "Insert › Tables", cell_enter).when_writing(needs_html),
        CommandSpec::new("table.cellLeave", "Leave Cell", "Insert › Tables", cell_leave).pure().when_writing(needs_html),
        CommandSpec::new("table.cellBorders", "Cell Borders", "Insert › Tables", cell_borders)
            .pure()
            .params(r#"{"from": number (the table's place in the text), "row": number, "cell": number (both 0-based)} — the sides that cell has a line on: {"top", "right", "bottom", "left"}"#)
            .when_writing(needs_html),
        CommandSpec::new("table.borders", "Cell Borders", "Insert › Tables", table_borders)
            .params(r#"{"from": number (the table's place in the text), "row": number, "cell": number (both 0-based), "top"?: bool, "right"?: bool, "bottom"?: bool, "left"?: bool} — the sides that cell has a line on; the rest stay as they are"#)
            .when_writing(needs_html),
        CommandSpec::new("insert.table", "Table", "Insert › Tables", insert_table)
            .pure()
            .params(r#"{"rows"?: number, "columns"?: number} (default: 3 rows, 3 columns) — an empty grid at the caret, in an HTML message"#)
            .when_writing(needs_html),
        CommandSpec::new("compose.formatHtml", "HTML", "Format Text › Format", format_html).pure().when_writing(needs_compose),
        CommandSpec::new("compose.formatPlain", "Plain Text", "Format Text › Format", format_plain).pure().when_writing(needs_compose),
    ]
}

/// Enabled while a message is being written or a signature is open.
pub(super) fn needs_doc(s: &Session) -> Option<&'static str> {
    if s.doc().is_none() { Some("no message is being written") } else { None }
}

/// Enabled only while a message in HTML is being written (or a signature is open).
pub(super) fn needs_html(s: &Session) -> Option<&'static str> {
    match s.doc() {
        None => Some("no message is being written"),
        Some((_, true)) => Some("the message is plain text; switch it to HTML first"),
        Some(_) => None,
    }
}

fn compose_mut(s: &mut Session) -> Result<&mut Compose, CmdError> {
    s.compose.as_mut().ok_or_else(|| fail("no message is being written"))
}

fn doc_mut(s: &mut Session) -> Result<Doc<'_>, CmdError> {
    s.doc_mut().ok_or_else(|| fail("no message is being written"))
}

fn model(e: ModelError) -> CmdError {
    fail(e.to_string())
}

/// A position from the params.
pub(super) fn position(v: &Value, key: &str) -> Result<Option<usize>, CmdError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => p::u64(v, key).and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| bad(format!("`{key}` is a position (a number)"))),
    }
}

/// Where the text the owner edits ends in the message being written (or the signature open): its
/// end. Find, Replace and the formats keep within it.
pub(super) fn reach(s: &Session) -> usize {
    s.doc().map_or(0, |(b, _)| b.text().len())
}

/// Select what `from` and `to` name, when the params name something, within `reach`.
fn select_range(body: &mut RichBody, v: &Value, reach: usize) -> Result<(), CmdError> {
    let (from, to) = (position(v, "from")?, position(v, "to")?);
    if from.is_none() && to.is_none() {
        return Ok(());
    }
    let sel = body.selection();
    let from = from.unwrap_or(sel.start());
    let to = to.unwrap_or(from);
    let len = body.text().len();
    // Past the end is an error as ever; past the owner's text, only as far as it goes.
    if from > len || to > len {
        return Err(model(ModelError::Invalid(format!("the text has {len} characters"))));
    }
    body.select(from.min(reach), to.min(reach)).map_err(model)
}

/// Run an edit of the text; the message (or signature) is changed when its text is.
pub(super) fn edit<T>(s: &mut Session, v: &Value, f: impl FnOnce(&mut RichBody) -> Result<T, ModelError>) -> Result<T, CmdError> {
    let reach = reach(s);
    let d = doc_mut(s)?;
    select_range(d.body, v, reach)?;
    let before = d.body.text().clone();
    let out = f(d.body).map_err(model)?;
    if *d.body.text() != before {
        *d.dirty = true;
    }
    Ok(out)
}

/// What the answer of an edit says: the selection now.
pub(super) fn state(s: &Session) -> Value {
    match s.doc() {
        Some((body, _)) => {
            let sel = body.selection();
            json!({"from": sel.anchor, "to": sel.head, "length": body.text().len()})
        }
        None => json!({}),
    }
}

fn select(s: &mut Session, v: &Value) -> CmdResult {
    let from = position(v, "from")?.ok_or_else(|| bad("`from` (number) is required"))?;
    let to = position(v, "to")?.unwrap_or(from);
    doc_mut(s)?.body.select(from, to).map_err(model)?;
    Ok(state(s))
}

fn type_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?.to_string();
    edit(s, &json!({}), |b| b.type_text(&text))?;
    // Lists as you type (#973): only after a space, in HTML, when the owner leaves them on.
    if text == " " && p::bool(v, "autoList").unwrap_or(false) && s.prefs.auto_lists && matches!(s.doc(), Some((_, false))) {
        edit(s, &json!({}), |b| b.auto_list())?;
    }
    Ok(state(s))
}

fn replace_text(s: &mut Session, v: &Value) -> CmdResult {
    let from = position(v, "from")?.ok_or_else(|| bad("`from` (number) is required"))?;
    let to = position(v, "to")?.ok_or_else(|| bad("`to` (number) is required"))?;
    let expected = p::req_str(v, "expected")?;
    let text = p::req_str(v, "text")?;
    edit(s, &json!({}), |body| body.replace_exact((from, to), expected, text))?;
    Ok(state(s))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let forward = p::bool(v, "forward").unwrap_or(false);
    let named = position(v, "from")?.is_some() || position(v, "to")?.is_some();
    edit(s, v, |b| {
        let sel = b.selection();
        if named || !sel.is_empty() {
            b.delete(sel.start(), sel.end())
        } else if forward {
            b.delete_forward()
        } else {
            b.backspace()
        }
    })?;
    Ok(state(s))
}

fn undo(s: &mut Session, _: &Value) -> CmdResult {
    if !step(s, false)? {
        return Err(fail("nothing to undo in the message"));
    }
    Ok(state(s))
}

fn redo(s: &mut Session, _: &Value) -> CmdResult {
    if !step(s, true)? {
        return Err(fail("nothing to redo in the message"));
    }
    Ok(state(s))
}

/// One step back (or forward, `redo`) in the open text; a step of the message's can put a
/// picture's width back (`compose.picture.set`). Whether there was one.
fn step(s: &mut Session, redo: bool) -> Result<bool, CmdError> {
    if s.signature_edit.is_none()
        && let Some(c) = s.compose.as_mut()
    {
        let pictures = &mut c.pictures;
        let swap = |mark| super::picture::swap_width(pictures, mark);
        let done = if redo { c.body.redo_with(swap) } else { c.body.undo_with(swap) };
        c.dirty |= done;
        return Ok(done);
    }
    let d = doc_mut(s)?;
    let done = if redo { d.body.redo() } else { d.body.undo() };
    *d.dirty |= done;
    Ok(done)
}

fn bold(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        let on = cell_toggle(s, &|style| style.bold, Format::Bold(true), Format::Bold(false))?;
        return Ok(json!({"bold": on}));
    }
    let on = edit(s, v, |b| b.toggle_bold())?;
    Ok(json!({"bold": on}))
}

fn italic(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        let on = cell_toggle(s, &|style| style.italic, Format::Italic(true), Format::Italic(false))?;
        return Ok(json!({"italic": on}));
    }
    let on = edit(s, v, |b| b.toggle_italic())?;
    Ok(json!({"italic": on}))
}

fn underline(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        let on = cell_toggle(s, &|style| style.underline, Format::Underline(true), Format::Underline(false))?;
        return Ok(json!({"underline": on}));
    }
    let on = edit(s, v, |b| b.toggle_underline())?;
    Ok(json!({"underline": on}))
}

fn color(s: &mut Session, v: &Value) -> CmdResult {
    let value = match v.get("value") {
        Some(Value::Null) => None,
        Some(_) => Some(p::str(v, "value").and_then(safe_color).ok_or_else(|| bad("`value` is a colour written #rrggbb, or null"))?),
        None => return Err(bad("`value` (#rrggbb, or null) is required")),
    };
    if cell_target(s, v).is_some() {
        cell_apply(s, Format::Color(value.clone()))?;
        return Ok(json!({"value": value}));
    }
    edit(s, v, |b| b.format(Format::Color(value.clone())))?;
    Ok(json!({"value": value}))
}

fn highlight(s: &mut Session, v: &Value) -> CmdResult {
    let value = match v.get("value") {
        Some(Value::Null) => None,
        Some(_) => Some(p::str(v, "value").and_then(safe_color).ok_or_else(|| bad("`value` is a colour written #rrggbb, or null"))?),
        None => return Err(bad("`value` (#rrggbb, or null) is required")),
    };
    if cell_target(s, v).is_some() {
        cell_apply(s, Format::Highlight(value.clone()))?;
        return Ok(json!({"value": value}));
    }
    edit(s, v, |b| b.format(Format::Highlight(value.clone())))?;
    Ok(json!({"value": value}))
}

/// Symbols Insert › Symbol offers: common marks the fonts the window draws in all have.
pub const SYMBOLS: [char; 36] = [
    '©', '®', '™', '€', '£', '¥', '¢', '§', '¶', '°', '±', '×', '÷', '≠', '≤', '≥', '≈', '∞', 'µ', '½', '¼', '¾', '…', '–', '—', '•', '·', '←', '→',
    '↑', '↓', '«', '»', '¿', '¡', '\u{a0}',
];

/// Whether `c` may be put in as a symbol: something that shows (or the no-break space), not a
/// control or format character, a private-use code point, a non-character or the character a
/// picture stands as.
pub fn symbol_ok(c: char) -> bool {
    let n = u32::from(c);
    let private = (0xe000..=0xf8ff).contains(&n) || n >= 0xf_0000;
    let nonchar = (0xfdd0..=0xfdef).contains(&n) || n & 0xfffe == 0xfffe;
    !(c.is_control()
        || mailcraft_names::is_format(c)
        || private
        || nonchar
        || c == mailcraft_model::rich::OBJECT
        || (c.is_whitespace() && c != '\u{a0}'))
}

/// The symbol the params name.
fn symbol_of(v: &Value) -> Result<char, CmdError> {
    let bad_code = || bad("`code` is a Unicode code point, a number or written U+20AC");
    let c = match (v.get("char"), v.get("code")) {
        (Some(Value::String(t)), None) => {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => return Err(bad("`char` is one character")),
            }
        }
        (None, Some(Value::Number(_))) => p::u64(v, "code").and_then(|n| u32::try_from(n).ok()).and_then(char::from_u32).ok_or_else(bad_code)?,
        (None, Some(Value::String(t))) => {
            let hex = t.trim();
            let hex = hex.strip_prefix("U+").or_else(|| hex.strip_prefix("u+")).unwrap_or(hex);
            if hex.is_empty() || hex.len() > 6 {
                return Err(bad_code());
            }
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).ok_or_else(bad_code)?
        }
        _ => return Err(bad("`char` (one character) or `code` (a code point) is required")),
    };
    if !symbol_ok(c) {
        return Err(bad("that character can't be put in: it is a control or format character, white space, or shows nothing"));
    }
    Ok(c)
}

fn symbol(s: &mut Session, v: &Value) -> CmdResult {
    let c = symbol_of(v)?;
    edit(s, &json!({}), |b| b.type_text(&c.to_string()))?;
    let mut out = state(s);
    if let Some(o) = out.as_object_mut() {
        o.insert("char".into(), json!(c.to_string()));
        o.insert("code".into(), json!(format!("U+{:04X}", u32::from(c))));
    }
    Ok(out)
}

fn font(s: &mut Session, v: &Value) -> CmdResult {
    let value = match v.get("value") {
        Some(Value::Null) => None,
        // Any family the owner has installed, named by its own name: the window lists what this
        // computer has (`FontIndex`), and this only checks the shape of the name — the engine
        // never looks at the fonts folders, and a font that isn't installed here is drawn in the
        // window's own and named to the recipient all the same.
        Some(_) => Some(p::str(v, "value").and_then(Font::parse).ok_or_else(|| bad("`value` is a font family, or null"))?),
        None => return Err(bad("`value` (a font, or null) is required")),
    };
    if cell_target(s, v).is_some() {
        cell_apply(s, Format::Font(value.clone()))?;
        return Ok(json!({"value": value.as_ref().map(|f| f.label().to_string())}));
    }
    edit(s, v, |b| b.format(Format::Font(value.clone())))?;
    Ok(json!({"value": value.as_ref().map(|f| f.label().to_string())}))
}

fn size(s: &mut Session, v: &Value) -> CmdResult {
    let size = match v.get("size") {
        Some(Value::Null) => None,
        Some(_) => {
            let n = p::u64(v, "size").filter(|n| (u64::from(MIN_SIZE)..=u64::from(MAX_SIZE)).contains(n));
            Some(
                n.and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| bad(format!("`size` is a number of points, {MIN_SIZE} to {MAX_SIZE}, or null")))?,
            )
        }
        None => return Err(bad("`size` (points, or null) is required")),
    };
    if cell_target(s, v).is_some() {
        cell_apply(s, Format::Size(size))?;
        return Ok(json!({"size": size}));
    }
    edit(s, v, |b| b.format(Format::Size(size)))?;
    Ok(json!({"size": size}))
}

fn list(s: &mut Session, v: &Value, kind: ListKind) -> CmdResult {
    if cell_target(s, v).is_some() {
        return Err(fail("a table cell holds lines of text, not a list"));
    }
    let now = edit(s, v, |b| b.toggle_list(kind))?;
    Ok(json!({"list": now}))
}

fn bullets(s: &mut Session, v: &Value) -> CmdResult {
    list(s, v, ListKind::Bullet)
}

fn numbering(s: &mut Session, v: &Value) -> CmdResult {
    list(s, v, ListKind::Numbered)
}

fn align(s: &mut Session, v: &Value) -> CmdResult {
    let align = p::str(v, "value").and_then(parse_enum::<Align>).ok_or_else(|| bad("`value` is left, center or right"))?;
    if let Some((place, _, _)) = cell_target(s, v) {
        // A cell is aligned whole: its lines are one block in the grid.
        let cell_alignment = match align {
            Align::Left => CellAlignment::Left,
            Align::Center => CellAlignment::Center,
            Align::Right => CellAlignment::Right,
        };
        edit(s, &json!({"from": place.from}), |body| {
            body.edit_text(|text| {
                let table = text.table_at_mut(place.from).ok_or_else(|| ModelError::Invalid("there is no table there".into()))?;
                if table.set_cell_alignment(place.row, place.cell, cell_alignment) {
                    Ok(())
                } else {
                    Err(ModelError::Invalid("there is no such cell in that table".into()))
                }
            })
        })?;
        return Ok(json!({"value": enum_code(&cell_alignment)}));
    }
    edit(s, v, |b| b.set_align(align))?;
    Ok(json!({"value": s.doc().map(|(b, _)| enum_code(&b.text().align_at(b.selection().start())))}))
}

fn clear(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        cell_apply(s, Format::Clear)?;
        return Ok(state(s));
    }
    edit(s, v, |b| b.clear_formatting())?;
    Ok(state(s))
}

fn link(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        return Err(fail("a link is put on the text of the message, not inside a table cell"));
    }
    let url = match v.get("url") {
        None | Some(Value::Null) => None,
        Some(Value::String(u)) => Some(u.clone()),
        Some(_) => return Err(bad("`url` is a string")),
    };
    edit(s, v, |b| b.set_link(url.as_deref()))?;
    Ok(state(s))
}

fn indent(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        return Err(fail("a table cell holds no list to indent"));
    }
    edit(s, v, |b| b.indent(1))?;
    Ok(json!({"level": s.doc().map_or(0, |(b, _)| b.text().level_at(b.selection().start()))}))
}

fn outdent(s: &mut Session, v: &Value) -> CmdResult {
    if cell_target(s, v).is_some() {
        return Err(fail("a table cell holds no list to outdent"));
    }
    edit(s, v, |b| b.indent(-1))?;
    let d = s.doc();
    Ok(json!({
        "level": d.map_or(0, |(b, _)| b.text().level_at(b.selection().start())),
        "list": d.map(|(b, _)| b.text().list_at(b.selection().start())),
    }))
}

/// The selection (after the params' range, when they name one) as the clipboard takes it: HTML
/// of the set, written as the message's own HTML part is, and its plain text.
fn clipboard(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let reach = reach(s);
    let d = doc_mut(s)?;
    select_range(d.body, v, reach)?;
    let sel = d.body.selection();
    if sel.is_empty() {
        return Err(fail("nothing is selected in the message"));
    }
    let part = d.body.text().slice(sel.start(), sel.end()).map_err(model)?;
    let html = (!d.plain).then(|| mailcraft_mime_model::rich::to_html(&part));
    Ok(json!({"text": part.to_plain(), "html": html}))
}

fn copy(s: &mut Session, v: &Value) -> CmdResult {
    clipboard(s, v)
}

fn cut(s: &mut Session, v: &Value) -> CmdResult {
    let out = clipboard(s, v)?;
    edit(s, &json!({}), |b| {
        let sel = b.selection();
        b.delete(sel.start(), sel.end())
    })?;
    Ok(out)
}

/// Most HTML or text one paste takes, in bytes.
pub(crate) const MAX_PASTE: usize = 4 << 20;

fn paste(s: &mut Session, v: &Value) -> CmdResult {
    let plain = s.doc().is_some_and(|(_, plain)| plain);
    let (html, text) = (p::str(v, "html"), p::str(v, "text"));
    // Excel's own clipboard data, when there is no HTML beside it: its cells made into a table
    // (a range copied on Windows carries both, but a copy may carry only this).
    let html = match html {
        Some(html) => Some(html.to_owned()),
        None => match p::str(v, "spreadsheet") {
            Some(xml) if xml.len() <= MAX_PASTE => super::spreadsheet::to_html(xml),
            Some(_) => return Err(fail("that is too much to paste into one message")),
            None => None,
        },
    };
    if html.is_none() && text.is_none() {
        return Err(bad("`html`, `spreadsheet` or `text` (string) is required"));
    }
    let too_much = || fail("that is too much to paste into one message");
    // A signature's pictures, downloaded by the window before this (the engine asks no server):
    // each is put in the signature, and stands in the pasted text where its address was. One
    // that isn't a picture, or is over the signature's caps, is left where it was not.
    let pasted_pictures = v.get("pictures").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut by_url = std::collections::HashMap::new();
    if !pasted_pictures.is_empty()
        && let Some(open) = s.signature_edit.as_mut()
        && open.signature.html().is_none()
    {
        let mut sig = open.signature.clone();
        sig.set_body(open.body.text().clone()).map_err(model)?;
        for pic in &pasted_pictures {
            let Some(url) = p::str(pic, "url") else { continue };
            let (_, bytes) = super::source(pic)?;
            let file = p::str(pic, "file").unwrap_or("picture");
            if let Ok(id) = sig.add_picture(file, bytes, p::str(pic, "alt"), None) {
                by_url.insert(url.to_string(), id);
            }
        }
        open.signature = sig;
        open.dirty = true;
    }
    let from_html = match html {
        Some(html) if html.len() > MAX_PASTE => return Err(too_much()),
        Some(html) => {
            // A table the HTML holds stays an object to type in, in a message and in a
            // signature alike (the cells of a pasted signature are the owner's to edit).
            let rich = if plain {
                mailcraft_mime_model::rich::from_html(&html)
            } else if by_url.is_empty() {
                mailcraft_mime_model::rich::from_html_for_compose(&html)
            } else {
                mailcraft_mime_model::rich::from_html_with_pictures_any(&html, &mut |img| by_url.get(&img.cid).copied())
            };
            Some(if plain { RichText::from_plain(&rich.text()) } else { rich }).filter(|rich| !rich.is_empty())
        }
        None => None,
    };
    let fragment = match (from_html, text) {
        (Some(rich), _) => rich,
        (None, Some(text)) if text.len() > MAX_PASTE => return Err(too_much()),
        (None, Some(text)) => pasted_text(text, plain, s.signature_edit.is_some()),
        (None, None) => return Ok(state(s)),
    };
    if fragment.is_empty() {
        return Ok(state(s));
    }
    edit(s, &json!({}), |b| b.paste(&fragment))?;
    let mut out = state(s);
    // A signature's pictures come from files or from the window's downloads; when neither brought
    // any, say so for the ones the HTML had.
    let left_out = pasted_pictures.is_empty() && p::str(v, "html").is_some_and(|h| h.to_ascii_lowercase().contains("<img"));
    if left_out
        && s.signature_edit.is_some()
        && let Some(o) = out.as_object_mut()
    {
        o.insert("note".into(), json!("pictures in what was pasted were left out: save each one and add it with Insert Picture"));
    }
    Ok(out)
}

/// Text with no HTML beside it: a spreadsheet's Copy — rows of lines, the cells of a row between
/// tabs — becomes a table in an HTML message, so the grid the owner copied stays a grid. Anything
/// else, and anything into plain text or a signature, is the text as it came.
fn pasted_text(text: &str, plain: bool, signature: bool) -> RichText {
    if plain || signature {
        return RichText::from_plain(text);
    }
    match PastedTable::from_text(text) {
        Some(table) => RichText::from_paragraphs(vec![Paragraph::new(vec![Run::table(std::sync::Arc::new(table))], ListKind::None)]),
        None => RichText::from_plain(text),
    }
}

/// Where in a table cell the owner types.
fn cell_edit(s: &mut Session, v: &Value) -> CmdResult {
    let (from, row, cell) = cell_of(s, v)?;
    let at = p::u64(v, "at").and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let edit = s.set_table_edit(from, row, cell, at).ok_or_else(|| fail("there is no such cell in that table"))?;
    Ok(cell_caret(s, edit))
}

fn cell_type(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").ok_or_else(|| bad("`text` (string) is required"))?.to_string();
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    // What the owner chose for what they type next (a bold button with no text selected, say),
    // else the style where the caret is.
    let style = s.table_pending.clone().or_else(|| style_in_table(s, edit)).unwrap_or_default();
    let advance = text.chars().count();
    cell_write(
        s,
        move |table, edit, lo, hi| table.cell_erase(edit.row, edit.cell, lo, hi) && table.cell_insert_styled(edit.row, edit.cell, lo, &text, &style),
        advance,
    )
}

/// The style of the character where the caret is in the cell being edited.
fn style_in_table(s: &Session, edit: crate::session::TableEdit) -> Option<Style> {
    s.doc()?.0.text().table_at(edit.from)?.cell_style_at(edit.row, edit.cell, edit.at)
}

/// Paste into the cell being edited: the clipboard's HTML, with its formatting, or plain text in
/// the style the cell has at the caret.
fn cell_paste(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::str(v, "text").unwrap_or_default().to_string();
    let Some(html) = p::str(v, "html").filter(|html| !html.trim().is_empty()) else {
        let style = s.table_edit_now().and_then(|edit| s.table_pending.clone().or_else(|| style_in_table(s, edit))).unwrap_or_default();
        let advance = text.chars().count();
        return cell_write(
            s,
            move |table, edit, lo, hi| {
                table.cell_erase(edit.row, edit.cell, lo, hi) && table.cell_insert_styled(edit.row, edit.cell, lo, &text, &style)
            },
            advance,
        );
    };
    let rich = mailcraft_mime_model::rich::from_html_for_compose(html);
    let rich = if rich.is_empty() { RichText::from_plain(&text) } else { rich };
    let advance = rich.len();
    cell_write(
        s,
        move |table, edit, lo, hi| table.cell_erase(edit.row, edit.cell, lo, hi) && table.cell_insert_rich(edit.row, edit.cell, lo, &rich),
        advance,
    )
}

/// Put a picture above the text of the cell being edited: in the signature open or the message
/// being written, its pictures are the ones its text shows.
fn cell_picture(s: &mut Session, v: &Value) -> CmdResult {
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let path = p::str(v, "path").map(str::trim).unwrap_or_default();
    if p::str(v, "data").is_none() && ["http:", "https:", "ftp:", "//"].iter().any(|scheme| path.to_ascii_lowercase().starts_with(scheme)) {
        return Err(fail("pictures from the web are not downloaded: save the picture and add it from the file"));
    }
    let (_, bytes) = super::source(&json!({"data": v.get("data"), "path": v.get("path")}))?;
    let file = p::str(v, "file").or_else(|| (!path.is_empty()).then(|| path.rsplit(['/', '\\']).next().unwrap_or(path))).unwrap_or("picture");
    let alt = p::str(v, "alt");
    let id = if let Some(e) = s.signature_edit.as_mut() {
        let mut sig = e.signature.clone();
        sig.set_body(e.body.text().clone()).map_err(model)?;
        let id = sig.store_picture(file, bytes, alt).map_err(model)?;
        e.signature = sig;
        if e.body
            .edit_text(|text| {
                let table = text.table_at_mut(edit.from).ok_or_else(|| ModelError::Invalid("there is no such cell in that table".into()))?;
                if table.set_cell_image(edit.row, edit.cell, Some(id)) {
                    Ok(())
                } else {
                    Err(ModelError::Invalid("there is no such cell in that table".into()))
                }
            })
            .is_err()
        {
            return Err(fail("there is no such cell in that table"));
        }
        e.dirty = true;
        id
    } else if let Some(c) = s.compose.as_mut() {
        let held: std::collections::HashSet<u32> = c.body.text().pictures().into_iter().collect();
        let id = mailcraft_model::pictures::add(&mut c.pictures, &mut c.picture_seq, &held, file, bytes, alt, None).map_err(model)?;
        if c.body
            .edit_text(|text| {
                let table = text.table_at_mut(edit.from).ok_or_else(|| ModelError::Invalid("there is no such cell in that table".into()))?;
                if table.set_cell_image(edit.row, edit.cell, Some(id)) {
                    Ok(())
                } else {
                    Err(ModelError::Invalid("there is no such cell in that table".into()))
                }
            })
            .is_err()
        {
            return Err(fail("there is no such cell in that table"));
        }
        c.dirty = true;
        id
    } else {
        return Err(fail("no message is being written"));
    };
    Ok(json!({"id": id, "from": edit.from, "row": edit.row, "cell": edit.cell}))
}

fn cell_erase(s: &mut Session, v: &Value) -> CmdResult {
    let forward = p::bool(v, "forward").unwrap_or(false);
    let to = p::u64(v, "to").and_then(|n| usize::try_from(n).ok());
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let length = s
        .doc()
        .and_then(|(body, _)| body.text().table_at(edit.from))
        .and_then(|table| table.cell_len(edit.row, edit.cell))
        .ok_or_else(|| fail("there is no such cell in that table"))?;
    let (lo, hi) = s.table_selected(edit);
    let (from, to, new_at) = if to.is_none() && !forward && lo != hi {
        // Backspace or Delete over a selection takes it, as in the text of a message.
        (lo.min(length), hi.min(length), lo)
    } else {
        let (from, to) = match to {
            Some(to) => (edit.at.min(length), to.min(length)),
            None if forward => (edit.at.min(length), edit.at.saturating_add(1).min(length)),
            None => (edit.at.saturating_sub(1), edit.at.min(length)),
        };
        let (from, to) = (from.min(to), from.max(to));
        let removed = to.saturating_sub(from);
        let new_at = if from < edit.at { edit.at.saturating_sub(removed) } else { edit.at };
        (from, to, new_at)
    };
    cell_write_at(s, |table, edit| table.cell_erase(edit.row, edit.cell, from, to), new_at)
}

fn cell_enter(s: &mut Session, v: &Value) -> CmdResult {
    let _ = v;
    let style = s.table_edit_now().and_then(|edit| s.table_pending.clone().or_else(|| style_in_table(s, edit))).unwrap_or_default();
    cell_write(
        s,
        move |table, edit, lo, hi| table.cell_erase(edit.row, edit.cell, lo, hi) && table.cell_insert_styled(edit.row, edit.cell, lo, "\n", &style),
        1,
    )
}

fn cell_leave(s: &mut Session, _: &Value) -> CmdResult {
    s.table_edit = None;
    Ok(json!({"editing": false}))
}

/// Edit the cell the owner is in, over its selection, and put the caret `advance` characters
/// after where the selection began.
fn cell_write(
    s: &mut Session,
    f: impl FnOnce(&mut mailcraft_model::rich::PastedTable, crate::session::TableEdit, usize, usize) -> bool,
    advance: usize,
) -> CmdResult {
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let (lo, hi) = s.table_selected(edit);
    let new_at = lo.saturating_add(advance);
    cell_write_at(s, move |table, edit| f(table, edit, lo, hi), new_at)
}

fn cell_write_at(
    s: &mut Session,
    f: impl FnOnce(&mut mailcraft_model::rich::PastedTable, crate::session::TableEdit) -> bool,
    new_at: usize,
) -> CmdResult {
    let place = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    edit(s, &json!({"from": place.from}), |body| {
        body.edit_text(|text| {
            let table = text.table_at_mut(place.from).ok_or_else(|| ModelError::Invalid("there is no table there".into()))?;
            if f(table, place) { Ok(()) } else { Err(ModelError::Invalid("there is no such cell in that table".into())) }
        })
    })?;
    if let Some(edit) = s.table_edit.as_mut() {
        edit.at = new_at;
        edit.anchor = new_at;
    }
    Ok(cell_caret(s, s.table_edit.unwrap_or(place)))
}

fn cell_caret(s: &Session, edit: crate::session::TableEdit) -> Value {
    let length = s.doc().and_then(|(body, _)| body.text().table_at(edit.from)).and_then(|table| table.cell_len(edit.row, edit.cell)).unwrap_or(0);
    json!({
        "from": edit.from,
        "row": edit.row,
        "cell": edit.cell,
        "at": edit.at.min(length),
        "anchor": edit.anchor.min(length),
        "length": length,
    })
}

/// Select the characters of a cell between two places in it.
fn cell_select(s: &mut Session, v: &Value) -> CmdResult {
    let (from, row, cell) = cell_of(s, v)?;
    let n = |key: &str| p::u64(v, key).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(format!("`{key}` is a character of the cell")));
    let (anchor, at) = (n("anchor")?, n("at")?);
    let edit = s.set_table_selection(from, row, cell, anchor, at).ok_or_else(|| fail("there is no such cell in that table"))?;
    Ok(cell_caret(s, edit))
}

/// Select all the text of the cell being edited.
fn cell_select_all(s: &mut Session, _: &Value) -> CmdResult {
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let length = s
        .doc()
        .and_then(|(body, _)| body.text().table_at(edit.from))
        .and_then(|table| table.cell_len(edit.row, edit.cell))
        .ok_or_else(|| fail("there is no such cell in that table"))?;
    let edit = s.set_table_selection(edit.from, edit.row, edit.cell, 0, length).ok_or_else(|| fail("there is no such cell in that table"))?;
    Ok(cell_caret(s, edit))
}

/// The selected characters of the cell being edited as the clipboard takes them: HTML of the
/// set, and its plain text.
fn cell_clipboard(s: &mut Session) -> Result<(Option<String>, String), CmdError> {
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let (lo, hi) = s.table_selected(edit);
    if lo == hi {
        return Err(fail("nothing is selected in the cell"));
    }
    let (plain, part) = {
        let (body, plain) = s.doc().ok_or_else(|| fail("no message is being written"))?;
        let table = body.text().table_at(edit.from).ok_or_else(|| fail("there is no table there"))?;
        let part = table.cell_fragment(edit.row, edit.cell, lo, hi).ok_or_else(|| fail("there is no such cell in that table"))?;
        (plain, part)
    };
    let html = (!plain).then(|| mailcraft_mime_model::rich::to_html(&part));
    Ok((html, part.to_plain()))
}

fn cell_copy(s: &mut Session, _: &Value) -> CmdResult {
    let (html, text) = cell_clipboard(s)?;
    Ok(json!({"html": html, "text": text}))
}

fn cell_cut(s: &mut Session, _: &Value) -> CmdResult {
    let (html, text) = cell_clipboard(s)?;
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let (lo, hi) = s.table_selected(edit);
    cell_write_at(s, |table, edit| table.cell_erase(edit.row, edit.cell, lo, hi), lo)?;
    Ok(json!({"html": html, "text": text}))
}

/// The cell the owner is in and its selected characters, when a command names no range of the
/// text (the format commands the ribbon and the keys run with none).
fn cell_target(s: &mut Session, v: &Value) -> Option<(crate::session::TableEdit, usize, usize)> {
    let names = |key: &str| v.get(key).is_some_and(|x| !x.is_null());
    if names("from") || names("to") {
        return None;
    }
    let edit = s.table_edit_now()?;
    let (lo, hi) = s.table_selected(edit);
    Some((edit, lo, hi))
}

/// Change how the selected characters of the cell being edited look, or, with nothing selected,
/// the style the next typed character will have.
fn cell_apply(s: &mut Session, change: Format) -> Result<(), CmdError> {
    let place = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let (lo, hi) = s.table_selected(place);
    if lo == hi {
        let mut style = s.table_pending.clone().or_else(|| style_in_table(s, place)).unwrap_or_default();
        change.apply(&mut style);
        s.table_pending = Some(style);
        return Ok(());
    }
    edit(s, &json!({"from": place.from}), |body| {
        body.edit_text(|text| {
            let table = text.table_at_mut(place.from).ok_or_else(|| ModelError::Invalid("there is no table there".into()))?;
            if table.cell_format(place.row, place.cell, lo, hi, &change) {
                Ok(())
            } else {
                Err(ModelError::Invalid("there is no such cell in that table".into()))
            }
        })
    })
}

/// Turn something on or off over the selection of the cell being edited (the next typed
/// character's style with nothing selected): on unless all of it has it already. Whether it is
/// on now.
fn cell_toggle(s: &mut Session, has: &dyn Fn(&Style) -> bool, on: Format, off: Format) -> Result<bool, CmdError> {
    let edit = s.table_edit_now().ok_or_else(|| fail("the owner isn't typing in a table cell"))?;
    let (lo, hi) = s.table_selected(edit);
    let already = if lo == hi {
        let style = s.table_pending.clone().or_else(|| style_in_table(s, edit)).unwrap_or_default();
        has(&style)
    } else {
        let (body, _) = s.doc().ok_or_else(|| fail("no message is being written"))?;
        let table = body.text().table_at(edit.from).ok_or_else(|| fail("there is no table there"))?;
        table.cell_all(edit.row, edit.cell, lo, hi, has).ok_or_else(|| fail("there is no such cell in that table"))?
    };
    let now = !already;
    cell_apply(s, if now { on } else { off })?;
    Ok(now)
}

/// The sides a table cell has a line on, as the window's selected cell names it.
fn cell_borders(s: &mut Session, v: &Value) -> CmdResult {
    let (from, row, cell) = cell_of(s, v)?;
    let table = s.doc().and_then(|(body, _)| body.text().table_at(from)).ok_or_else(|| fail("there is no table there"))?;
    let borders = table.borders(row, cell).ok_or_else(|| fail("there is no such cell in that table"))?;
    Ok(cell_answer(from, row, cell, borders))
}

/// Turn some of a table cell's sides on or off.
fn table_borders(s: &mut Session, v: &Value) -> CmdResult {
    let (from, row, cell) = cell_of(s, v)?;
    let before = s
        .doc()
        .and_then(|(body, _)| body.text().table_at(from))
        .and_then(|table| table.borders(row, cell))
        .ok_or_else(|| fail("there is no such cell in that table"))?;
    let mut borders = before;
    for (key, side) in [("top", &mut borders.top), ("right", &mut borders.right), ("bottom", &mut borders.bottom), ("left", &mut borders.left)] {
        if let Some(on) = p::bool(v, key) {
            *side = on;
        }
    }
    edit(s, &json!({"from": from}), |body| {
        body.edit_text(|text| {
            let table = text.table_at_mut(from).ok_or_else(|| ModelError::Invalid("there is no table there".into()))?;
            if table.set_borders(row, cell, borders) { Ok(()) } else { Err(ModelError::Invalid("there is no such cell in that table".into())) }
        })
    })?;
    Ok(cell_answer(from, row, cell, borders))
}

/// `from`, `row` and `cell` of a table cell, as the window names it.
fn cell_of(s: &Session, v: &Value) -> Result<(usize, usize, usize), CmdError> {
    let from = position(v, "from")?.ok_or_else(|| bad("`from` (number) is required"))?;
    let n = |key: &str| p::u64(v, key).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(format!("`{key}` is a number of a row or a cell")));
    let (row, cell) = (n("row")?, n("cell")?);
    if s.doc().is_none() {
        return Err(fail("no message is being written"));
    }
    Ok((from, row, cell))
}

fn cell_answer(from: usize, row: usize, cell: usize, borders: mailcraft_model::rich::CellBorders) -> Value {
    json!({"from": from, "row": row, "cell": cell, "top": borders.top, "right": borders.right, "bottom": borders.bottom, "left": borders.left})
}

/// Insert › Table: an empty grid at the caret, as a pasted one is.
fn insert_table(s: &mut Session, v: &Value) -> CmdResult {
    let (rows, columns) = (cells(v, "rows")?, cells(v, "columns")?);
    if !(1..=MAX_PASTED_ROWS).contains(&rows) || !(1..=MAX_PASTED_COLUMNS).contains(&columns) {
        return Err(bad(format!("a table of {rows} rows and {columns} columns is more than a message holds")));
    }
    let Some(table) = PastedTable::new(
        u16::try_from(columns).unwrap_or(MAX_PASTED_COLUMNS as u16),
        (0..rows).map(|_| (0..columns).map(|_| empty_cell()).collect()).collect(),
    ) else {
        return Err(bad(format!("a table of {rows} rows and {columns} columns is more than a message holds")));
    };
    // Where it goes, so that the owner types in its first cell at once, as in Outlook: the caret
    // of the message is not moved (a script's `compose.type` still types where it was).
    let start = s.doc().map_or(0, |(body, _)| body.selection().start());
    edit(s, &json!({}), |b| b.paste(&RichText::from_paragraphs(vec![Paragraph::new(vec![Run::table(std::sync::Arc::new(table))], ListKind::None)])))?;
    s.set_table_edit(start, 0, 0, 0);
    Ok(state(s))
}

/// A row or column count from the params: 3 when it isn't given.
fn cells(v: &Value, key: &str) -> Result<usize, CmdError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(3),
        Some(_) => p::u64(v, key).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(format!("`{key}` is a number of cells"))),
    }
}

/// A cell of a table just inserted: one empty line.
fn empty_cell() -> TableCell {
    TableCell {
        lines: vec![vec![CellText { text: String::new(), style: Style::default() }]],
        image: None,
        header: false,
        colspan: 1,
        rowspan: 1,
        background: None,
        alignment: CellAlignment::Left,
        borders: mailcraft_model::rich::CellBorders::ALL,
    }
}

fn set_format(s: &mut Session, format: BodyFormat) -> CmdResult {
    let c = compose_mut(s)?;
    if c.format != format {
        c.format = format;
        c.dirty = true;
    }
    Ok(json!({"format": c.format}))
}

fn format_html(s: &mut Session, _: &Value) -> CmdResult {
    set_format(s, BodyFormat::Html)
}

fn format_plain(s: &mut Session, _: &Value) -> CmdResult {
    set_format(s, BodyFormat::Plain)
}

#[cfg(test)]
#[path = "highlight_tests.rs"]
mod highlight_tests;
