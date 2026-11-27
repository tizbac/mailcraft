// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Address books read from and written to a CardDAV server, against the strict stand-in of
//! `mailcraft-dav` (nothing here is a real account's).

use std::collections::HashMap;

use mailcraft_dav::http::HttpRequest;
use mailcraft_dav::standin::StandIn;
use mailcraft_model::remote::{ContactChangeKind, ContactOp, ContactOutcome, ContactsUpload, EventResult};
use mailcraft_model::{Account, DavServer, Event};

use super::*;

const ORIGIN: &str = "https://dav.example.org";
const USER: &str = "owner@example.org";
const PASSWORD: &str = "test-only-not-a-secret";
const BOOK: &str = "/dav/card/owner/people/";

fn server() -> StandIn {
    let s = StandIn::new(ORIGIN, USER, PASSWORD).unwrap();
    s.add_address_book("people", "People", false);
    s
}

fn account() -> Account {
    let home = format!("{ORIGIN}/dav/card/owner/");
    Account {
        id: 7,
        email: USER.into(),
        username: USER.into(),
        dav: Some(DavServer { contact_homes: vec![home], ..Default::default() }),
        ..Default::default()
    }
}

fn passwords() -> HashMap<String, String> {
    HashMap::from([(ORIGIN.to_string(), PASSWORD.to_string())])
}

/// The exchange against the stand-in, as the network would be.
fn exchange<'a>(s: &'a StandIn) -> impl Fn(HttpRequest, &str) -> Option<Reply> + 'a {
    move |request, _trusted| {
        let reply = s.handle(&request);
        Some(Reply { status: reply.status(), body: reply.body().to_vec() })
    }
}

fn vcard(uid: &str, name: &str, extra: &str) -> String {
    format!("BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nN:{name};;;;\r\nEMAIL:{uid}@example.org\r\n{extra}END:VCARD\r\n")
}

fn create(uid: &str, name: &str) -> ContactOp {
    ContactOp {
        kind: ContactChangeKind::Create,
        contact: 1,
        book: format!("{ORIGIN}{BOOK}"),
        uid: uid.into(),
        card: vcard(uid, name, ""),
        token: format!("tok-{uid}"),
        ..Default::default()
    }
}

fn outcome(uploaded: &ContactsUploaded, at: usize) -> &ContactOutcome {
    uploaded.outcomes.get(at).expect("an outcome")
}

#[test]
fn the_books_a_home_lists_and_their_cards_come_back() {
    let s = server();
    let reply = exchange(&s);
    let read = read_with(&account(), &passwords(), &reply);
    assert_eq!(read.problem, mailcraft_model::DavProblem::None);
    assert!(read.download.complete);
    assert_eq!(read.download.books.len(), 1);
    assert_eq!(read.download.books[0].name, "People");
    assert!(read.download.books[0].rights == mailcraft_model::Rights::Edit, "the stand-in lets it be written");
    assert!(read.download.contacts.is_empty(), "a book with no cards has none");
}

#[test]
fn a_new_contact_is_put_with_if_none_match_and_the_card_comes_back_with_it() {
    let s = server();
    let reply = exchange(&s);
    let op = create("ada-1", "Ada Example");
    let upload = ContactsUpload { account: 7, email: USER.into(), ops: vec![op] };
    let done = push_with(&account(), &upload, &passwords(), &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Done);
    assert!(outcome(&done, 0).remote.ends_with("ada-1.vcf"), "{}", outcome(&done, 0).remote);
    assert_eq!(s.writes(), 1);
    // The read brings it, with the server's ETag.
    let read = read_with(&account(), &passwords(), &reply);
    assert_eq!(read.download.contacts.len(), 1);
    assert_eq!(read.download.contacts[0].display_name, "Ada Example");
    assert!(!read.download.contacts[0].change_key.is_empty());
}

