// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `.msg` file as it is written and as it is read back: the properties in the top-level stream,
//! a recipient storage and an attachment storage, each with its own values.
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::io::{Read as _, Write as _};
use std::sync::Arc;

use cfb::CompoundFile;
use mailcraft_model::{Address, Attachment, Body, Message};
use proptest::prelude::*;

use super::{
    ATTACHMENT_STORAGE_PREFIX, MSG_CLSID, PR_ATTACH_DATA, PR_ATTACH_LONG_FILENAME, PR_ATTACH_MIME_TAG, PR_BODY, PR_CODE_PAGE, PR_DELIVERY_TIME,
    PR_DISPLAY_CC, PR_DISPLAY_NAME, PR_DISPLAY_TO, PR_EMAIL_ADDRESS, PR_HTML, PR_IMPORTANCE, PR_MESSAGE_CLASS, PR_MESSAGE_FLAGS, PR_RECIPIENT_TYPE,
    PR_RTF_COMPRESSED, PR_SENDER_EMAIL_ADDRESS, PR_SENDER_NAME, PR_SENT_TIME, PR_SMTP_ADDRESS, PR_SUBJECT, PR_TRANSMITABLE_DISPLAY_NAME,
    PROPERTIES_STREAM, PT_BINARY, PT_LONG, PT_OBJECT, PT_STRING8, PT_SYSTIME, PT_UNICODE, RECIPIENT_BCC, RECIPIENT_CC, RECIPIENT_STORAGE_PREFIX,
    RECIPIENT_TO, VALUE_STREAM_PREFIX, read, write,
};

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
    // A string in a property stream ends with a null ([MS-OXMSG] §2.4.2.1).
    let text = String::from_utf16(&units).unwrap();
    text.strip_suffix('\0').unwrap_or(&text).to_string()
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

/// The compound file says at its root that it is a message, keeps the (empty) name id streams a
/// client looks for, and writes its flags as the long they are — the things Outlook refused when
/// they were missing.
#[test]
fn a_message_is_marked_as_one_at_its_root() {
    let bytes = write(&message()).unwrap();
    let mut file = CompoundFile::open(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(*file.root_entry().clsid(), uuid::Uuid::from_bytes(MSG_CLSID), "the message class");
    assert!(file.exists("/__nameid_version1.0"));
    for name in ["00020102", "00030102", "00040102"] {
        assert!(file.exists(format!("/__nameid_version1.0/__substg1.0_{name}")));
    }
    let top = stream(&mut file, "/__properties_version1.0");
    let flags = top[32..]
        .chunks_exact(16)
        .find(|e| u32::from_le_bytes([e[0], e[1], e[2], e[3]]) == 0x0E07_0003)
        .map(|e| u32::from_le_bytes([e[8], e[9], e[10], e[11]]));
    assert!(flags.is_some_and(|flags| flags & 0x12 == 0x12), "PR_MESSAGE_FLAGS as a long, read: {flags:?}");
    // A string is null-terminated, and the length the entry gives includes the null.
    let subject = stream(&mut file, "/__substg1.0_0037001F");
    assert!(subject.len() >= 2 && subject.ends_with(&[0, 0]), "the subject ends with a null: {subject:?}");
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
        assert!(CompoundFile::open(std::io::Cursor::new(bytes.clone())).is_ok());
        let _ = read(&bytes);
    }
}

// ---------------------------------------------------------------- reading

/// A compound file built by hand the way Outlook leaves one: `None` for a storage, bytes for a
/// stream. Paths are given without their leading slash.
fn made(paths: &[(String, Option<Vec<u8>>)]) -> Vec<u8> {
    let mut file = CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
    file.set_storage_clsid("/", uuid::Uuid::from_bytes(MSG_CLSID)).unwrap();
    for (path, bytes) in paths {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for i in 1..parts.len() {
            let _ = file.create_storage(format!("/{}", parts[..i].join("/")));
        }
        let full = format!("/{}", parts.join("/"));
        match bytes {
            None => {
                file.create_storage(full).unwrap();
            }
            Some(bytes) => {
                let mut out = file.create_stream(full).unwrap();
                out.write_all(bytes).unwrap();
            }
        }
    }
    file.into_inner().into_inner()
}

/// Eight bytes of a property entry holding a number, a boolean or a time.
fn number(value: i32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&value.to_le_bytes());
    out
}

/// A property stream: the 32-byte header, then one 16-byte entry per property.
fn properties(entries: &[(u16, u16, [u8; 8])]) -> Vec<u8> {
    let mut out = vec![0u8; 32];
    for &(id, kind, entry) in entries {
        out.extend_from_slice(&((u32::from(id) << 16) | u32::from(kind)).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&entry);
    }
    out
}

