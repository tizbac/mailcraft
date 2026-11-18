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
//!
//! With `MAILCRAFT_E2E_PAUSE_SECONDS=20` added, the test that makes an appointment here and
//! reads it back waits between the reads, so the owner can open that appointment in the web
//! window meanwhile (the case where it becomes invisible here and stays in the web window).

use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_ews::directory_run::sync_directory;
use mailcraft_model::remote::EventResult;
use mailcraft_model::remote::{ContactChangeKind, ContactOp, ContactsUpload, EventChangeKind, EventFields, EventOp, EventTime};
use mailcraft_model::remote_tasks::TaskResult;
use mailcraft_model::remote_tasks::{TaskOp, TaskOpKind, TasksUpload};
use mailcraft_model::sync::{EwsDirectoryRequest, EwsDirectoryResult};
use mailcraft_model::{Contact, Event, Labeled, TaskPatch};

use crate::ews_runner::{Transport, client};
use mailcraft_engine::Session;

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
    assert!(!result.auth_refused, "the sign-in was refused");
    assert!(result.untrusted.is_empty(), "the certificate isn't trusted: {}", result.untrusted);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
}

/// Waits `MAILCRAFT_E2E_PAUSE_SECONDS` between the reads of the test below, so the owner can open
/// the appointment in the web window while the test syncs it back (five minutes at most).
fn pause(round: &str) {
    let seconds = std::env::var("MAILCRAFT_E2E_PAUSE_SECONDS").ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(0).min(300);
    if seconds == 0 {
        return;
    }
    eprintln!("{round}: open the appointment in the web window now ({seconds} s)");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
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

    // ---- Three events made here all come back. ----
    let now = mailcraft_time::now();
    let starts = [now + 3_600, now + 7_200, now + 86_400];
    let ops: Vec<EventOp> = starts
        .iter()
        .enumerate()
        .map(|(index, start)| EventOp {
            kind: EventChangeKind::Create,
            event: 100 + index as u64,
            token: format!("e2e-multi-{index}"),
            fields: EventFields {
                title: Some(format!("E2E Multi {index}")),
                time: Some(EventTime { start: *start, end: *start + 1_800, all_day: false }),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    let sent = run(
        &base,
        &EwsDirectoryRequest {
            changes: Some(mailcraft_model::remote::EventsUpload { account: 1, email: base.email.clone(), zone: "Europe/Berlin".into(), ops }),
            ..request(&base)
        },
    );
    let outcomes = sent.uploaded.as_ref().expect("an answer").outcomes.clone();
    assert_eq!(outcomes.len(), 3, "{outcomes:?}");
    assert!(outcomes.iter().all(|outcome| outcome.result == EventResult::Done), "{outcomes:?}");
    let read = run(&base, &request(&base));
    let titles: Vec<&str> = read.events.iter().flat_map(|download| &download.events).map(|event| event.title.as_str()).collect();
    for index in 0..3 {
        assert!(titles.contains(&format!("E2E Multi {index}").as_str()), "E2E Multi {index} came back: {titles:?}");
    }
    // The same three through the mailbox's own merge: all of them, not one.
    let now = mailcraft_time::now();
    let mut session = Session::at(mailcraft_engine::sample::mailbox(now, 0), now, 0);
    let account = session
        .run("account.add", &serde_json::json!({"email": base.email, "kind": "exchange", "ews": {"url": base.url, "username": base.username}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let engine_base = EwsDirectoryRequest { account, ..base.clone() };
    let first = run(&engine_base, &request(&engine_base));
    mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &first).unwrap();
    let merged: Vec<&str> =
        session.mailbox.events.values().map(|event| event.title.as_str()).filter(|title| title.starts_with("E2E Multi ")).collect();
    assert_eq!(merged.len(), 3, "three events in the mailbox: {merged:?}");
    drop(session);

    // Take them away again.
    let removes: Vec<EventOp> = read
        .events
        .iter()
        .flat_map(|download| &download.events)
        .filter(|event| event.title.starts_with("E2E Multi "))
        .map(|event| EventOp { kind: EventChangeKind::Delete, event: 1, remote: event.id.clone(), ..Default::default() })
        .collect();
    if !removes.is_empty() {
        let _ = run(
            &base,
            &EwsDirectoryRequest {
                changes: Some(mailcraft_model::remote::EventsUpload {
                    account: 1,
                    email: base.email.clone(),
                    zone: "Europe/Berlin".into(),
                    ops: removes,
                }),
                ..request(&base)
            },
        );
    }

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

    // ---- A contact made in the window, in the Exchange book, reaches the server. ----
    let now = mailcraft_time::now();
    let mut session = Session::at(mailcraft_engine::sample::mailbox(now, 0), now, 0);
    let account = session
        .run("account.add", &serde_json::json!({"email": base.email, "kind": "exchange", "ews": {"url": base.url, "username": base.username}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let engine_base = EwsDirectoryRequest { account, ..base.clone() };
    let first = run(&engine_base, &request(&engine_base));
    mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &first).unwrap();
    let book = session
        .mailbox
        .contact_books
        .iter()
        .find(|book| book.account == account && book.rights == mailcraft_model::Rights::Edit)
        .map(|book| book.id.clone())
        .unwrap_or_else(|| panic!("a writable Exchange address book: {:?}", session.mailbox.contact_books));
    let made = session
        .run("people.new", &serde_json::json!({"displayName": "E2E New Contact", "addressBook": book, "addressBookAccount": account}))
        .unwrap();
    let id = made["id"].as_u64().unwrap();
    let asked = session.run("sync.ewsDirectoryRequest", &serde_json::json!({"account": account})).unwrap();
    let asked: EwsDirectoryRequest = serde_json::from_value(asked).unwrap();
    assert!(asked.contact_changes.as_ref().is_some_and(|upload| !upload.ops.is_empty()), "the new contact is in the request");
    let sent = run(&base, &asked);
    mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &sent).unwrap();
    let read = run(&base, &request(&base));
    let found = read.contacts.as_ref().is_some_and(|contacts| contacts.contacts.iter().any(|contact| contact.display_name == "E2E New Contact"));
    assert!(found, "the contact made here is on the server");
    // Take it away again through the same path, so a second run starts as before.
    if session.run("people.delete", &serde_json::json!({"id": id})).is_ok() {
        let asked = session.run("sync.ewsDirectoryRequest", &serde_json::json!({"account": account})).unwrap();
        let asked: EwsDirectoryRequest = serde_json::from_value(asked).unwrap();
        let sent = run(&base, &asked);
        mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &sent).unwrap();
    }
}

/// An appointment made in MailCraft itself goes up, and stays there when the server is read
/// back a few times (the case where one made here became invisible while one made in the web
/// window stayed).
#[test]
#[ignore = "needs a Grommunio server (docs/exchange-servers.md)"]
fn an_appointment_made_here_survives_reading_it_back() {
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
    let now = mailcraft_time::now();
    // The owner's own clock, as the window shows it: the zone reaches the server with the change.
    let zone = mailcraft_time::Zone::named("Europe/Rome").unwrap_or_else(|| panic!("Europe/Rome"));
    let mut session = Session::in_zone(mailcraft_engine::sample::mailbox(now, 0), now, zone);
    let account = session
        .run("account.add", &serde_json::json!({"email": base.email, "kind": "exchange", "ews": {"url": base.url, "username": base.username}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let engine_base = EwsDirectoryRequest { account, ..base.clone() };
    let first = run(&engine_base, &request(&engine_base));
    mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &first).unwrap();
    let calendar = session
        .mailbox
        .calendars
        .iter()
        .find(|calendar| calendar.origin.account == account && calendar.rights == mailcraft_model::Rights::Edit)
        .map(|calendar| calendar.id)
        .unwrap_or_else(|| panic!("a writable Exchange calendar: {:?}", session.mailbox.calendars));
    let title = format!("E2E Made Here {now}");
    eprintln!("an appointment called {title} is made in MailCraft now");
    let start = mailcraft_time::DateTime::from_timestamp(now + 86_400, 60).iso();
    let made = session.run("calendar.new", &serde_json::json!({"title": title, "start": start, "calendar": calendar})).unwrap();
    let id = made["id"].as_u64().unwrap();
    // Two or three reads after each change is where one made here used to go: the server stops
    // returning it, and the next complete read takes it out of the mailbox.
    let sync = |session: &mut Session, round: &str| {
        let asked = session.run("sync.ewsDirectoryRequest", &serde_json::json!({"account": account})).unwrap();
        let asked: EwsDirectoryRequest = serde_json::from_value(asked).unwrap();
        let sent = run(&engine_base, &asked);
        assert!(sent.errors.is_empty(), "{round}: the run said {:?}", sent.errors);
        mailcraft_engine::cmd::ews_directory::apply_ews_directory(session, &sent).unwrap();
        assert!(session.mailbox.events.contains_key(&id), "{round}: the appointment made here is gone from the mailbox");
        let on_server = run(&base, &request(&base)).events.iter().flat_map(|list| &list.events).any(|event| event.title == title);
        assert!(on_server, "{round}: the appointment made here is gone from the server");
    };
    for round in 0..3 {
        sync(&mut session, &format!("round {round}"));
        pause(&format!("round {round}"));
    }
    // The date changed here, as the owner would move it: it must still be there after the next reads.
    let moved = mailcraft_time::DateTime::from_timestamp(now + 2 * 86_400, 60).iso();
    session.run("calendar.move", &serde_json::json!({"id": id, "start": moved})).unwrap();
    for round in 3..6 {
        sync(&mut session, &format!("round {round}"));
        pause(&format!("round {round}"));
    }
    // Take it away again through the same path, so a second run starts as before.
    if session.run("calendar.delete", &serde_json::json!({"id": id})).is_ok() {
        let asked = session.run("sync.ewsDirectoryRequest", &serde_json::json!({"account": account})).unwrap();
        let asked: EwsDirectoryRequest = serde_json::from_value(asked).unwrap();
        let sent = run(&engine_base, &asked);
        let _ = mailcraft_engine::cmd::ews_directory::apply_ews_directory(&mut session, &sent);
    }
}
