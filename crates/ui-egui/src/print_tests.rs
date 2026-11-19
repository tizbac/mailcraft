// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! File › Print in the window: the engine makes the printable text, and the system is handed it.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::{MailApp, Services};

const NOW: i64 = 1_800_000_000;

fn app() -> MailApp {
    let mut app = MailApp::new(mailcraft_engine::Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app
}

#[test]
fn the_window_hands_the_printable_text_to_the_system() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    let seen: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
    let slot = seen.clone();
    app.services.print = Some(Box::new(move |name: &str, text: &str| {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((name.to_owned(), text.to_owned()));
        Ok(())
    }));
    app.execute("file.print", json!({"id": id})).expect("printed");
    let (name, text) = seen.lock().unwrap_or_else(|e| e.into_inner()).clone().expect("the system was handed the text");
    assert!(name.ends_with(".txt"), "{name}");
    assert!(text.contains("Subject:") && text.contains("From:"), "{text:?}");
}

#[test]
fn without_a_printer_the_command_says_so() {
    let mut app = app();
    let id = *app.session.mailbox.messages.keys().next().unwrap_or(&0);
    let why = app.execute("file.print", json!({"id": id})).expect_err("no printer here");
    assert!(why.contains("printing isn't available"), "{why}");
}
