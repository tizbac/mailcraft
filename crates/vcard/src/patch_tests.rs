// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A changed contact written into the card the server sent: only what changed, and every other
//! line kept as it came.

use std::collections::BTreeSet;

use mailcraft_model::Contact;
use mailcraft_model::contact_sync::ContactField;

use super::patch;

const CARD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ada-1\r\nN:Example;Ada;;;\r\nFN:Ada Example\r\nEMAIL:ada@example.org\r\nPHOTO;ENCODING=b:AAAA\r\nX-APPLE-SUBSCRIBED:YES\r\nEND:VCARD\r\n";

fn contact() -> Contact {
    Contact { uid: "ada-1".into(), first: "Ada".into(), last: "Example".into(), display_name: "Ada Example".into(), ..Default::default() }
}

fn fields(list: &[ContactField]) -> BTreeSet<ContactField> {
    list.iter().copied().collect()
}

#[test]
fn a_photo_and_another_programs_property_are_kept_as_they_came() {
    let mut c = contact();
    c.notes = "Ring twice".into();
    let doc = patch(CARD, &c, &fields(&[ContactField::Notes])).expect("patched");
    assert!(doc.contains("PHOTO;ENCODING=b:AAAA"), "the photo stays:\n{doc}");
    assert!(doc.contains("X-APPLE-SUBSCRIBED:YES"));
    assert!(doc.contains("NOTE:Ring twice"));
    assert!(doc.contains("FN:Ada Example"));
    assert_eq!(doc.matches("NOTE:").count(), 1);
    assert!(doc.starts_with("BEGIN:VCARD\r\n") && doc.trim_end().ends_with("END:VCARD"));
}

#[test]
fn a_changed_name_replaces_the_parts_and_the_shown_name_and_nothing_else() {
    let mut c = contact();
    c.last = "Lovelace".into();
    c.display_name = "Ada Lovelace".into();
    let doc = patch(CARD, &c, &fields(&[ContactField::Last, ContactField::DisplayName])).expect("patched");
    assert!(doc.contains("N:Lovelace;Ada;;;"), "{doc}");
    assert!(doc.contains("FN:Ada Lovelace"), "{doc}");
    assert!(!doc.contains("FN:Ada Example"));
    assert!(doc.contains("EMAIL:ada@example.org"));
}

#[test]
fn a_property_the_contact_no_longer_has_is_removed_and_its_custom_label_with_it() {
    let with_phone = CARD.replace("END:VCARD", "item1.TEL;TYPE=CELL:555\r\nitem1.X-ABLabel:Car\r\nEND:VCARD");
    let mut c = contact();
    c.phones.clear();
    let doc = patch(&with_phone, &c, &fields(&[ContactField::Phones])).expect("patched");
    assert!(!doc.contains("TEL"), "the number is gone:\n{doc}");
    assert!(!doc.contains("X-ABLabel:Car"), "and its label");
    assert!(doc.contains("PHOTO;ENCODING=b:AAAA"));
}

#[test]
fn a_custom_label_is_written_with_its_property() {
    let mut c = contact();
    c.phones = vec![mailcraft_model::Labeled { label: "car".into(), value: "555".into() }];
    let doc = patch(CARD, &c, &fields(&[ContactField::Phones])).expect("patched");
    assert!(doc.contains("TEL") && doc.contains("555"), "{doc}");
    assert!(doc.contains("X-ABLabel:car"), "{doc}");
}

#[test]
fn a_document_that_is_not_one_card_is_not_changed() {
    assert!(patch("BEGIN:VCARD\r\nFN:x\r\n", &contact(), &fields(&[ContactField::Notes])).is_none(), "no END");
    let two = format!("{CARD}{CARD}");
    assert!(patch(&two, &contact(), &fields(&[ContactField::Notes])).is_none(), "two cards");
    assert!(patch("not a card at all", &contact(), &fields(&[ContactField::Notes])).is_none());
    // Folded lines stay with the line before them, and are kept as they came.
    let long = CARD.replace("X-APPLE-SUBSCRIBED:YES", "X-LONG:one\r\n two");
    let doc = patch(&long, &contact(), &fields(&[ContactField::Notes])).expect("patched");
    assert!(doc.contains("X-LONG:one\r\n two"), "folding kept:\n{doc}");
}