#[test]
fn a_change_keeps_what_the_server_sent_and_the_server_refuses_a_stale_one() {
    let s = server();
    let reply = exchange(&s);
    let upload = ContactsUpload { account: 7, email: USER.into(), ops: vec![create("ada-1", "Ada Example")] };
    push_with(&account(), &upload, &passwords(), &reply);
    let read = read_with(&account(), &passwords(), &reply);
    let card = &read.download.contacts[0];
    let etag = card.change_key.clone();
    // A change with the server's ETag goes, and its card keeps a property MailCraft doesn't read
    // (the server stores whatever was put; this card carries one before the change).
    let changed = ContactOp {
        kind: ContactChangeKind::Update,
        contact: 1,
        book: format!("{ORIGIN}{BOOK}"),
        remote: card.id.clone(),
        uid: card.uid.clone(),
        card: vcard("ada-1", "Ada Lovelace", "X-CUSTOM:keep\r\n"),
        etag: etag.clone(),
        token: "tok-2".into(),
        ..Default::default()
    };
    let done = push_with(&account(), &ContactsUpload { account: 7, email: USER.into(), ops: vec![changed] }, &passwords(), &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Done, "{}", outcome(&done, 0).error);
    // A second change with the ETag it had before is not put over the server's.
    let stale = ContactOp {
        kind: ContactChangeKind::Update,
        contact: 1,
        book: format!("{ORIGIN}{BOOK}"),
        remote: card.id.clone(),
        uid: card.uid.clone(),
        card: vcard("ada-1", "Ada, again", ""),
        etag,
        token: "tok-3".into(),
        ..Default::default()
    };
    let done = push_with(&account(), &ContactsUpload { account: 7, email: USER.into(), ops: vec![stale] }, &passwords(), &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Failed);
    assert_eq!(outcome(&done, 0).status, 412, "the server's copy changed since");
    assert!(outcome(&done, 0).error.contains("changed since"));
}

#[test]
fn a_change_kept_by_the_owners_word_goes_over_the_servers_copy() {
    let s = server();
    let reply = exchange(&s);
    let upload = ContactsUpload { account: 7, email: USER.into(), ops: vec![create("ada-1", "Ada Example")] };
    push_with(&account(), &upload, &passwords(), &reply);
    let read = read_with(&account(), &passwords(), &reply);
    let card = &read.download.contacts[0];
    let op = ContactOp {
        kind: ContactChangeKind::Update,
        contact: 1,
        book: format!("{ORIGIN}{BOOK}"),
        remote: card.id.clone(),
        uid: card.uid.clone(),
        card: vcard("ada-1", "Kept Mine", ""),
        etag: card.change_key.clone(),
        token: "tok-4".into(),
        force: true,
    };
    let done = push_with(&account(), &ContactsUpload { account: 7, email: USER.into(), ops: vec![op] }, &passwords(), &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Done, "{}", outcome(&done, 0).error);
    let read = read_with(&account(), &passwords(), &reply);
    assert_eq!(read.download.contacts[0].display_name, "Kept Mine");
}

#[test]
fn a_deletion_goes_with_the_servers_etag_and_an_unwritable_book_refuses_it() {
    let s = server();
    let reply = exchange(&s);
    let upload = ContactsUpload { account: 7, email: USER.into(), ops: vec![create("ada-1", "Ada Example")] };
    push_with(&account(), &upload, &passwords(), &reply);
    let read = read_with(&account(), &passwords(), &reply);
    let card = &read.download.contacts[0];
    let delete = ContactOp {
        kind: ContactChangeKind::Delete,
        contact: 1,
        book: format!("{ORIGIN}{BOOK}"),
        remote: card.id.clone(),
        uid: card.uid.clone(),
        etag: card.change_key.clone(),
        token: "tok-5".into(),
        ..Default::default()
    };
    let done = push_with(&account(), &ContactsUpload { account: 7, email: USER.into(), ops: vec![delete] }, &passwords(), &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Gone, "{}", outcome(&done, 0).error);
    assert!(read_with(&account(), &passwords(), &reply).download.contacts.is_empty());
    // No password for the book's origin: nothing is sent, and it says so.
    let no_password = HashMap::new();
    let done = push_with(&account(), &ContactsUpload { account: 7, email: USER.into(), ops: vec![create("b", "B")] }, &no_password, &reply);
    assert_eq!(outcome(&done, 0).result, EventResult::Failed);
    assert!(outcome(&done, 0).error.contains("password"));
    let _ = Event::default();
}