/// The name of the stream holding a property's value: the property and the type it is kept as.
fn value(id: u16, kind: u16) -> String {
    format!("{VALUE_STREAM_PREFIX}{id:04X}{kind:04X}")
}

/// A storage in a message's compound file, given its kind and number.
fn storage(kind: &str, index: usize) -> String {
    format!("{kind}#{index:08X}")
}

/// A path inside a storage of the compound file.
fn under(dir: &str, path: &str) -> String {
    format!("{dir}/{path}")
}

fn words(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in text.encode_utf16() {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.push(0);
    out.push(0);
    out
}

fn filetime(seconds: i64) -> [u8; 8] {
    const EPOCH: u64 = 11_644_473_600;
    ((u64::try_from(seconds).unwrap_or(0) + EPOCH) * 10_000_000).to_le_bytes()
}

#[test]
fn a_written_message_reads_back_as_itself() {
    let made = message();
    let bytes = write(&made).unwrap();
    let back = read(&bytes).unwrap();
    assert_eq!(back.subject, "Lunch");
    assert_eq!(back.from, Address::new("Ada Lovelace", "ada@example.org"));
    assert_eq!(back.to, vec![Address::new("Bob", "bob@example.org")]);
    assert_eq!(back.cc, vec![Address::new("", "carol@example.org")]);
    assert_eq!(back.message_id, "m1@example.org");
    assert_eq!(back.date, 1_800_000_000);
    assert_eq!(back.received, 1_800_000_000);
    assert_eq!(back.body.text, "See you at noon.");
    assert_eq!(back.body.html.as_deref(), Some("<p>See you at noon.</p>"));
    assert_eq!(back.attachments.len(), 1);
    let file = &back.attachments[0];
    assert_eq!(file.name, "notes.txt");
    assert_eq!(file.mime, "text/plain");
    assert_eq!(file.content_id, "notes@example.org");
    assert!(!file.inline, "a plain file is not shown in the body");
    assert_eq!(file.data, Arc::new(b"the notes".to_vec()));
    assert!(!back.read, "the flags say it was not read here");
    assert!(!back.draft);
}

#[test]
fn a_note_outlook_wrote_is_read() {
    // A file of the shape Outlook leaves: strings in the older one-byte form, the HTML base64,
    // the recipients only in the display lines, a sender under the sent-representing ids.
    let bytes = made(&[
        (
            PROPERTIES_STREAM.into(),
            Some(properties(&[
                (PR_MESSAGE_FLAGS, PT_LONG, number(1)), // read, no attachments
                (PR_IMPORTANCE, PT_LONG, number(2)),    // high importance
                (PR_DELIVERY_TIME, PT_SYSTIME, filetime(1_700_000_000)),
                (PR_SENT_TIME, PT_SYSTIME, filetime(1_700_000_000)),
            ])),
        ),
        (value(PR_MESSAGE_CLASS, PT_STRING8), Some(b"IPM.Note\0".to_vec())),
        (value(PR_SUBJECT, PT_STRING8), Some(b"Quarter results\0".to_vec())),
        (value(PR_HTML, PT_BINARY), Some(base64_html(b"<p>Attached is the quarter.</p>"))),
        (value(PR_SENDER_NAME, PT_STRING8), Some(b"Ada Lovelace\0".to_vec())),
        (value(PR_SENDER_EMAIL_ADDRESS, PT_STRING8), Some(b"ada@example.org\0".to_vec())),
        (value(PR_DISPLAY_TO, PT_STRING8), Some(b"Bob <bob@example.org>; Carol <carol@example.org>\0".to_vec())),
        (value(PR_DISPLAY_CC, PT_STRING8), Some(b"Dan <dan@example.org>\0".to_vec())),
    ]);
    let m = read(&bytes).unwrap();
    assert_eq!(m.subject, "Quarter results");
    assert_eq!(m.from, Address::new("Ada Lovelace", "ada@example.org"));
    assert_eq!(m.to, vec![Address::new("Bob", "bob@example.org"), Address::new("Carol", "carol@example.org")]);
    assert_eq!(m.cc, vec![Address::new("Dan", "dan@example.org")]);
    assert_eq!(m.body.html.as_deref(), Some("<p>Attached is the quarter.</p>"));
    assert_eq!(m.body.text, "Attached is the quarter.");
    assert_eq!(m.date, 1_700_000_000);
    assert!(m.read);
    assert_eq!(m.importance, mailcraft_model::Importance::High);
}

fn base64_html(html: &[u8]) -> Vec<u8> {
    mailcraft_charset::base64::STANDARD.encode(html).into_bytes()
}

#[test]
fn one_byte_strings_are_read_in_the_code_page_the_file_names() {
    // A note written by a Russian Outlook: the code page says 1251, and the strings are Cyrillic
    // in that set. Read as Windows-1252 they would be mojibake.
    let bytes = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[(PR_CODE_PAGE, PT_LONG, number(1251))]))),
        (value(PR_MESSAGE_CLASS, PT_STRING8), Some(b"IPM.Note\0".to_vec())),
        (value(PR_SUBJECT, PT_STRING8), Some(b"Subject \xe0\xe2\xf2\0".to_vec())),
    ]);
    let m = read(&bytes).unwrap();
    assert_eq!(m.subject, "Subject авт", "the bytes are Cyrillic in 1251, not Latin-1: {:?}", m.subject);

    // UTF-8 named as the code page reads as UTF-8; a page nobody knows falls back to 1252.
    let utf8 = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[(PR_CODE_PAGE, PT_LONG, number(65001))]))),
        (value(PR_MESSAGE_CLASS, PT_STRING8), Some(b"IPM.Note\0".to_vec())),
        (value(PR_SUBJECT, PT_STRING8), Some("Ümläut".to_string().into_bytes())),
    ]);
    assert_eq!(read(&utf8).unwrap().subject, "Ümläut");
    let odd = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[(PR_CODE_PAGE, PT_LONG, number(8859))]))),
        (value(PR_MESSAGE_CLASS, PT_STRING8), Some(b"IPM.Note\0".to_vec())),
        (value(PR_SUBJECT, PT_STRING8), Some(b"caf\xe9\0".to_vec())),
    ]);
    assert_eq!(read(&odd).unwrap().subject, "café");
}

