// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Opening an attachment from the window: what is opened, what is asked first, what is refused,
//! the browser's download in its place, and the message files MailCraft reads itself (#126).

use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::Attachment;
use serde_json::json;

use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

type Opened = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// A message of the sample mailbox with attachments of name, media type and bytes, and a host that
/// notes what it is asked to open. The message's id comes back too.
fn app_with(parts: &[(&str, &str, &[u8])]) -> (MailApp, u64, Opened) {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    let id = app.session.rows().first().map(|r| r.id).unwrap();
    let m = app.session.mailbox.message_mut(id).unwrap();
    m.attachments = parts.iter().map(|(n, mime, bytes)| Attachment::new(*n, *mime, bytes.to_vec())).collect();
    app.run("mail.select", json!({"id": id})).unwrap();
    let opened: Opened = Default::default();
    let sink = opened.clone();
    app.services.open_file = Some(Box::new(move |name, bytes| {
        sink.lock().unwrap().push((name.to_string(), bytes.to_vec()));
        Ok(())
    }));
    (app, id, opened)
}

/// The same with `names` all holding `hello`, of an unknown kind.
fn app(names: &[&str]) -> (MailApp, u64, Opened) {
    app_with(&names.iter().map(|n| (*n, "application/octet-stream", b"hello" as &[u8])).collect::<Vec<_>>())
}

/// A message file, as one would be attached.
const EML: &str = "From: Maya Lin <maya@lumen.example>\r\nTo: avery@lumen.example\r\nSubject: Handed on\r\nDate: Wed, 14 Oct 2026 10:00:00 +0000\r\nMessage-ID: <handed@lumen.example>\r\n\r\nThe mail inside the mail.\r\n";

#[test]
fn an_ordinary_file_is_opened_under_a_safe_name() {
    let (mut a, id, opened) = app(&["../../report.pdf"]);
    let r = a.run("mail.openAttachment", json!({"id": id, "index": 0})).unwrap();
    assert_eq!((r["name"].as_str(), r["opened"].as_bool()), (Some("report.pdf"), Some(true)));
    assert_eq!(*opened.lock().unwrap(), vec![("report.pdf".to_string(), b"hello".to_vec())]);
}

#[test]
fn programs_and_scripts_are_refused_and_nothing_is_opened() {
    let (mut a, id, opened) = app(&["setup.exe", "invoice.pdf.js", "run.sh", "README"]);
    for index in 0..4 {
        let e = a.run("mail.openAttachment", json!({"id": id, "index": index})).unwrap_err();
        assert!(e.contains("doesn't open") && e.contains("Save As"), "{e}");
        // Said in a box, as the owner did it by hand.
        assert!(crate::refusal_worth_a_box("mail.openAttachment", &e));
    }
    // A script asking without the window's question is refused too.
    assert!(a.execute("mail.openAttachment", json!({"id": id, "index": 0})).is_err());
    assert!(opened.lock().unwrap().is_empty());
    // Saving them is still the owner's choice.
    assert!(a.execute("mail.saveAttachment", json!({"id": id, "index": 0})).is_ok());
}

#[test]
fn macros_and_web_pages_are_asked_about_first() {
    let (mut a, id, opened) = app(&["budget.xlsm", "page.html"]);
    for index in 0..2 {
        let r = a.run("mail.openAttachment", json!({"id": id, "index": index})).unwrap();
        assert_eq!(r["pending"], "confirm");
        let Some(crate::dialogs::Dialog::Confirm { ask, .. }) = &a.dialog else { panic!("no question") };
        assert_eq!(ask.title, "Opening Mail Attachment");
        a.dialog = None;
    }
    assert!(opened.lock().unwrap().is_empty());
    // Once the owner said yes, it opens.
    a.run("mail.openAttachment", json!({"id": id, "index": 0, "confirmed": true})).unwrap();
    assert_eq!(opened.lock().unwrap().len(), 1);
}

