// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Insert › Date & Time and Table, and Review › Word Count in the window (#507): the tabs while a
//! message is being written, the dialogs they open, and what those do.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use mailcraft_engine::Session;
use serde_json::json;

use crate::dialogs::Dialog;
use crate::{MailApp, RichClipboard, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

/// A new message whose body is "Hello world" alone.
fn writing() -> MailApp {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.execute("mail.new", json!({})).unwrap();
    let len = app.session.compose.as_ref().unwrap().body.text().len();
    app.execute("compose.select", json!({"from": 0, "to": len})).unwrap();
    app.execute("compose.type", json!({"text": "Hello world"})).unwrap();
    app
}

fn window(app: MailApp) -> Harness<'static, MailApp> {
    let mut harness = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn body(app: &MailApp) -> String {
    app.session.compose.as_ref().unwrap().body.plain()
}

#[test]
fn insert_and_review_are_tabs_while_writing() {
    let app = writing();
    assert_eq!(crate::ribbon::tabs(&app)[..6], ["File", "Message", "Insert", "Options", "Format Text", "Review"]);
    let mut app = writing();
    app.execute("compose.discard", json!({})).unwrap();
    assert!(!crate::ribbon::tabs(&app).contains(&"Insert"));
}

#[test]
fn date_and_time_lists_the_formats_and_ok_puts_one_in() {
    let mut h = window(writing());
    h.state_mut().run("insert.dateTime", json!({})).unwrap();
    h.run_steps(3);
    match h.state().dialog.as_ref() {
        Some(Dialog::DateTime { choices, selected: 0, .. }) => {
            assert_eq!(choices.len(), 8);
            assert_eq!(choices[0], ("long", "Wednesday, October 14, 2026".to_string()));
        }
        _ => panic!("Date and Time isn't open"),
    }
    // The second (Down), then OK.
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert!(matches!(h.state().dialog, Some(Dialog::DateTime { selected: 1, .. })));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(body(h.state()), "Hello worldOctober 14, 2026");
}

/// OK puts in what the dialog showed, though the clock has gone on to another day since.
#[test]
fn date_and_time_puts_in_what_it_showed() {
    let mut h = window(writing());
    h.state_mut().run("insert.dateTime", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().session.clock.now = Some(NOW + 86_400);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(body(h.state()), "Hello worldWednesday, October 14, 2026");
}

/// Ctrl+Shift+G (Cmd+Shift+G on a Mac) is Word Count while a message is being written, and
/// nothing else has it.
#[test]
fn ctrl_shift_g_counts_the_words() {
    let app = writing();
    assert_eq!(crate::keys::resolve(&app, "Mod+Shift+G"), Some("review.wordCount"));
    let bound: Vec<&str> =
        app.session.registry.all().iter().filter(|c| c.shortcut.split(" / ").any(|k| k.eq_ignore_ascii_case("Mod+Shift+G"))).map(|c| c.id).collect();
    assert_eq!(bound, ["review.wordCount"]);
    let mut app = writing();
    app.execute("compose.discard", json!({})).unwrap();
    assert_eq!(crate::keys::resolve(&app, "Mod+Shift+G"), None);
}

/// The rest of the two tabs is there, greyed, and runs nothing.
#[test]
fn the_rest_of_insert_and_review_is_pending() {
    let mut app = writing();
    let tabs: [(&str, &[&str]); 2] = [("Insert", &["Diagram", "Saved Text", "Chart"]), ("Review", &["Thesaurus", "Translate", "Language"])];
    for (tab, pending) in tabs {
        app.execute("ui.tab", json!({"tab": tab})).unwrap();
        let described = crate::ribbon::describe(&app);
        for &label in pending {
            let item = described.items().find(|item| item.label == label).unwrap_or_else(|| panic!("{tab}: no {label}"));
            assert!(item.pending && item.command.is_none() && item.menu.is_none(), "{tab}: {label}");
        }
    }
    let mut h = window(app);
    h.state_mut().execute("ui.ribbonMode", json!({"value": "classic"})).unwrap();
    h.run_steps(3);
    // Named for a screen reader, and disabled.
    let pending = h.get_by_label("Thesaurus, pending development");
    assert!(pending.accesskit_node().is_disabled());
    let before = body(h.state());
    let rect = h.state().hit.rect("ribbon", crate::ribbon::hit_id("review.thesaurus")).unwrap();
    let pos = rect.center();
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(body(h.state()), before);
}

/// Insert › Table puts an empty grid at the caret, as the command does.
#[test]
fn the_table_button_inserts_a_grid() {
    let mut app = writing();
    app.execute("ui.tab", json!({"tab": "Insert"})).unwrap();
    assert!(app.enabled("insert.table"), "the command is enabled while a message is being written");
    let described = crate::ribbon::describe(&app);
    let item = described.items().find(|item| item.label == "Table").unwrap_or_else(|| panic!("no Table in Insert"));
    assert_eq!(item.command, Some(("ui.dialog", json!({"name": "insertTable"}))), "the button asks for a size");
    let mut h = window(app);
    crate::reminders::dismiss(h.state_mut(), None);
    h.state_mut().run("ui.tab", json!({"tab": "Insert"})).unwrap();
    h.run_steps(3);
    // The window's own (simplified) row shows Table with its name, not an unnamed grid among
    // others: it is what the Insert tab is for.
    let simple = h
        .state()
        .hit
        .rect("ribbon", crate::ribbon::hit_id("insert.table"))
        .unwrap_or_else(|| panic!("Table isn't drawn in the simplified Insert row"));
    assert!(simple.width() > 60.0, "Table keeps its name in the simplified row: {simple:?}");
    h.state_mut().run("ui.ribbonMode", json!({"value": "classic"})).unwrap();
    h.run_steps(3);
    let rect = h.state().hit.rect("ribbon", crate::ribbon::hit_id("insert.table")).unwrap_or_else(|| panic!("Table isn't drawn in the ribbon"));
    click_at(&mut h, rect.center());
    assert!(
        matches!(h.state().dialog, Some(crate::dialogs::Dialog::InsertTable { rows: 3, columns: 3 })),
        "the button asks how many rows and columns"
    );
    h.state_mut().run("ui.closeDialog", json!({})).unwrap();
    h.state_mut().run("insert.table", json!({"rows": 2, "columns": 3})).unwrap();
    let state = h.state();
    let compose = state.session.compose.as_ref().unwrap_or_else(|| panic!("the message is still being written"));
    let grid = compose
        .body
        .text()
        .paragraphs()
        .iter()
        .find_map(|p| p.runs.iter().find_map(|r| r.table.as_ref()))
        .unwrap_or_else(|| panic!("no grid in {:?}", compose.body.plain()));
    assert_eq!((grid.columns(), grid.rows().len()), (3, 2));
    assert_eq!(grid.plain(), "\t\t\n\t\t");
}

fn click_at(harness: &mut Harness<'static, MailApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        harness.step();
    }
    harness.run_steps(3);
}

#[test]
fn word_count_shows_the_counts() {
    let mut h = window(writing());
    h.state_mut().run("review.wordCount", json!({})).unwrap();
    h.run_steps(3);
    match h.state().dialog.as_ref() {
        Some(Dialog::WordCount { counts, selection: false }) => assert_eq!(*counts, [2, 10, 11, 1]),
        _ => panic!("Word Count isn't open"),
    }
    assert!(h.query_by_label("Characters (with spaces)").is_some());
    // (The message being written has a Close of its own.)
    h.query_all_by_label("Close").last().unwrap().click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(body(h.state()), "Hello world");
}

#[test]
fn neither_opens_without_a_message() {
    let mut app = writing();
    app.execute("compose.discard", json!({})).unwrap();
    assert!(app.run("review.wordCount", json!({})).is_err());
    assert!(app.run("insert.dateTime", json!({})).is_err());
    assert!(app.dialog.is_none());
}

#[test]
fn the_borders_dialog_opens_for_the_clicked_table_cell() {
    let mut app = writing();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    app.ui.table_cell = Some((from, 0, 0));
    app.execute("ui.dialog", json!({"name": "tableBorders"})).unwrap();
    match &app.dialog {
        Some(crate::dialogs::Dialog::TableBorders { from: at, row: 0, cell: 0, top: true, right: true, bottom: true, left: true }) if *at == from => {
        }
        other => panic!("the cell's borders were not asked about: {:?}", other.is_some()),
    }
}

/// Typing while the caret is in a table cell goes into that cell (the click that puts it there
/// does `table.cellEdit`, and the text events follow it), not into the message around it.
#[test]
fn typing_goes_into_the_table_cell_the_caret_is_in() {
    let mut app = writing();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(3);
    let answer = h.state_mut().execute("table.cellEdit", json!({"from": from, "row": 1, "cell": 1, "at": 0})).expect("the cell can be entered");
    assert_eq!(answer.get("at").and_then(serde_json::Value::as_u64), Some(0));
    h.state_mut().ui.table_cell = Some((from, 1, 1));
    h.state_mut().ui.table_caret = Some((from, 1, 1, 0));
    h.run_steps(2);
    h.event(egui::Event::Text("cell".into()));
    h.run_steps(3);
    let cell = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).and_then(|table| table.cell_text(1, 1));
    assert_eq!(cell.as_deref(), Some("cell"));
}

