// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The pages the window prints: one message set on paper, its pictures and tables, and what a
//! hostile message cannot do to them (gap 15).

use std::sync::Arc;

use mailcraft_engine::Session;
use mailcraft_model::{Address, FolderKind, Id, Mailbox, Message};
use mailcraft_print::Paper;
use serde_json::json;

use crate::print::{Ask, Choices, Job, page_count, pages, printers};
use crate::{MailApp, Services};

const NOW: i64 = 1_800_000_000;

/// Whether the written PDF says this somewhere — the page box, a drawn picture.
fn on_paper(pdf: &[u8], needle: &[u8]) -> bool {
    pdf.windows(needle.len()).any(|w| w == needle)
}
/// A 4×4 red PNG.
fn red_png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0, 169, 241, 158, 126, 0, 0, 0, 18, 73, 68,
        65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 25, 51, 144, 46, 0, 0, 60, 64, 31, 225, 224, 129, 119, 180, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ]
}

/// The bytes as base64, the way a message writes a picture into itself.
fn base64_of(bytes: &[u8]) -> String {
    const CODE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(CODE[(n >> 18) as usize] as char);
        out.push(CODE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(CODE[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(CODE[(n & 63) as usize] as char);
        }
    }
    out
}

struct Rig {
    app: MailApp,
    plain: Id,
    rich: Id,
}

fn rig() -> Rig {
    let mut mailbox = Mailbox::with_local_account("Ann", "ann@example.org");
    let account = mailbox.accounts[0].id;
    let inbox = mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let plain = mailbox
        .add_message(Message {
            account,
            folder: inbox,
            subject: "Plain text".into(),
            from: Address::new("Dan", "dan@example.org"),
            to: vec![Address::new("Ann", "ann@example.org")],
            date: NOW,
            body: mailcraft_model::Body { text: "Two lines\nof plain text".into(), html: None },
            ..Default::default()
        })
        .unwrap();
    let rich = mailbox
        .add_message(Message {
            account,
            folder: inbox,
            subject: "The quarterly review".into(),
            from: Address::new("Bo", "bo@example.org"),
            to: vec![Address::new("Ann", "ann@example.org")],
            date: NOW,
            body: mailcraft_model::Body {
                text: String::new(),
                html: Some(format!(
                    "<h1>Review</h1><p>A <b>bold</b> word and an <i>italic</i> one.</p>\
                     <ul><li>one thing</li><li>another</li></ul>\
                     <table border=1><tr><th>Month</th><th>Sum</th></tr><tr><td>October</td><td>12</td></tr></table>\
                     <img src=\"data:image/png;base64,{}\" alt=\"the chart\">\
                     <img src=\"cid:not-there@example.org\" width=40 height=40 alt=\"the photo that isn't there\">\
                     <img src=\"https://example.org/web.png\" alt=\"a picture on the web\">",
                    base64_of(&red_png())
                )),
            },
            attachments: vec![mailcraft_model::Attachment {
                name: "chart.png".into(),
                mime: "image/png".into(),
                content_id: "inline-chart@example.org".into(),
                inline: true,
                data: Arc::new(red_png()),
                kept: None,
            }],
            ..Default::default()
        })
        .unwrap();
    let mut app = MailApp::new(Session::at(mailbox, NOW, 0), Services::default());
    app.autosave = false;
    Rig { app, plain, rich }
}

#[test]
fn a_message_becomes_pages_on_the_sheet_asked_for() {
    let r = rig();
    let m = r.app.session.mailbox.message(r.rich).unwrap();
    let a4 = pages(&r.app, None, m, Paper::A4, "Inbox").unwrap_or_else(|why| panic!("the message printed: {why}"));
    let letter = pages(&r.app, None, m, Paper::LETTER, "Inbox").unwrap_or_else(|why| panic!("the message printed: {why}"));
    assert!(a4.starts_with(b"%PDF-"), "it is a PDF");
    // The sheet, as the writer put it in each page's own box.
    assert!(on_paper(&a4, b"/MediaBox[0 0 595.28"), "A4 is 595 points wide");
    assert!(on_paper(&letter, b"/MediaBox[0 0 612"), "Letter is 612 points wide");
    assert!(page_count(&a4) >= 1, "there is at least one page");
}

#[test]
fn a_plain_text_message_prints_its_lines() {
    let r = rig();
    let m = r.app.session.mailbox.message(r.plain).unwrap();
    let pdf = pages(&r.app, None, m, Paper::A4, "").unwrap_or_else(|why| panic!("plain text printed: {why}"));
    assert!(page_count(&pdf) == 1, "two lines are one page");
}

#[test]
fn a_picture_in_the_message_is_drawn_and_one_with_no_bytes_is_not_a_crash() {
    let r = rig();
    let m = r.app.session.mailbox.message(r.rich).unwrap();
    let with = pages(&r.app, None, m, Paper::A4, "").unwrap_or_else(|why| panic!("it printed: {why}"));
    // Only the pictures the caller has bytes for are drawn: the one written in, and the one that
    // is an attachment. Nothing is fetched for the web one, and the missing one prints its words.
    assert!(on_paper(&with, b"/Subtype/Image"), "a picture is drawn");
    let bigger = with.len();
    // Take the pictures away: fewer bytes on the same words.
    let mut bare = Message { attachments: Vec::new(), ..m.clone() };
    bare.body.html = Some("<h1>Review</h1><p>A <b>bold</b> word and an <i>italic</i> one.</p>".to_owned());
    let without = pages(&r.app, None, &bare, Paper::A4, "").unwrap_or_else(|why| panic!("it printed: {why}"));
    assert!(bigger > without.len(), "the pictures add to the pages");
}

#[test]
fn a_hostile_body_still_makes_pages() {
    let r = rig();
    let mut m = r.app.session.mailbox.message(r.rich).cloned().unwrap_or_default();
    // Deep quoting, many blocks, a picture that is not one, sizes out of range.
    let mut html = String::new();
    html.push_str(&"<blockquote>".repeat(60));
    html.push_str(&"<li style='margin:200px;font-size:200px'>x</li>".repeat(400));
    html.push_str(&format!("<img src=\"data:image/png;base64,{}\" alt=\"junk\">", base64_of(&[7u8; 4000])));
    m.body.html = Some(html);
    let pdf = pages(&r.app, None, &m, Paper::A4, "").unwrap_or_else(|why| panic!("a flood still printed: {why}"));
    assert!(pdf.starts_with(b"%PDF-"), "the pages are written");
    assert!(page_count(&pdf) >= 1, "there is something on them");
}

#[test]
fn paper_that_is_not_paper_is_refused() {
    let r = rig();
    let m = r.app.session.mailbox.message(r.plain).unwrap();
    assert!(pages(&r.app, None, m, Paper { width: 595.0, height: 842.0, margin: 500.0 }, "").is_err(), "no room left for text");
    assert!(pages(&r.app, None, m, Paper { width: f32::NAN, height: 842.0, margin: 50.0 }, "").is_err(), "a sheet that isn't a number");
}

#[test]
fn the_print_dialog_prints_with_what_the_owner_chose() {
    let mut r = rig();
    let seen: Arc<std::sync::Mutex<Vec<Job>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let slot = seen.clone();
    r.app.services.printers = Some(Box::new(|| {
        vec![
            crate::print::Printer { name: "Office laser".into(), default: false, two_sided: Some(true) },
            crate::print::Printer { name: "Home inkjet".into(), default: true, two_sided: Some(false) },
        ]
    }));
    r.app.services.print = Some(Box::new(move |job: Job| {
        slot.lock().unwrap_or_else(|e| e.into_inner()).push(job);
        Ok("job 7 on Office laser".to_owned())
    }));
    r.app.execute("mail.select", json!({"id": r.rich})).unwrap();
    let mut ask = Ask::open(&r.app).unwrap_or_else(|why| panic!("the dialog opened: {why}"));
    // The system's default printer is the one marked default.
    assert_eq!(ask.printer.as_deref(), Some("Home inkjet"));
    assert_eq!(printers(&r.app).len(), 2);
    ask.printer = Some("Office laser".to_owned());
    ask.choices = Choices { letter: true, copies: 3, collate: true, two_sided: true, grayscale: true };
    assert!(ask.go(&mut r.app), "it printed");
    let jobs = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!((job.printer.as_deref(), job.copies, job.collate, job.two_sided, job.grayscale), (Some("Office laser"), 3, true, true, true));
    assert_eq!(job.title, "The quarterly review");
    assert!(job.pdf.starts_with(b"%PDF-"), "the pages handed over are a PDF");
    // Nothing selected: the dialog says so, and prints nothing.
    r.app.execute("mail.select", json!({"ids": []})).unwrap();
    assert!(Ask::open(&r.app).is_err(), "nothing to print");
}

#[test]
fn without_a_spooler_the_dialog_says_so() {
    let mut r = rig();
    r.app.execute("mail.select", json!({"id": r.plain})).unwrap();
    let why = Ask::open(&r.app).err().expect("no printer here");
    assert!(why.contains("printing isn't available"), "{why}");
}
