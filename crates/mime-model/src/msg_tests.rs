// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `.msg` file as it is written: the properties in the top-level stream, a recipient storage
//! and an attachment storage, each with its own values.

use std::io::Read as _;
use std::sync::Arc;

use cfb::CompoundFile;
use mailcraft_model::{Address, Attachment, Body, Message};

use super::write;

fn message() -> Message {
    Message {
        subject: "Lunch".into(),
        message_id: "m1@example.org".into(),
        from: Address::new("Ada Lovelace", "ada@example.org"),
        to: vec![Address::new("Bob", "bob@example.org")],
        cc: vec![Address::new("", "carol@example.org")],
        date: 1_800_000_000,
        body: Body { text: "See you at noon.".into(), html: Some("<p>See you at noon.</p>".into()) },
        attachments: vec![Attachment {
            name: "notes.txt".into(),
            mime: "text/plain".into(),
            data: Arc::new(b"the notes".to_vec()),
            content_id: "notes@example.org".into(),
            ..Attachment::default()
        }],
        ..Message::default()
    }
}

fn stream(file: &mut CompoundFile<std::io::Cursor<Vec<u8>>>, path: &str) -> Vec<u8> {
    let mut out = Vec::new();
    file.open_stream(path).unwrap_or_else(|e| panic!("{path}: {e}")).read_to_end(&mut out).unwrap();
    out
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).unwrap()
}

#[test]
fn a_message_is_a_compound_file_with_its_properties_recipients_and_attachments() {
    let bytes = write(&message()).unwrap();
    let mut file = CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    assert!(file.exists("/__properties_version1.0"));
    let top = stream(&mut file, "/__properties_version1.0");
    // A 32-byte header, then 16 bytes of each property it holds.
    assert!(top.len() >= 32);
    assert_eq!(top.len() % 16, 0);
    assert_eq!(u32::from_le_bytes([top[16], top[17], top[18], top[19]]), 2, "two recipients");
    assert_eq!(u32::from_le_bytes([top[20], top[21], top[22], top[23]]), 1, "one attachment");
    assert_eq!(utf16(&stream(&mut file, "/__substg1.0_0037001F")), "Lunch");
    assert_eq!(utf16(&stream(&mut file, "/__substg1.0_001A001F")), "IPM.Note");
    assert_eq!(String::from_utf8(stream(&mut file, "/__substg1.0_10130102")).unwrap(), "<p>See you at noon.</p>");
    // The recipients are storages with their own properties, and the book is not lost.
    for (index, (name, kind)) in [("Bob", 1u32), ("carol@example.org", 2)].into_iter().enumerate() {
        let storage = format!("/__recip_version1.0_#{index:08X}");
        assert!(file.exists(&storage));
        assert_eq!(utf16(&stream(&mut file, &format!("{storage}/__substg1.0_3001001F"))), name);
        let props = stream(&mut file, &format!("{storage}/__properties_version1.0"));
        let kind_seen = props[8..]
            .chunks_exact(16)
            .find(|e| u32::from_le_bytes([e[0], e[1], e[2], e[3]]) == 0x0C15_0003)
            .map(|e| u32::from_le_bytes([e[8], e[9], e[10], e[11]]));
        assert_eq!(kind_seen, Some(kind));
    }
    let attach = "/__attach_version1.0_#00000000";
    assert_eq!(utf16(&stream(&mut file, &format!("{attach}/__substg1.0_3707001F"))), "notes.txt");
    assert_eq!(stream(&mut file, &format!("{attach}/__substg1.0_37010102")), b"the notes");
}

#[test]
fn a_message_without_html_has_only_its_text() {
    let mut m = message();
    m.body.html = None;
    m.attachments.clear();
    m.cc.clear();
    let bytes = write(&m).unwrap();
    let mut file = CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    assert!(!file.exists("/__substg1.0_10130102"));
    assert_eq!(utf16(&stream(&mut file, "/__substg1.0_1000001F")), "See you at noon.");
    let top = stream(&mut file, "/__properties_version1.0");
    assert_eq!(u32::from_le_bytes([top[16], top[17], top[18], top[19]]), 1);
    assert_eq!(u32::from_le_bytes([top[20], top[21], top[22], top[23]]), 0);
}

#[test]
fn every_message_of_the_sample_writes_without_panicking() {
    // Whatever a message holds, what is written is a compound file a reader can open: the
    // reader of a file from outside is as hostile as that file.
    for subject in ["", "x", &"long ".repeat(100_000)] {
        let mut m = message();
        m.subject = subject.to_string();
        let bytes = write(&m).unwrap();
        assert!(CompoundFile::open(std::io::Cursor::new(bytes)).is_ok());
    }
}
