// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A CardDAV card as the engine takes a contact in: the fields, its book, and the server's marks.

use crate::dav_contacts::{self, remote};

#[test]
fn a_card_becomes_a_contact_of_its_book_with_the_servers_marks() {
    let cards = mailcraft_vcard::parse(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ada-card\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada;;;\r\nORG:Analytical Engines\r\nTITLE:Programmer\r\nEMAIL;TYPE=WORK:ada@example.org\r\nTEL;TYPE=CELL:+44 20 7946 0958\r\nEND:VCARD\r\n",
    );
    assert_eq!(cards.len(), 1);
    let contact = remote(&cards[0], "https://dav.example.org/cards/ada/one.vcf", "\"7-1\"", "My contacts", "https://dav.example.org/cards/ada/");
    assert_eq!(contact.id, "https://dav.example.org/cards/ada/one.vcf");
    assert_eq!(contact.change_key, "\"7-1\"");
    assert_eq!(contact.uid, "ada-card");
    assert_eq!(contact.display_name, "Ada Lovelace");
    assert_eq!(contact.company, "Analytical Engines");
    assert_eq!(contact.title, "Programmer");
    assert_eq!(contact.emails.first().map(|e| e.value.as_str()), Some("ada@example.org"));
    assert_eq!(contact.phones.first().map(|p| p.value.as_str()), Some("+44 20 7946 0958"));
    assert_eq!((contact.book.as_str(), contact.book_id.as_str()), ("My contacts", "https://dav.example.org/cards/ada/"));
}

#[test]
fn an_account_with_no_contact_homes_reads_nothing() {
    let account = mailcraft_model::Account {
        email: "ada@example.org".into(),
        kind: mailcraft_model::AccountKind::Dav,
        dav: Some(mailcraft_model::DavServer::default()),
        ..Default::default()
    };
    let read = dav_contacts::read(&account, &std::collections::HashMap::new());
    assert!(read.download.complete && read.download.contacts.is_empty());
}
