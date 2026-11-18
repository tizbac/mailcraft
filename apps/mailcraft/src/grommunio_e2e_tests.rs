// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! End to end against a real Grommunio server: contacts, calendars and to-dos made, changed,
//! read back and deleted through Exchange Web Services. Not part of `cargo test` (it needs a
//! server): run it with the variables `docs/exchange-servers.md` names:
//!
//! ```text
//! MAILCRAFT_E2E_EWS_URL=https://mail.grommunio.test:28443/EWS/Exchange.asmx \
//! MAILCRAFT_E2E_USER=ada@grommunio.test MAILCRAFT_E2E_PASS=TestPass123! \
//! MAILCRAFT_E2E_PIN=<the certificate's SHA-256 fingerprint> \
//!   cargo test -p mailcraft --bin mailcraft grommunio -- --ignored --nocapture
//! ```
//!
//! Nothing here is a real mailbox: the people, subjects and cards are made up.

use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_ews::directory_run::sync_directory;
use mailcraft_model::remote::EventResult;
use mailcraft_model::remote::{ContactChangeKind, ContactOp, ContactsUpload, EventChangeKind, EventFields, EventOp, EventTime};
use mailcraft_model::remote_tasks::TaskResult;
use mailcraft_model::remote_tasks::{TaskOp, TaskOpKind, TasksUpload};
use mailcraft_model::sync::{EwsDirectoryRequest, EwsDirectoryResult};
use mailcraft_model::{Contact, Event, Labeled, TaskPatch};

use crate::ews_runner::{Transport, client};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn request(base: &EwsDirectoryRequest) -> EwsDirectoryRequest {
    EwsDirectoryRequest { contacts: true, calendars: true, tasks: true, ..base.clone() }
}

fn run(base: &EwsDirectoryRequest, req: &EwsDirectoryRequest) -> EwsDirectoryResult {
    let transport = Transport::new(&base.trusted);
    let client = client(&base.url, &base.username, &base.email, &env("MAILCRAFT_E2E_PASS")).unwrap_or_else(|why| panic!("{why}"));
    let cancel = AtomicBool::new(false);
    let mut wait = |_: u32| true;
    let result = crate::block_on::block_on(sync_directory(&client, &transport, req, &cancel, &mut wait));
    assert!(result.auth_refused == false, "the sign-in was refused");
    assert!(result.untrusted.is_empty(), "the certificate isn't trusted: {}", result.untrusted);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
}