/// A click in a table cell puts the caret there and typing goes into the cell: the whole way a
/// press takes, not only `table.cellEdit` run by a script.
#[test]
fn a_click_in_a_table_cell_takes_the_typing() {
    let mut app = writing();
    // An empty message, whose table is the one thing at the top.
    app.execute("compose.select", json!({"from": 0})).unwrap();
    app.execute("compose.type", json!({"text": "\u{8}"})).ok();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.run_steps(3);
    let body = h.state().hit.rect("composeBody", 0).expect("the body is drawn");
    click_at(&mut h, body.min + egui::vec2(20.0, 12.0));
    assert_eq!(h.state().ui.table_caret.map(|(place, row, cell, _)| (place, row, cell)), Some((from, 0, 0)), "the click is in the first cell");
    assert!(h.state().session.compose.as_ref().unwrap().body.selection().is_empty(), "the table is not selected beside the cell");
    h.event(egui::Event::Text("p".into()));
    h.run_steps(3);
    let cell = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).and_then(|table| table.cell_text(0, 0));
    assert_eq!(cell.as_deref(), Some("p"));
}

/// Insert › Table puts the window's caret in the first cell, so that typing goes there without a
/// click (the engine says where it is; the window follows).
#[test]
fn inserting_a_table_puts_the_typing_in_its_first_cell() {
    let mut app = writing();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(3);
    assert_eq!(h.state().ui.table_caret, Some((from, 0, 0, 0)), "the first cell has the caret");
    h.event(egui::Event::Text("t".into()));
    h.run_steps(3);
    let cell = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).and_then(|table| table.cell_text(0, 0));
    assert_eq!(cell.as_deref(), Some("t"));
}

