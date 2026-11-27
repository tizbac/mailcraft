// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Address books a server lists, and putting a new contact into one of them.

use mailcraft_model::remote::{ContactOutcome, ContactsUploaded, EventResult};
use mailcraft_model::{ContactBook, Rights};
use serde_json::json;

use crate::Session;
use crate::cmd::remote;

const NOW: i64 = 1_791_993_600;
const BOOK: &str = "https://mail.example.org/carddav/ada@example.org/contacts/personal/";

fn session() -> (Session, u64) {
    let mut s = Session::at(crate::sample::mailbox(NOW, 0), NOW, 0);
    let account = s.mailbox.accounts.first().map(|account| account.id).unwrap_or(0);
    s.mailbox.contact_books.push(ContactBook { account, id: BOOK.into(), name: "My contacts".into(), rights: Rights::Edit });
    (s, account)
}

#[test]
fn a_new_contact_can_go_into_the_address_book_chosen() {
    let (mut s, account) = session();
    let listed = s.run("people.addressBooks", &json!({})).unwrap();
    assert_eq!(listed["books"][0]["id"], BOOK);
    assert_eq!(listed["books"][0]["rights"], "edit");

    let made = s.run("people.new", &json!({"first": "Ada", "last": "Lovelace", "addressBook": BOOK})).unwrap();
    let id = made["id"].as_u64().unwrap();
    let contact = s.mailbox.contact(id).unwrap();
    assert_eq!((contact.book.as_str(), contact.book_id.as_str(), contact.origin.account), ("My contacts", BOOK, account));
    assert!(!contact.uid.is_empty());
    assert!(!contact.origin.is_local(), "it is for the server, not only here");

    // It waits for the next CardDAV run, which writes the card under its own UID.
    let upload = remote::book_contact_upload(&s, account).expect("a contact to send");
    assert_eq!((upload.ops.len(), upload.ops[0].book.as_str(), upload.ops[0].contact), (1, BOOK, id));
    assert!(upload.ops[0].card.contains(&format!("UID:{}\r\n", upload.ops[0].uid)), "{}", upload.ops[0].card);
    let uploaded = ContactsUploaded {
        account,
        email: upload.email.clone(),
        outcomes: vec![ContactOutcome {
            op: upload.ops[0].clone(),
            result: EventResult::Done,
            remote: format!("{BOOK}{}.vcf", upload.ops[0].uid),
            status: 201,
            ..Default::default()
        }],
    };
    remote::apply_book_contact_outcomes(&mut s, &uploaded).unwrap();
    let contact = s.mailbox.contact(id).unwrap();
    assert_eq!(contact.origin.id, format!("{BOOK}{}.vcf", upload.ops[0].uid));
    assert!(remote::book_contact_upload(&s, account).is_none(), "sent once");
}

#[test]
fn a_contact_without_a_book_stays_on_this_computer() {
    let (mut s, _) = session();
    let made = s.run("people.new", &json!({"first": "Grace", "last": "Hopper"})).unwrap();
    let contact = s.mailbox.contact(made["id"].as_u64().unwrap()).unwrap();
    assert!(contact.origin.is_local() && contact.book.is_empty() && contact.book_id.is_empty());
    assert!(!contact.uid.is_empty(), "a new card has an identity if it is ever exported");
}

#[test]
fn a_read_only_address_book_is_not_offered_a_new_contact() {
    let (mut s, account) = session();
    s.mailbox.contact_books.push(ContactBook {
        account,
        id: "https://mail.example.org/carddav/ada@example.org/contacts/~book-9/".into(),
        name: "Team".into(),
        rights: Rights::Read,
    });
    let why = s
        .run("people.new", &json!({"first": "Ada", "addressBook": "https://mail.example.org/carddav/ada@example.org/contacts/~book-9/"}))
        .unwrap_err()
        .to_string();
    assert!(why.contains("can't be written"), "{why}");
}

