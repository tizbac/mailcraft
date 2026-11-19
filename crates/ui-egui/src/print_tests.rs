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