/// A click in a cell takes the keyboard from the line the new message opened on (its To line),
/// as the owner's first click does: the cell is typed in, and the message around it is not.
#[test]
fn a_click_in_a_table_cell_takes_the_keyboard_from_the_recipient_line() {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.execute("mail.new", json!({})).unwrap();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("compose.select", json!({"from": 0})).unwrap();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.run_steps(3);
    let body = h.state().hit.rect("composeBody", 0).expect("the body is drawn");
    click_at(&mut h, body.min + egui::vec2(20.0, 12.0));
    assert_eq!(h.state().ui.table_caret.map(|(place, _, _, _)| place), Some(from), "the click put the caret in the cell");
    h.event(egui::Event::Text("k".into()));
    h.run_steps(3);
    let cell = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).and_then(|table| table.cell_text(0, 0));
    assert_eq!(cell.as_deref(), Some("k"), "the letter went into the cell, not the message");
}

/// A clipboard with HTML on it, and no picture.
struct HtmlBoard(String);

impl RichClipboard for HtmlBoard {
    fn put(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn html(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

/// A paste into a table cell keeps the clipboard's formatting: the HTML goes to `table.cellPaste`,
/// not as the plain text a typing goes in as.
#[test]
fn a_paste_into_a_table_cell_keeps_its_html_formatting_in_the_editor() {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.execute("mail.new", json!({})).unwrap();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("compose.select", json!({"from": 0})).unwrap();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    app.services.clipboard = Some(Box::new(HtmlBoard("<b>Bold</b> <span style=\"color:#ff0000\">Red</span>".into())));
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.run_steps(3);
    let body = h.state().hit.rect("composeBody", 0).expect("the body is drawn");
    click_at(&mut h, body.min + egui::vec2(20.0, 12.0));
    h.event(egui::Event::Paste("Bold Red".into()));
    h.run_steps(3);
    let table = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).expect("the table");
    let cell = &table.rows()[0][0];
    assert!(cell.lines.iter().flatten().any(|run| run.text == "Bold" && run.style.bold), "{:?}", cell.lines);
    assert!(cell.lines.iter().flatten().any(|run| run.text == "Red" && run.style.color.as_deref() == Some("#ff0000")), "{:?}", cell.lines);
}

/// An image on the clipboard, and nothing else.
struct ImageBoard(Vec<u8>);

impl RichClipboard for ImageBoard {
    fn put(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn html(&self) -> Option<String> {
        None
    }

    fn image(&self) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

/// Pasting a picture with the caret in a cell puts it above that cell's text.
#[test]
fn a_pasted_picture_goes_above_the_table_cell_text() {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.execute("mail.new", json!({})).unwrap();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("compose.select", json!({"from": 0})).unwrap();
    app.execute("insert.table", json!({"rows": 2, "columns": 2})).unwrap();
    let png = b"\x89PNG\r\n\x1a\nIHDR....".to_vec();
    app.services.clipboard = Some(Box::new(ImageBoard(png)));
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.run_steps(3);
    let body = h.state().hit.rect("composeBody", 0).expect("the body is drawn");
    click_at(&mut h, body.min + egui::vec2(20.0, 12.0));
    h.event(egui::Event::Paste(String::new()));
    h.run_steps(3);
    let table = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).expect("the table");
    assert!(table.cell_image(0, 0).is_some(), "the picture is above the first cell");
    assert!(h.state().session.compose.as_ref().unwrap().pictures.iter().any(|p| p.id == table.cell_image(0, 0).unwrap_or(0)));
}

/// What the system's clipboard was given.
struct Recorder(std::sync::Arc<std::sync::Mutex<Option<(String, String)>>>);

impl RichClipboard for Recorder {
    fn put(&self, html: &str, text: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some((html.to_string(), text.to_string()));
        Ok(())
    }

    fn html(&self) -> Option<String> {
        None
    }
}

/// Shift and an arrow key select inside the cell being typed in, and a format command the ribbon
/// or the keys run changes only what is selected there.
#[test]
fn shift_and_the_arrow_keys_select_in_a_table_cell() {
    let mut app = writing();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 1, "columns": 1})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(2);
    h.event(egui::Event::Text("word".into()));
    h.run_steps(3);
    for _ in 0..3 {
        h.event(egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT });
        h.run_steps(1);
    }
    assert_eq!(h.state().session.table_selection(), Some((from, 0, 0, 1, 4)), "three characters are selected");
    // The ribbon's Underline (and the key's) run `format.underline`, which acts in the cell.
    let said = h.state_mut().execute("format.underline", json!({})).unwrap();
    assert_eq!(said["underline"], true);
    let table = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).expect("the table");
    assert_eq!(table.cell_all(0, 0, 0, 1, &|style| style.underline), Some(false), "the first character is not underlined");
    assert_eq!(table.cell_all(0, 0, 1, 4, &|style| style.underline), Some(true), "the selection is");
    // The window shows the selection, and the ribbon reads the cell's look.
    assert_eq!(h.state().ui.table_anchor.map(|(_, _, _, at)| at), Some(4), "the selection began where the caret was");
    assert!(crate::ribbon::look_now(h.state()).underline, "the ribbon says the selection is underlined");
}