#[test]
fn the_browser_downloads_instead_and_nothing_breaks_without_a_host() {
    let (mut a, id, _) = app(&["photo.jpg"]);
    a.services.open_file = None;
    // No host at all: said, not crashed.
    let e = a.run("mail.openAttachment", json!({"id": id, "index": 0})).unwrap_err();
    assert!(e.contains("Save As"), "{e}");
    let got: Opened = Default::default();
    let sink = got.clone();
    a.services.download = Some(Box::new(move |name, bytes| {
        sink.lock().unwrap().push((name.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let r = a.run("mail.openAttachment", json!({"id": id, "index": 0})).unwrap();
    assert_eq!(r["downloaded"], true);
    assert_eq!(got.lock().unwrap().len(), 1);
}

/// The windows the app has open, as the window reports them.
fn open_windows(app: &MailApp) -> Vec<serde_json::Value> {
    crate::windows::inspect(app).as_array().map_or(Vec::new(), |w| w.to_vec())
}

#[test]
fn a_message_file_attachment_is_read_in_a_window_of_its_own_and_not_handed_to_the_computer() {
    let (mut a, id, opened) = app_with(&[
        ("forwarded.eml", "message/rfc822", EML.as_bytes()),
        ("FW: Handed on.eml", "application/octet-stream", EML.as_bytes()),
        ("attachment with no name", "message/rfc822", EML.as_bytes()),
        ("notes.txt", "text/plain", b"a note".as_slice()),
    ]);
    let before = a.session.rows().len();
    for index in 0..3 {
        let r = a.run("mail.openAttachment", json!({"id": id, "index": index})).unwrap();
        assert_eq!(r["opened"], true, "the message file is opened");
        assert!(r["window"].is_number(), "in a window of its own: {r}");
        assert!(r["downloaded"].is_null(), "the browser does not download it instead");
        // Nothing of it is written to disk or given to the computer's program for it.
        assert!(opened.lock().unwrap().is_empty(), "nothing was handed to the host: {r}");
        let windows = open_windows(&a);
        assert!(windows.iter().any(|w| w["kind"] == "file" && w["id"] == r["window"]), "the window of the message inside: {windows:?}");
    }
    // Nothing was filed: the owner says where it goes from the window.
    assert_eq!(a.session.rows().len(), before, "opening a message file adds nothing to the mailbox");
    // An ordinary file still goes to the computer.
    let r = a.run("mail.openAttachment", json!({"id": id, "index": 3})).unwrap();
    assert_eq!(r["opened"], true);
    assert_eq!(*opened.lock().unwrap(), vec![("notes.txt".to_string(), b"a note".to_vec())]);
}

#[test]
fn a_message_file_named_like_a_program_is_read_by_mailcraft_and_not_refused() {
    // (Nothing is given to the system for it, so the kinds that can run don't apply: MailCraft's
    // own bounded parser reads it.)
    let (mut a, id, opened) =
        app_with(&[("forwarded.eml.exe", "message/rfc822", EML.as_bytes()), ("handover", "application/octet-stream", EML.as_bytes())]);
    let r = a.run("mail.openAttachment", json!({"id": id, "index": 0})).unwrap();
    assert_eq!((r["opened"].as_bool(), r["window"].is_number()), (Some(true), true));
    assert!(opened.lock().unwrap().is_empty());
    // A file of no known kind that isn't a message is still refused, even as a window is open.
    let e = a.run("mail.openAttachment", json!({"id": id, "index": 1})).unwrap_err();
    assert!(e.contains("doesn't open") && e.contains("Save As"), "{e}");
    assert!(opened.lock().unwrap().is_empty());
}

#[test]
fn a_message_file_that_is_not_a_message_is_shown_or_said_so_and_never_crashes() {
    for junk in [b"\x00\x00\x01\r\n\r\n\x00\xff\xfe".as_slice(), b"".as_slice(), b"..help..".as_slice(), b"From: <>\r\n\r\n".as_slice()] {
        for name in ["forwarded.eml", "junk.eml"] {
            let (mut a, id, opened) = app_with(&[(name, "message/rfc822", junk)]);
            let before = a.session.rows().len();
            let r = a.run("mail.openAttachment", json!({"id": id, "index": 0}));
            // Read as every message is: shown in a window, or refused with a reason. Either way,
            // nothing is filed and nothing is handed to the computer.
            match r {
                Ok(r) => assert!(r["window"].is_number(), "{r}"),
                Err(e) => assert!(!e.is_empty(), "refused with a reason"),
            }
            assert!(opened.lock().unwrap().is_empty());
            assert_eq!(a.session.rows().len(), before, "the mailbox is untouched either way");
        }
    }
}

#[test]
fn a_message_file_larger_than_the_window_takes_is_refused_with_a_reason() {
    let big = vec![b'A'; crate::opened_message::MAX_BYTES as usize + 1_usize];
    let (mut a, id, opened) = app_with(&[("huge.eml", "message/rfc822", big.as_slice())]);
    let e = a.run("mail.openAttachment", json!({"id": id, "index": 0})).unwrap_err();
    assert!(e.contains("Save As"), "{e}");
    assert!(open_windows(&a).is_empty(), "no window for a file it won't hold");
    assert!(opened.lock().unwrap().is_empty());
}

#[test]
fn junk_params_are_refused() {
    let (mut a, id, opened) = app(&["report.pdf"]);
    for p in [
        json!({}),
        json!({"id": id}),
        json!({"id": id, "index": 7}),
        json!({"id": id, "index": -1}),
        json!({"id": id, "index": u64::MAX}),
        json!({"id": u64::MAX, "index": 0}),
        json!({"id": "x", "index": "y"}),
        json!(null),
        json!([1, 2]),
    ] {
        let _ = a.run("mail.openAttachment", p.clone());
        let _ = a.execute("mail.openAttachment", p);
    }
    assert!(opened.lock().unwrap().is_empty());
}