#[test]
fn recipients_are_read_from_their_own_storages_by_type() {
    let mut paths = vec![
        (PROPERTIES_STREAM.into(), Some(properties(&[]))),
        (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Note"))),
        (value(PR_SUBJECT, PT_UNICODE), Some(words("A note"))),
    ];
    // Each kind of recipient keeps its name and address under a different id, as the programs that
    // write these files differ in which they fill in.
    for (index, kind, name_id, name, address_id, address) in [
        (0, RECIPIENT_TO, PR_DISPLAY_NAME, "Ada", PR_EMAIL_ADDRESS, "ada@example.org"),
        (1, RECIPIENT_CC, PR_TRANSMITABLE_DISPLAY_NAME, "Bob", PR_EMAIL_ADDRESS, "bob@example.org"),
        (2, RECIPIENT_BCC, PR_DISPLAY_NAME, "Carol", PR_SMTP_ADDRESS, "carol@example.org"),
    ] {
        let dir = storage(RECIPIENT_STORAGE_PREFIX, index);
        paths.push((dir.clone(), None));
        paths.push((under(&dir, PROPERTIES_STREAM), Some(properties(&[(PR_RECIPIENT_TYPE, PT_LONG, number(kind as i32))]))));
        paths.push((under(&dir, &value(name_id, PT_UNICODE)), Some(words(name))));
        paths.push((under(&dir, &value(address_id, PT_UNICODE)), Some(words(address))));
    }
    let m = read(&made(&paths)).unwrap();
    assert_eq!(m.to, vec![Address::new("Ada", "ada@example.org")]);
    assert_eq!(m.cc, vec![Address::new("Bob", "bob@example.org")]);
    assert_eq!(m.bcc, vec![Address::new("Carol", "carol@example.org")]);
}

#[test]
fn a_message_attached_to_a_message_is_carried_as_a_msg_file() {
    let attach = storage(ATTACHMENT_STORAGE_PREFIX, 0);
    let inner = under(&attach, &value(PR_ATTACH_DATA, PT_OBJECT));
    let bytes = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[]))),
        (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Note"))),
        (value(PR_SUBJECT, PT_UNICODE), Some(words("Outer"))),
        (value(PR_BODY, PT_UNICODE), Some(words("See the note inside."))),
        (attach.clone(), None),
        (under(&attach, &value(PR_ATTACH_LONG_FILENAME, PT_UNICODE)), Some(words("Inner note"))),
        (inner.clone(), None),
        (under(&inner, PROPERTIES_STREAM), Some(properties(&[]))),
        (under(&inner, &value(PR_MESSAGE_CLASS, PT_UNICODE)), Some(words("IPM.Note"))),
        (under(&inner, &value(PR_SUBJECT, PT_UNICODE)), Some(words("Inner"))),
        (under(&inner, &value(PR_BODY, PT_UNICODE)), Some(words("The inner body."))),
    ]);
    let m = read(&bytes).unwrap();
    assert_eq!(m.subject, "Outer");
    assert_eq!(m.attachments.len(), 1, "{:?}", m.attachments);
    let carried = &m.attachments[0];
    assert_eq!(carried.name, "Inner note.msg");
    assert_eq!(carried.mime, "application/octet-stream");
    // What is carried is a `.msg` file of its own, and reading it gives the inner message.
    let back = read(&carried.data).unwrap();
    assert_eq!(back.subject, "Inner");
    assert_eq!(back.body.text, "The inner body.");
}

