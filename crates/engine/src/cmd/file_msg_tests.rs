// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Save Message As, as an Outlook `.msg` file: what File › Options says the default is, and that
//! the file is a compound file before anything reads it.

use serde_json::json;

use crate::Session;
use crate::sample;

fn session() -> (Session, u64) {
    let now = 1_800_000_000;
    let s = Session::at(sample::mailbox(now, 0), now, 0);
    let id = *s.mailbox.messages.keys().next().unwrap_or(&0);
    (s, id)
}

fn saved(s: &mut Session, params: serde_json::Value) -> (String, Vec<u8>) {
    let r = s.run("file.exportMessage", &params).unwrap();
    let name = r["name"].as_str().unwrap_or_default().to_string();
    let data = mailcraft_charset::base64::STANDARD.decode(r["data"].as_str().unwrap_or_default()).unwrap();
    (name, data)
}

#[test]
fn a_message_saved_as_msg_is_a_compound_file_with_that_extension() {
    let (mut s, id) = session();
    let (name, bytes) = saved(&mut s, json!({"id": id, "format": "msg"}));
    assert!(name.ends_with(".msg"), "{name}");
    // The compound file binary format's first eight bytes ([MS-CFB] §2.1).
    assert_eq!(bytes.get(..8), Some(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1][..]), "a .msg is a compound file");
}

#[test]
fn the_default_is_what_file_options_says() {
    let (mut s, id) = session();
    let (name, _) = saved(&mut s, json!({"id": id}));
    assert!(name.ends_with(".eml"), "the default stays .eml: {name}");
    let reply = s.run("file.options", &json!({"saveMessageFormat": "msg"})).unwrap();
    assert_eq!(reply["saveMessageFormat"], json!("msg"));
    let (name, bytes) = saved(&mut s, json!({"id": id}));
    assert!(name.ends_with(".msg"), "{name}");
    assert_eq!(bytes.get(..2), Some(&[0xD0, 0xCF][..]));
    // And an explicit format still wins over the default.
    let (name, _) = saved(&mut s, json!({"id": id, "format": "eml"}));
    assert!(name.ends_with(".eml"), "{name}");
}

#[test]
fn a_format_that_isnt_one_is_refused() {
    let (mut s, id) = session();
    assert!(s.run("file.exportMessage", &json!({"id": id, "format": "pst"})).is_err());
    assert!(s.run("file.options", &json!({"saveMessageFormat": "pst"})).is_err());
}