/// A copy of text selected in a table cell goes to the clipboard as HTML and text, with its look.
#[test]
fn copying_a_selection_in_a_table_cell_keeps_its_formatting() {
    let mut app = writing();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 1, "columns": 1})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let kept = std::sync::Arc::new(std::sync::Mutex::new(None));
    app.services.clipboard = Some(Box::new(Recorder(kept.clone())));
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(2);
    h.event(egui::Event::Text("bold plain".into()));
    h.run_steps(3);
    h.state_mut().execute("table.cellSelect", json!({"from": from, "row": 0, "cell": 0, "anchor": 0, "at": 4})).unwrap();
    h.state_mut().execute("format.bold", json!({})).unwrap();
    h.run_steps(1);
    h.event(egui::Event::Copy);
    h.run_steps(2);
    let (html, text) = kept.lock().unwrap().clone().expect("the cell's selection went to the clipboard");
    assert_eq!(text, "bold");
    assert!(html.contains("<b>") || html.contains("font-weight:bold"), "{html}");
    assert!(html.contains("bold"), "{html}");
}

/// An image copied from a browser (its HTML is that picture alone) goes into the cell being typed
/// in, from the picture the clipboard holds, rather than pasting its web address as a link.
#[test]
fn a_picture_copied_from_a_browser_goes_above_the_table_cell_text() {
    let mut app = writing();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 1, "columns": 1})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    app.services.clipboard = Some(Box::new(PictureBoard));
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(2);
    h.event(egui::Event::Paste(String::new()));
    h.run_steps(3);
    let table = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).expect("the table");
    assert!(table.cell_image(0, 0).is_some(), "the picture is above the cell's text");
}