#[test]
#[ignore = "needs a Grommunio server (docs/exchange-servers.md)"]
fn contacts_calendars_and_todos_go_up_and_come_back() {
    let base = EwsDirectoryRequest {
        account: 1,
        email: env("MAILCRAFT_E2E_USER"),
        url: env("MAILCRAFT_E2E_EWS_URL"),
        username: String::new(),
        mailbox: String::new(),
        trusted: env("MAILCRAFT_E2E_PIN"),
        calendars: true,
        contacts: true,
        tasks: true,
        from: mailcraft_time::now() - 86_400,
        to: mailcraft_time::now() + 30 * 86_400,
        ..Default::default()
    };
    let result = run(&base, &request(&base));
    let books = result.contacts.as_ref().expect("address books").books.clone();
    assert!(!books.is_empty(), "an address book");
    let book = books.first().map(|b| b.id.clone()).unwrap_or_default();
    assert!(result.calendars.as_ref().is_some_and(|c| !c.calendars.is_empty()), "a calendar");
    assert!(result.tasks.as_ref().is_some_and(|t| !t.lists.is_empty()), "a to-do list");

    // ---- A contact made here comes back changed and then gone. ----
    let contact = Contact { display_name: "E2E Contact".into(), emails: vec![Labeled::new("", "e2e-contact@example.org")], ..Default::default() };
    let card = mailcraft_vcard::write(&[&contact]);
    let create = ContactOp {
        kind: ContactChangeKind::Create,
        contact: 1,
        book: book.clone(),
        uid: "e2e-contact-uid".into(),
        card,
        token: "e2e-contact-token".into(),
        ..Default::default()
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest { contact_changes: Some(ContactsUpload { account: 1, email: base.email.clone(), ops: vec![create] }), ..request(&base) },
    );
    let made = sent.contacts_uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(made.result, EventResult::Done, "contact made: {made:?}");
    assert!(!made.remote.is_empty(), "the contact's server id");

    let read = run(&base, &request(&base));
    let downloaded = read.contacts.as_ref().expect("contacts");
    let seen = downloaded.contacts.iter().find(|c| c.emails.iter().any(|e| e.value == "e2e-contact@example.org")).cloned();
    let seen = seen.unwrap_or_else(|| panic!("the contact came back: {:?}", downloaded.contacts.len()));
    let change_key = seen.change_key.clone();

    let mut changed = contact.clone();
    changed.company = "E2E Works".into();
    let update = ContactOp {
        kind: ContactChangeKind::Update,
        contact: 1,
        book: book.clone(),
        remote: made.remote.clone(),
        uid: seen.uid.clone(),
        card: mailcraft_vcard::write(&[&changed]),
        etag: change_key.clone(),
        token: "e2e-contact-token-2".into(),
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest { contact_changes: Some(ContactsUpload { account: 1, email: base.email.clone(), ops: vec![update] }), ..request(&base) },
    );
    let outcome = sent.contacts_uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, EventResult::Done, "contact changed: {outcome:?}");
    let read = run(&base, &request(&base));
    let now = read
        .contacts
        .as_ref()
        .and_then(|c| c.contacts.iter().find(|c| c.emails.iter().any(|e| e.value == "e2e-contact@example.org")).cloned())
        .expect("changed contact");
    assert_eq!(now.company, "E2E Works", "the change arrived");

    let delete = ContactOp {
        kind: ContactChangeKind::Delete,
        contact: 0,
        book: book.clone(),
        remote: made.remote.clone(),
        uid: now.uid.clone(),
        card: String::new(),
        etag: now.change_key.clone(),
        token: "e2e-contact-token-3".into(),
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest { contact_changes: Some(ContactsUpload { account: 1, email: base.email.clone(), ops: vec![delete] }), ..request(&base) },
    );
    let outcome = sent.contacts_uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, EventResult::Done, "contact deleted: {outcome:?}");
    let read = run(&base, &request(&base));
    assert!(
        read.contacts.as_ref().is_none_or(|c| !c.contacts.iter().any(|c| c.emails.iter().any(|e| e.value == "e2e-contact@example.org"))),
        "the contact is gone"
    );

    // ---- An event made here, changed, and deleted. ----
    let start = mailcraft_time::now() + 3_600;
    let event = Event {
        title: "E2E Meeting".into(),
        location: "Room 1".into(),
        start: mailcraft_time::DateTime::from_timestamp(start, 0),
        end: mailcraft_time::DateTime::from_timestamp(start + 3_600, 0),
        ..Default::default()
    };
    let create = EventOp {
        kind: EventChangeKind::Create,
        event: 1,
        token: "e2e-event-token".into(),
        fields: EventFields {
            title: Some(event.title.clone()),
            location: Some(event.location.clone()),
            time: Some(EventTime { start, end: start + 3_600, all_day: false }),
            ..Default::default()
        },
        ..Default::default()
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest {
            changes: Some(mailcraft_model::remote::EventsUpload {
                account: 1,
                email: base.email.clone(),
                zone: "Europe/Berlin".into(),
                ops: vec![create],
            }),
            ..request(&base)
        },
    );
    let outcome = sent.uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, EventResult::Done, "event made: {outcome:?}");
    let read = run(&base, &request(&base));
    let seen = read.events.iter().flat_map(|d| &d.events).find(|e| e.title == "E2E Meeting").cloned().expect("the event came back");
    let update = EventOp {
        kind: EventChangeKind::Update,
        event: 1,
        remote: seen.id.clone(),
        fields: EventFields { title: Some("E2E Meeting, moved".into()), location: Some("Room 2".into()), ..Default::default() },
        ..Default::default()
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest {
            changes: Some(mailcraft_model::remote::EventsUpload {
                account: 1,
                email: base.email.clone(),
                zone: "Europe/Berlin".into(),
                ops: vec![update],
            }),
            ..request(&base)
        },
    );
    let outcome = sent.uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, EventResult::Done, "event changed: {outcome:?}");
    let read = run(&base, &request(&base));
    let seen = read.events.iter().flat_map(|d| &d.events).find(|e| e.title == "E2E Meeting, moved").cloned().expect("the changed event came back");
    let delete = EventOp { kind: EventChangeKind::Delete, event: 1, remote: seen.id.clone(), ..Default::default() };
    let sent = run(
        &base,
        &EwsDirectoryRequest {
            changes: Some(mailcraft_model::remote::EventsUpload {
                account: 1,
                email: base.email.clone(),
                zone: "Europe/Berlin".into(),
                ops: vec![delete],
            }),
            ..request(&base)
        },
    );
    let outcome = sent.uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, EventResult::Done, "event deleted: {outcome:?}");

    // ---- A to-do made here and completed. ----
    let create = TaskOp {
        kind: TaskOpKind::Create,
        task: 1,
        patch: TaskPatch { title: Some("E2E To-do".into()), ..Default::default() },
        ..Default::default()
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest { task_changes: Some(TasksUpload { account: 1, email: base.email.clone(), ops: vec![create] }), ..request(&base) },
    );
    let outcome = sent.tasks_uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, TaskResult::Done, "to-do made: {outcome:?}");
    let read = run(&base, &request(&base));
    let seen = read.tasks.as_ref().and_then(|t| t.tasks.iter().find(|t| t.title == "E2E To-do").cloned()).expect("the to-do came back");
    let update = TaskOp {
        kind: TaskOpKind::Update,
        task: 1,
        id: seen.id.clone(),
        list: seen.list.clone(),
        patch: TaskPatch { completed: Some(Some(mailcraft_time::now())), ..Default::default() },
    };
    let sent = run(
        &base,
        &EwsDirectoryRequest { task_changes: Some(TasksUpload { account: 1, email: base.email.clone(), ops: vec![update] }), ..request(&base) },
    );
    let outcome = sent.tasks_uploaded.as_ref().expect("an answer").outcomes.first().cloned().expect("one outcome");
    assert_eq!(outcome.result, TaskResult::Done, "to-do completed: {outcome:?}");
    let _ = Ordering::Relaxed;
}