/// A session with an account whose calendars and address books are on a server, and one writable
/// book of it: the address book's changes are the server's to take.
fn dav_session() -> (Session, u64) {
    let mut s = Session::at(crate::sample::mailbox(NOW, 0), NOW, 0);
    s.dav_enabled = true;
    let account = s
        .run("account.add", &json!({"email": "dav@example.org", "incoming": {"host": "imap.example.org"}, "outgoing": {"host": "smtp.example.org"}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    s.run("account.davSet", &json!({"id": account, "contactHomes": ["https://mail.example.org/carddav/dav@example.org/"]})).unwrap();
    s.mailbox.contact_books.push(ContactBook { account, id: BOOK.into(), name: "My contacts".into(), rights: Rights::Edit });
    (s, account)
}

/// A contact of the book that the server has as `href` at the ETag `etag`, with a change of its
/// company waiting to go up.
fn changed_contact(s: &mut Session, account: u64, href: &str, etag: &str) -> u64 {
    let id = s.run("people.new", &json!({"first": "Ada", "last": "Lovelace", "addressBook": BOOK})).unwrap()["id"].as_u64().unwrap();
    {
        let contact = s.mailbox.contact_mut(id).unwrap();
        contact.origin.account = account;
        contact.origin.id = href.to_string();
        contact.origin.change_key = etag.to_string();
    }
    s.run("people.edit", &json!({"id": id, "company": "Analytical Engines"})).unwrap();
    id
}

/// The answer a server gives a change it has changed since: 412.
fn conflicted(s: &mut Session, account: u64, upload: &mailcraft_model::remote::ContactsUpload) -> ContactsUploaded {
    let op = upload.ops[0].clone();
    let uploaded = ContactsUploaded {
        account,
        email: upload.email.clone(),
        outcomes: vec![ContactOutcome {
            op,
            result: EventResult::Failed,
            remote: String::new(),
            status: 412,
            error: "the server's copy changed since".into(),
        }],
    };
    remote::apply_book_contact_outcomes(s, &uploaded).unwrap();
    uploaded
}

#[test]
fn a_contact_the_server_changed_too_is_kept_until_the_owner_says_which_to_keep() {
    let (mut s, account) = dav_session();
    let id = changed_contact(&mut s, account, &format!("{BOOK}ada.vcf"), "\"7\"");
    let upload = remote::book_contact_upload(&s, account).expect("the change");
    conflicted(&mut s, account, &upload);
    // Held: nothing more goes up, and the owner is asked.
    assert!(s.dav_contact_conflicts.len() == 1);
    assert!(remote::book_contact_upload(&s, account).is_none(), "held, not sent over it");
    let asked = s.run("sync.davConflicts", &json!({})).unwrap();
    let asked = asked["conflicts"].as_array().unwrap();
    let row = asked.iter().find(|row| row["kind"] == "contact").expect("the contact is listed");
    let href = row["href"].as_str().expect("the resource, as the window sends it back").to_string();
    assert!(href.ends_with("/ada.vcf"), "{href}");
    // Keep mine: the change goes up over the server's copy whatever version it has.
    s.run("sync.davResolve", &json!({"href": href.clone(), "keep": "mine"})).unwrap();
    let up = remote::book_contact_upload(&s, account).expect("kept, it goes up");
    assert!(up.ops[0].force, "over the server's copy");
    assert!(up.ops[0].card.contains("ORG:Analytical Engines"));
    assert_eq!(up.ops[0].contact, id);
}

#[test]
fn a_contact_change_given_up_for_the_servers_copy_waits_to_be_read() {
    let (mut s, account) = dav_session();
    let id = changed_contact(&mut s, account, &format!("{BOOK}ada.vcf"), "\"7\"");
    let upload = remote::book_contact_upload(&s, account).expect("the change");
    conflicted(&mut s, account, &upload);
    let href = s.dav_contact_conflicts.keys().next().cloned().expect("the conflict's resource");
    s.run("sync.davResolve", &json!({"href": href, "keep": "theirs"})).unwrap();
    assert!(s.dav_contact_conflicts.is_empty());
    assert!(remote::book_contact_upload(&s, account).is_none(), "given up");
    // (Its ETag is the one the change was made on, not the server's: the next read brings the
    // card as the server has it now.)
    assert_eq!(s.mailbox.contact(id).unwrap().origin.change_key, "");
}

#[test]
fn a_contact_conflict_with_both_kept_makes_a_copy_in_the_same_book() {
    let (mut s, account) = dav_session();
    s.run("file.options", &json!({"davConflicts": "keepBoth"})).unwrap();
    let id = changed_contact(&mut s, account, &format!("{BOOK}ada.vcf"), "\"7\"");
    let upload = remote::book_contact_upload(&s, account).expect("the change");
    conflicted(&mut s, account, &upload);
    assert!(s.dav_contact_conflicts.is_empty(), "nothing is asked");
    assert!(s.mailbox.book_contact_changes.is_empty(), "the change is gone");
    let up = remote::book_contact_upload(&s, account).expect("the copy waits");
    assert_eq!(up.ops[0].kind, mailcraft_model::remote::ContactChangeKind::Create, "a new card");
    assert_ne!(up.ops[0].contact, id, "a copy of its own");
    assert_eq!(up.ops[0].book, BOOK);
    assert!(up.ops[0].card.contains("ORG:Analytical Engines"));
}