#[test]
fn a_body_kept_only_as_rich_text_is_read_as_text() {
    let rtf = b"{\\rtf1\\ansi hello rich text}";
    let mut stored = Vec::new();
    stored.extend_from_slice(&(rtf.len() as u32).to_le_bytes());
    stored.extend_from_slice(&(rtf.len() as u32).to_le_bytes());
    stored.extend_from_slice(&0x414C_454Du32.to_le_bytes()); // MELA: the text as it is
    stored.extend_from_slice(&[0u8; 4]);
    stored.extend_from_slice(rtf);
    let bytes = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[]))),
        (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Note"))),
        (value(PR_SUBJECT, PT_UNICODE), Some(words("Rich only"))),
        (value(PR_RTF_COMPRESSED, PT_BINARY), Some(stored)),
    ]);
    let m = read(&bytes).unwrap();
    assert_eq!(m.body.text, "hello rich text");
    assert!(m.body.html.is_none());
}

#[test]
fn an_attachment_with_no_name_keeps_one_of_its_own() {
    let attach = storage(ATTACHMENT_STORAGE_PREFIX, 0);
    let bytes = made(&[
        (PROPERTIES_STREAM.into(), Some(properties(&[]))),
        (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Note"))),
        (attach.clone(), None),
        (under(&attach, PROPERTIES_STREAM), Some(properties(&[]))),
        (under(&attach, &value(PR_ATTACH_MIME_TAG, PT_UNICODE)), Some(words("image/png"))),
        (under(&attach, &value(PR_ATTACH_DATA, PT_BINARY)), Some(vec![0x89, b'P', b'N', b'G'])),
    ]);
    let m = read(&bytes).unwrap();
    assert_eq!(m.attachments.len(), 1);
    assert_eq!(m.attachments[0].name, "attachment.png");
    assert_eq!(m.attachments[0].data, Arc::new(vec![0x89, b'P', b'N', b'G']));
}

#[test]
fn a_file_that_is_not_a_message_is_refused() {
    assert!(matches!(read(b"nothing here at all"), Err(super::MsgError::NotMessage)));
    assert!(matches!(read(&[]), Err(super::MsgError::NotMessage)));
    // A compound file that is not a note: an appointment, a contact.
    let appointment =
        made(&[(PROPERTIES_STREAM.into(), Some(properties(&[]))), (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Appointment")))]);
    let err = read(&appointment).err().unwrap_or_else(|| panic!("an appointment is not mail"));
    assert!(format!("{err}").contains("IPM.Appointment"), "{err}");
    // A compound file with no message mark at all.
    let other = {
        let mut file = CompoundFile::create(std::io::Cursor::new(Vec::new())).unwrap();
        file.create_stream("/whatever").unwrap();
        file.into_inner().into_inner()
    };
    assert!(matches!(read(&other), Err(super::MsgError::NotMessage)));
}

#[test]
fn a_truncated_or_mutated_msg_file_never_panics() {
    let whole = write(&message()).unwrap();
    // Cut off at every length, and change every byte: whatever the result, it is an answer, not a
    // crash, and nothing is read past what the file holds.
    for keep in 0..whole.len() {
        let _ = read(&whole[..keep]);
    }
    for at in 0..whole.len() {
        for value in [0u8, 0xff, 0x7f] {
            let mut mutated = whole.clone();
            mutated[at] = value;
            let _ = read(&mutated);
        }
    }
}

#[test]
fn more_storages_than_the_format_keeps_are_not_read() {
    let mut paths: Vec<(String, Option<Vec<u8>>)> =
        vec![(PROPERTIES_STREAM.into(), Some(properties(&[]))), (value(PR_MESSAGE_CLASS, PT_UNICODE), Some(words("IPM.Note")))];
    for i in 0..600 {
        paths.push((storage(ATTACHMENT_STORAGE_PREFIX, i), None));
        paths.push((under(&storage(ATTACHMENT_STORAGE_PREFIX, i), &value(PR_ATTACH_DATA, PT_BINARY)), Some(vec![i as u8])));
    }
    let m = read(&made(&paths)).unwrap();
    assert_eq!(m.attachments.len(), super::MAX_ATTACHMENTS, "the file holds 600, only what the format keeps is read");
}

/// Writes the fuzz target's seed files (`fuzz/seeds/msg/`): `cargo test -p mailcraft-mime-model --
/// --ignored write_fuzz_seeds`.
#[test]
#[ignore]
fn write_fuzz_seeds() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/msg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("note-with-attachment.msg"), write(&message()).unwrap()).unwrap();
    let mut plain = message();
    plain.body.html = None;
    plain.attachments.clear();
    std::fs::write(dir.join("plain-note.msg"), write(&plain).unwrap()).unwrap();
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic_when_read(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = read(&bytes);
    }
}