/// A browser's Copy Image: HTML that is one `<img>` alone, and the picture itself.
struct PictureBoard;

impl RichClipboard for PictureBoard {
    fn put(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn html(&self) -> Option<String> {
        Some("<img src=\"https://example.org/cat.png\">".to_string())
    }

    fn image(&self) -> Option<Vec<u8>> {
        Some(b"\x89PNG\r\n\x1a\nIHDR....".to_vec())
    }
}

/// A drag across the text of a table cell selects it, as a drag in the message's own text does.
#[test]
fn a_drag_in_a_table_cell_selects_its_text() {
    let mut app = writing();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 1, "columns": 1})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(2);
    h.event(egui::Event::Text("hello world".into()));
    h.run_steps(3);
    let body = h.state().hit.rect("composeBody", 0).expect("the body is drawn");
    let from_at = body.min + egui::vec2(20.0, 12.0);
    let to_at = body.min + egui::vec2(62.0, 12.0);
    h.event(egui::Event::PointerMoved(from_at));
    h.step();
    h.event(egui::Event::PointerButton { pos: from_at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerMoved(to_at));
    h.step();
    h.event(egui::Event::PointerMoved(to_at));
    h.step();
    h.event(egui::Event::PointerButton { pos: to_at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    let selection = h.state().session.table_selection().expect("the drag selected some of the cell");
    assert_eq!((selection.0, selection.1, selection.2), (from, 0, 0));
    assert!(selection.3 <= 2, "from near the beginning: {selection:?}");
    assert!(selection.4 > selection.3, "some characters are selected: {selection:?}");
    // And the window draws it: its own ends agree with the engine's.
    assert!(h.state().ui.table_anchor.is_some(), "the window knows the selection began there");
}

/// A picture file copied in a file manager comes as its path: pasted with the caret in a table
/// cell, the picture goes above the cell's text.
#[test]
fn a_picture_file_copied_in_a_file_manager_goes_into_the_cell() {
    let dir = std::env::temp_dir().join(format!("mailcraft-cell-paste-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary folder");
    let path = dir.join("cat.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\nIHDR....").expect("a picture file");
    let mut app = writing();
    app.execute("compose.set", json!({"body": ""})).unwrap();
    app.execute("insert.table", json!({"rows": 1, "columns": 1})).unwrap();
    let from = {
        let body = &app.session.compose.as_ref().unwrap().body;
        (0..body.text().len()).find(|at| body.text().table_at(*at).is_some()).expect("the table")
    };
    let mut h = window(app);
    h.state_mut().want_body_focus();
    h.run_steps(2);
    h.event(egui::Event::Paste(path.to_string_lossy().to_string()));
    h.run_steps(3);
    let table = h.state().session.compose.as_ref().unwrap().body.text().table_at(from).expect("the table");
    assert!(table.cell_image(0, 0).is_some(), "the picture is above the cell's text");
    std::fs::remove_dir_all(&dir).ok();
}
