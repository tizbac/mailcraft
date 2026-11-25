// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The folder chosen for what is sent from an address, and whether its copy is read.

use serde_json::json;

use crate::{Session, sample};

const NOW: i64 = 1_760_000_000;

fn session() -> Session {
    Session::at(sample::mailbox(NOW, 0), NOW, 0)
}

fn run(s: &mut Session, id: &str, params: serde_json::Value) -> serde_json::Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

#[test]
fn a_local_send_is_filed_where_the_address_says_and_unread_when_told() {
    let mut s = session();
    let account = s.mailbox.accounts.first().map(|a| a.id).unwrap();
    let email = s.mailbox.account(account).unwrap().email.clone();
    assert!(!email.is_empty(), "the sample's first account has an address");
    let folder = s.mailbox.add_folder(account, None, "Filed away").unwrap();
    run(&mut s, "file.options", json!({"sentUnread": true, "sentFolders": [{"address": email, "folder": folder}]}));
    run(&mut s, "mail.new", json!({"account": account, "to": "maya@example.org", "subject": "Filed", "body": "Hello"}));
    let id = run(&mut s, "compose.send", json!({}))["id"].as_u64().unwrap();
    let sent = s.mailbox.message(id).unwrap();
    assert_eq!(sent.folder, folder, "the chosen folder keeps the copy");
    assert!(!sent.read, "the copy is unread, as chosen");
    // Taking the choice back puts the account's own Sent Items back in use.
    run(&mut s, "file.options", json!({"sentFolders": [{"address": email, "folder": null}]}));
    assert_eq!(s.prefs.sent_folder(&email), None);
    // A folder that isn't here is refused, and nothing changes.
    assert!(s.run("file.options", &json!({"sentFolders": [{"address": email, "folder": 999_999}]})).is_err());
    assert_eq!(s.prefs.sent_folder(&email), None);
}

#[test]
fn a_server_copy_takes_the_read_state_chosen_for_it_when_it_comes_down() {
    let mut s = session();
    let account = s.mailbox.accounts.first().map(|a| a.id).unwrap();
    let outbox = s.mailbox.folder_of_kind(account, mailcraft_model::FolderKind::Outbox).unwrap();
    let sent = s.mailbox.folder_of_kind(account, mailcraft_model::FolderKind::Sent).unwrap();
    let going = mailcraft_model::Message {
        account,
        folder: outbox,
        message_id: "<kept@example.org>".into(),
        from: mailcraft_model::Address::new("Me", &s.mailbox.account(account).unwrap().email),
        to: vec![mailcraft_model::Address::new("Maya", "maya@example.org")],
        subject: "Going".into(),
        date: NOW,
        ..Default::default()
    };
    let id = s.mailbox.add_message(going).unwrap();
    let done =
        mailcraft_model::sync::SendDone { message: id, outcome: mailcraft_model::sync::SendOutcome::Sent, draft: None, draft_left: String::new() };
    // The owner chose unread: the copy that the server files is set so when it comes down.
    super::graph_send::send_done_at(&mut s.mailbox, account, &done, s.interface_language, &mut Vec::new(), true, Some(NOW));
    let copy = mailcraft_model::Message {
        account,
        folder: sent,
        message_id: "<kept@example.org>".into(),
        from: mailcraft_model::Address::new("Me", &s.mailbox.account(account).unwrap().email),
        to: vec![mailcraft_model::Address::new("Maya", "maya@example.org")],
        subject: "Going".into(),
        date: NOW,
        read: true,
        ..Default::default()
    };
    let copy = s.mailbox.add_message(copy).unwrap();
    assert!(!s.mailbox.message(copy).unwrap().read, "the server's copy is taken as unread");
}

#[test]
fn no_copy_of_a_local_send_is_kept() {
    let mut s = session();
    let account = s.mailbox.accounts.first().map(|a| a.id).unwrap();
    run(&mut s, "file.options", json!({"sentCopy": false}));
    run(&mut s, "mail.new", json!({"account": account, "to": "maya@example.org", "subject": "Gone", "body": "No copy"}));
    let id = run(&mut s, "compose.send", json!({}))["id"].as_u64().unwrap();
    assert!(s.mailbox.message(id).is_none(), "nothing is kept of what was sent");
}

#[test]
fn a_sent_message_leaves_the_outbox_without_a_sent_copy() {
    use mailcraft_model::sync::SyncResult;
    let mut s = session();
    let account = s.mailbox.accounts.first().map(|a| a.id).unwrap();
    let outbox = s.mailbox.folder_of_kind(account, mailcraft_model::FolderKind::Outbox).unwrap();
    let id = s
        .mailbox
        .add_message(mailcraft_model::Message {
            account,
            folder: outbox,
            message_id: "<no-copy@example.org>".into(),
            from: mailcraft_model::Address::new("Me", &s.mailbox.account(account).unwrap().email),
            to: vec![mailcraft_model::Address::new("Maya", "maya@example.org")],
            subject: "Gone".into(),
            date: NOW,
            ..Default::default()
        })
        .unwrap();
    let result = SyncResult { sent: vec![id], ..Default::default() };
    assert_eq!(super::account::file_sent(&mut s.mailbox, account, &result, false, false, NOW), 1);
    assert!(s.mailbox.message(id).is_none(), "it left the Outbox and no copy was kept");
}
