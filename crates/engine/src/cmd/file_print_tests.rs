// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Printing a message: the printable text `file.print` gives the window, with its headers, its
//! files by name, its text, and never the hidden text a sender tucked into the HTML.

use serde_json::json;

use crate::Session;
use crate::sample;
use mailcraft_model::{Address, Attachment, Body, Message};

fn session() -> Session {
    let now = 1_800_000_000;
    let mut s = Session::at(sample::mailbox(now, 0), now, 0);
    let folder = s.view.folder;
    let id = s
        .mailbox
        .add_message(Message {
            folder,
            from: Address::new("Ada Lovelace", "ada@example.org"),
            to: vec![Address::new("Bob", "bob@example.org")],
            subject: "Quarterly report".into(),
            date: now,
            body: Body { text: "The numbers are in.".into(), html: None },
            attachments: vec![Attachment { name: "report.pdf".into(), ..Default::default() }],
            ..Default::default()
        })
        .unwrap_or(0);
    s.sel.messages = vec![id];
    s
}

#[test]
fn a_message_is_printed_as_its_headers_its_files_and_its_text() {
    let mut s = session();
    let id = s.sel.focus().unwrap_or(0);
    let printed = s.run("file.print", &json!({"id": id})).unwrap();
    assert_eq!(printed["name"], json!("Quarterly report.txt"));
    let text = printed["text"].as_str().unwrap_or_default();
    for said in [
        "Subject: Quarterly report",
        "From: Ada Lovelace <ada@example.org>",
        "To: Bob <bob@example.org>",
        "Attachments: report.pdf",
        "The numbers are in.",
    ] {
        assert!(text.contains(said), "{said} is not in {text:?}");
    }
}

#[test]
fn the_text_of_an_html_body_is_printed_and_hidden_text_never_is() {
    let mut s = session();
    let id = s.sel.focus().unwrap_or(0);
    if let Ok(message) = s.mailbox.message_mut(id) {
        message.body.text.clear();
        message.body.html = Some("<p>Shown <span style=\"display:none\">hidden words</span></p>".into());
    }
    let printed = s.run("file.print", &json!({"id": id})).unwrap();
    let text = printed["text"].as_str().unwrap_or_default();
    assert!(text.contains("Shown"), "the visible text: {text:?}");
    assert!(!text.contains("hidden words"), "hidden sender text is never printed: {text:?}");
}

#[test]
fn printing_without_a_message_says_to_select_one() {
    let mut s = session();
    s.sel.messages.clear();
    assert!(s.run("file.print", &json!({})).is_err());
}
