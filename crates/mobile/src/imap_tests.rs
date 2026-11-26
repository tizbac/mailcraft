// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An IMAP account added on the phone and sent and received against the in-process fake servers:
//! the password is kept by the platform, the mail comes down, and the account and its password go
//! when it is removed. Nothing leaves this machine.

// (The fake servers' file has the HTTP and kept-answer fakes this test doesn't use.)
#![allow(dead_code)]

#[path = "../../net/src/fake.rs"]
mod fake;

use mailcraft_engine::Session;
use mailcraft_host::sync::SendReceive;

use crate::accounts::{ImapAccountSettings, MailServer};
use crate::fake_tests::{FakePlatform, block_on};
use crate::{Core, Event};

const PASSWORD: &str = "hunter2";

fn eml(subject: &str) -> Vec<u8> {
    format!(
        "From: Bob <bob@example.org>\r\nTo: Ada <ada@example.org>\r\nSubject: {subject}\r\n\
         Date: Tue, 01 Jul 2003 10:52:37 +0200\r\nMessage-ID: <{}@example.org>\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe body of {subject}.\r\n",
        subject.to_lowercase().replace(' ', "-")
    )
    .into_bytes()
}

fn message(uid: u32, flags: &str, subject: &str) -> fake::FakeMessage {
    fake::FakeMessage { uid, flags: flags.into(), internal_date: " 1-Jul-2003 09:52:37 +0000".into(), body: eml(subject), claimed_size: None }
}

fn mailbox(raw: &str, flags: &str, uid_validity: u32, messages: Vec<fake::FakeMessage>) -> fake::FakeMailbox {
    fake::FakeMailbox { raw: raw.into(), flags: flags.into(), uid_validity, messages, refuse_open: false, next_uid: 0 }
}

#[test]
fn an_imap_account_is_added_and_its_mail_comes_down() {
    let imap = fake::Fake::imap(fake::ImapWorld {
        user: "ada@example.org".into(),
        password: PASSWORD.into(),
        can_move: true,
        uidplus: true,
        mailboxes: vec![
            mailbox("INBOX", "\\HasNoChildren", 11, vec![message(1, "\\Seen", "One"), message(2, "", "Two")]),
            mailbox("Sent Items", "\\HasNoChildren \\Sent", 31, Vec::new()),
            mailbox("Deleted Items", "\\HasNoChildren \\Trash", 41, Vec::new()),
        ],
        ..Default::default()
    });
    let smtp = fake::Fake::smtp(fake::SmtpWorld { user: "ada@example.org".into(), password: PASSWORD.into(), auth: true, ..Default::default() });
    let platform = FakePlatform::new();
    let core = Core::start_with(platform.clone(), Session::empty, SendReceive::new()).expect("a core");
    let id = block_on(core.add_imap_account(ImapAccountSettings {
        email: "ada@example.org".into(),
        display_name: "Ada".into(),
        username: String::new(),
        password: PASSWORD.into(),
        incoming: MailServer { host: "127.0.0.1".into(), port: imap.port, security: "none".into() },
        outgoing: MailServer { host: "127.0.0.1".into(), port: smtp.port, security: "none".into() },
    }))
    .expect("the account is added");
    let account = id;
    let kind = block_on(core.call(move |s| Ok(s.mailbox.account(account).map(|a| a.kind)))).unwrap();
    assert_eq!(kind, Some(mailcraft_model::AccountKind::Imap));
    let summary = block_on(core.sync_imap_now(10_000)).expect("a sync");
    assert!(summary.ran && summary.finished, "{summary:?}");
    assert!(summary.errors.is_empty(), "{summary:?}");
    let subjects = block_on(core.call(|s| Ok(s.mailbox.messages.values().map(|m| m.subject.clone()).collect::<Vec<_>>()))).unwrap();
    assert!(subjects.iter().any(|s| s == "One") && subjects.iter().any(|s| s == "Two"), "{subjects:?}");
    // The password was kept by the platform while the owner lets sign-ins be remembered.
    let account = id;
    let key = block_on(core.call(move |s| {
        let account = s.mailbox.account(account).expect("the account");
        Ok(mailcraft_host::secrets::secret_key("password", account))
    }))
    .unwrap();
    assert!(platform.secrets.kept.lock().unwrap().iter().any(|(k, v)| *k == key && v == PASSWORD), "the password is not kept");
    // It goes with the account.
    block_on(core.remove_account(id)).unwrap();
    assert!(!platform.secrets.kept.lock().unwrap().iter().any(|(k, _)| *k == key), "the password is still kept");
    core.shutdown();
}

#[test]
fn without_a_password_an_imap_account_is_not_sent_and_received() {
    let platform = FakePlatform::new();
    let core = Core::start_with(platform, Session::empty, SendReceive::new()).expect("a core");
    // (Nothing listens here, and nothing is tried: the summary says what is missing first.)
    let imap = fake::Fake::imap(fake::ImapWorld {
        user: "ada@example.org".into(),
        password: PASSWORD.into(),
        mailboxes: vec![mailbox("INBOX", "\\HasNoChildren", 11, Vec::new())],
        ..Default::default()
    });
    let smtp = fake::Fake::smtp(fake::SmtpWorld { user: "ada@example.org".into(), password: PASSWORD.into(), ..Default::default() });
    let id = block_on(core.add_imap_account(ImapAccountSettings {
        email: "ada@example.org".into(),
        display_name: String::new(),
        username: String::new(),
        password: String::new(),
        incoming: MailServer { host: "127.0.0.1".into(), port: imap.port, security: "none".into() },
        outgoing: MailServer { host: "127.0.0.1".into(), port: smtp.port, security: "none".into() },
    }));
    assert!(id.is_err(), "an account without a password is refused");
    // Nothing was added.
    assert!(
        block_on(core.call(|s| Ok(!s.mailbox.accounts.iter().any(|a| a.kind == mailcraft_model::AccountKind::Imap)))).unwrap(),
        "an IMAP account was added without a password"
    );
    core.shutdown();
}

#[test]
fn the_events_of_an_imap_run_say_the_mailbox_changed() {
    let imap = fake::Fake::imap(fake::ImapWorld {
        user: "ada@example.org".into(),
        password: PASSWORD.into(),
        mailboxes: vec![mailbox("INBOX", "\\HasNoChildren", 11, vec![message(1, "", "Hello")])],
        ..Default::default()
    });
    let smtp = fake::Fake::smtp(fake::SmtpWorld { user: "ada@example.org".into(), password: PASSWORD.into(), ..Default::default() });
    let platform = FakePlatform::new();
    let core = Core::start_with(platform, Session::empty, SendReceive::new()).expect("a core");
    block_on(core.add_imap_account(ImapAccountSettings {
        email: "ada@example.org".into(),
        display_name: String::new(),
        username: String::new(),
        password: PASSWORD.into(),
        incoming: MailServer { host: "127.0.0.1".into(), port: imap.port, security: "none".into() },
        outgoing: MailServer { host: "127.0.0.1".into(), port: smtp.port, security: "none".into() },
    }))
    .unwrap();
    // What the app sees while the run goes: at least the mailbox's revision, and its end.
    let events = core.events.clone();
    let reader = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Some(event) = block_on(events.next()) {
            let done = matches!(event, Event::SyncFinished { .. });
            seen.push(event);
            if done {
                break;
            }
        }
        seen
    });
    block_on(core.sync_imap_now(10_000)).unwrap();
    let seen = reader.join().unwrap();
    assert!(seen.iter().any(|e| matches!(e, Event::RevChanged { .. })), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::SyncFinished { .. })), "{seen:?}");
    core.shutdown();
}
