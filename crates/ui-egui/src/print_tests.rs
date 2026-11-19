// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! File › Print in the window: the message is set on paper here, and the pages are handed to the
//! system's spooler with the owner's choices (gap 15).

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::MailApp;
use crate::Services;
use crate::print::{Job, Printer};

const NOW: i64 = 1_800_000_000;

fn app() -> MailApp {
    let mut app = MailApp::new(mailcraft_engine::Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app
}

/// A window whose spooler keeps what it was handed.
fn spooler(app: &mut MailApp) -> Arc<Mutex<Vec<Job>>> {
    let seen: Arc<Mutex<Vec<Job>>> = Arc::new(Mutex::new(Vec::new()));
    let slot = seen.clone();
    app.services.print = Some(Box::new(move |job: Job| {
        slot.lock().unwrap_or_else(|e| e.into_inner()).push(job);
        Ok("job 1 on the default printer".to_owned())
    }));
    seen
}

#[test]
fn the_window_sets_the_message_on_paper_and_hands_the_pages_to_the_system() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    let seen = spooler(&mut app);
    let done = app.execute("file.print", json!({"id": id})).expect("printed");
    assert_eq!(done["job"], "job 1 on the default printer", "{done}");
    let jobs = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(jobs.len(), 1, "one job for one message");
    let job = &jobs[0];
    assert!(job.pdf.starts_with(b"%PDF-"), "the pages handed over are a PDF");
    assert!(job.pdf.starts_with(b"%PDF-1.7") || job.pdf.len() > 500, "the pages have something on them");
    assert_eq!((job.copies, job.collate, job.two_sided, job.grayscale), (1, true, false, false), "the defaults");
    assert!(job.printer.is_none(), "the system's default printer is used when none is named");
    assert!(!job.title.is_empty(), "the job is named after the message");
}

#[test]
fn the_owner_can_name_the_printer_and_the_page() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    app.services.printers = Some(Box::new(|| vec![Printer { name: "Laser".into(), default: true, two_sided: Some(true) }]));
    let seen = spooler(&mut app);
    app.execute(
        "file.print",
        json!({"id": id, "printer": "Laser", "copies": 2, "collate": false, "twoSided": true, "grayscale": true, "paper": "letter"}),
    )
    .expect("printed");
    let job = seen.lock().unwrap_or_else(|e| e.into_inner()).first().cloned().expect("a job");
    assert_eq!(
        (job.printer.as_deref(), job.copies, job.collate, job.two_sided, job.grayscale),
        (Some("Laser"), 2, false, true, true),
        "what was asked for is what went"
    );
    assert!(job.pdf.windows(17).any(|w| w == b"/MediaBox[0 0 612"), "Letter paper: 612 points wide");
}

#[test]
fn a_copies_count_out_of_range_is_brought_back() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    let seen = spooler(&mut app);
    app.execute("file.print", json!({"id": id, "copies": 0})).expect("printed");
    app.execute("file.print", json!({"id": id, "copies": 5000})).expect("printed");
    let jobs = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(jobs.iter().map(|j| j.copies).collect::<Vec<_>>(), vec![1, 100], "one copy at least, a hundred at most");
}

#[test]
fn without_a_spooler_the_command_says_so() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    let why = app.execute("file.print", json!({"id": id})).expect_err("no printer here");
    assert!(why.contains("printing isn't available"), "{why}");
}

#[test]
fn a_message_that_is_not_there_is_said_so() {
    let mut app = app();
    let id = app.session.mailbox.messages.keys().next().copied().unwrap_or(0);
    assert!(app.execute("file.print", json!({"id": id + 9999})).is_err(), "no message with that id");
}

/// The whole window laid out as on a screen. (In the headless window a message window sits inside
/// the main one, as in a browser; a window of the system draws its own dialog inside itself.)
fn screen(app: MailApp) -> egui_kittest::Harness<'static, MailApp> {
    let mut harness = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    harness.run_steps(3);
    harness
}

#[test]
fn the_print_dialog_of_a_window_stands_over_that_window() {
    let mut app = app();
    spooler(&mut app);
    app.services.printers = Some(Box::new(|| vec![Printer { name: "Office laser".into(), default: true, two_sided: Some(true) }]));
    let mut harness = screen(app);
    let id = harness.state_mut().session.rows()[1].id;
    harness.state_mut().execute("mail.select", json!({"id": id})).unwrap_or_else(|e| panic!("mail.select: {e}"));
    let opened = harness.state_mut().execute("ui.openWindow", json!({"message": id})).unwrap_or_else(|e| panic!("ui.openWindow: {e}"));
    let window = opened["window"].as_u64().unwrap_or(0);
    harness.run_steps(3);
    let here = harness.state().hit.rect("window", window).unwrap_or(egui::Rect::NOTHING);
    harness.state_mut().execute("ui.printDialog", json!({"window": window})).unwrap_or_else(|e| panic!("ui.printDialog: {e}"));
    harness.run_steps(3);
    let dialog = harness.state().hit.rect("printDialog", window).unwrap_or(egui::Rect::NOTHING);
    assert!(dialog.width() > 0.0, "the dialog is drawn");
    assert!(here.contains(dialog.center()), "the Print dialog stands over the window it was asked from, not the main window: {here} vs {dialog}");
    // Closing the window takes the dialog's home with it: the next one is centred on the main
    // window, where the reading pane is.
    harness.state_mut().execute("ui.closeWindow", json!({"window": window})).unwrap_or_else(|e| panic!("ui.closeWindow: {e}"));
    harness.run_steps(3);
    harness.state_mut().execute("ui.printDialog", json!({})).unwrap_or_else(|e| panic!("ui.printDialog: {e}"));
    harness.run_steps(3);
    let main = harness.state().hit.rect("printDialog", 0).unwrap_or(egui::Rect::NOTHING);
    assert!(main.width() > 0.0 && main.height() > 0.0, "the main window's Print dialog is drawn: {main}");
}
