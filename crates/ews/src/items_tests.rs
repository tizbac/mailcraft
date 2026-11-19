// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The reader of `SyncFolderItems`' answer, over answers made by hand: which changes make a page
//! and which make it no page (#252, E6). A page is whole or it is an error; what an item lacks is
//! read as nothing said.

use crate::standin::response;
use crate::standin_mail::ok_message;
use crate::*;

fn page(changes: &str, max: u32) -> Result<ItemsPage, Error> {
    let inner =
        format!("<m:SyncState>S-NEW</m:SyncState><m:IncludesLastItemInRange>true</m:IncludesLastItemInRange><m:Changes>{changes}</m:Changes>");
    let body = response("SyncFolderItems", &ok_message("SyncFolderItems", &inner));
    let reply = Reply::parse(body.as_bytes()).unwrap();
    SyncFolderItems::new("FOLDER", "S-OLD", max, Shape::Full).answer(&reply)
}

fn bare(id: &str) -> String {
    format!("<t:Message><t:ItemId Id=\"{id}\" ChangeKey=\"CK\"/></t:Message>")
}

#[test]
fn more_changes_than_were_asked_for_are_taken_in_when_every_one_is_readable() {
    let changes: String = (0..50).map(|n| format!("<t:Create>{}</t:Create>", bare(&format!("ID{n}")))).collect();
    let got = page(&changes, 5).unwrap();
    assert_eq!((got.messages.len(), got.state.as_str(), got.last), (50, "S-NEW", true));
}

#[test]
fn an_item_that_lacks_what_is_not_essential_is_read_with_nothing_said() {
    // Only an id: no read state, importance, categories, preview, sender, times or size.
    let got = page(&format!("<t:Create>{}</t:Create><t:Update>{}</t:Update>", bare("ID1"), bare("ID2")), 10).unwrap();
    assert_eq!(got.messages.len(), 2);
    let one = &got.messages[0];
    assert_eq!((one.server.id.as_str(), one.server.read, one.server.categories.len(), one.subject.as_str()), ("ID1", false, 0, ""));
    assert!(one.preview.is_empty() && one.to.is_empty() && one.received == 0);
    // An id with any character but a control one is an id (the server's, and opaque).
    let odd = page(&format!("<t:Create>{}</t:Create>", bare("a b&lt;c&gt;/+=-_")), 10).unwrap();
    assert_eq!(odd.messages[0].server.id, "a b<c>/+=-_");
    // A read-state change that doesn't say which state carries nothing to lose: passed over.
    let none = page("<t:ReadFlagChange><t:ItemId Id=\"ID1\" ChangeKey=\"CK\"/></t:ReadFlagChange>", 10).unwrap();
    assert!(none.read.is_empty() && none.messages.is_empty() && none.read_unsaid == 1);
    // A kind of change this program doesn't know is passed over as well.
    let unknown = page("<t:Frobnicate><t:ItemId Id=\"ID1\"/></t:Frobnicate>", 10).unwrap();
    assert!(unknown.messages.is_empty() && unknown.removed.is_empty() && unknown.read.is_empty());
}

#[test]
fn a_change_with_no_usable_id_or_item_makes_the_page_no_page() {
    let long = "x".repeat(2_000);
    let bad: Vec<(&str, String)> = vec![
        ("a delete with no id", "<t:Delete><t:ItemId ChangeKey=\"CK\"/></t:Delete>".into()),
        ("a delete with an empty id", "<t:Delete><t:ItemId Id=\"\"/></t:Delete>".into()),
        ("a delete with a long id", format!("<t:Delete><t:ItemId Id=\"{long}\"/></t:Delete>")),
        ("a delete with a control character in the id", "<t:Delete><t:ItemId Id=\"A&#9;B\"/></t:Delete>".into()),
        ("a delete with no item", "<t:Delete/>".into()),
        ("a read-state change with a long id", format!("<t:ReadFlagChange><t:ItemId Id=\"{long}\"/><t:IsRead>true</t:IsRead></t:ReadFlagChange>")),
        ("a read-state change with no id", "<t:ReadFlagChange><t:IsRead>true</t:IsRead></t:ReadFlagChange>".into()),
        ("a create with nothing in it", "<t:Create/>".into()),
        ("a create with an item that has no id", "<t:Create><t:Message><t:Subject>Hello</t:Subject></t:Message></t:Create>".into()),
        ("an update with a long id", format!("<t:Update>{}</t:Update>", bare(&long))),
        ("an update with an empty id", format!("<t:Update>{}</t:Update>", bare(""))),
    ];
    for (what, change) in bad {
        // Alone, and after changes that are fine: either way nothing of the page is handed on.
        assert!(matches!(page(&change, 10).unwrap_err(), Error::Unreadable(_)), "{what}");
        let after = format!("<t:Create>{}</t:Create>{change}", bare("ID1"));
        assert!(matches!(page(&after, 10).unwrap_err(), Error::Unreadable(_)), "{what}, after a good one");
    }
    // A delete with a good id is a removal.
    let good = page("<t:Delete><t:ItemId Id=\"ID9\" ChangeKey=\"CK\"/></t:Delete>", 10).unwrap();
    assert_eq!(good.removed, ["ID9"]);
    // What couldn't be taken in is named by the kind of change, and by its item's subject when the
    // answer had one (for the owner's window only: the log takes neither).
    match page("<t:Create><t:Message><t:Subject>Hello</t:Subject></t:Message></t:Create>", 10).unwrap_err() {
        Error::Unreadable(why) => assert_eq!((why.kind.as_str(), why.subject.as_str()), ("Create", "Hello")),
        other => panic!("{other:?}"),
    }
    match page("<t:Delete/>", 10).unwrap_err() {
        Error::Unreadable(why) => assert_eq!((why.kind.as_str(), why.subject.as_str()), ("Delete", "")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_page_larger_than_the_model_takes_in_is_no_page() {
    let changes: String =
        (0..mailcraft_model::sync::MAX_EWS_PAGE_MESSAGES + 1).map(|n| format!("<t:Create>{}</t:Create>", bare(&format!("ID{n}")))).collect();
    assert!(matches!(page(&changes, 512).unwrap_err(), Error::Unreadable(_)));
}

#[test]
fn a_page_of_read_state_changes_by_the_thousand_is_still_a_page() {
    // (Grommunio sends the read states it already sent along with each new page: a folder whose mail
    // was read in bulk answers with thousands of `ReadFlagChange`s and no items at all. They are an
    // id and a flag, and the state says all of them have been given, so the page is taken in whole.)
    let changes: String =
        (0..12_000).map(|n| format!("<t:ReadFlagChange><t:ItemId Id=\"ID{n}\"/><t:IsRead>true</t:IsRead></t:ReadFlagChange>")).collect();
    let got = page(&changes, 512).unwrap_or_else(|why| panic!("12,000 read states are a page: {why:?}"));
    assert_eq!((got.read.len(), got.read_left_out, got.messages.len(), got.last), (12_000, 0, 0, true));
}

#[test]
fn a_page_of_read_states_past_the_cap_keeps_the_newest_and_goes_on() {
    // (The same server sends the read states it already sent at the front of every page, so a page
    // can carry far more of them than a page takes in. Refusing it would leave the folder stuck at
    // that page for good; instead the newest are kept and the repeats at the front are left out,
    // and the state the server gave is taken in all the same.)
    let past = mailcraft_model::sync::MAX_EWS_PAGE_READ;
    let changes: String = (0..past + 10_000)
        .map(|n| {
            format!("<t:ReadFlagChange><t:ItemId Id=\"R{n}\"/><t:IsRead>{}</t:IsRead></t:ReadFlagChange>", if n % 2 == 0 { "true" } else { "false" })
        })
        .collect();
    let got = page(&changes, 512).unwrap_or_else(|why| panic!("a page of {past} plus 10,000 read states is still a page: {why:?}"));
    assert_eq!((got.read.len(), got.read_left_out, got.last), (past, 10_000, true));
    assert_eq!(got.state, "S-NEW");
    // (The tail, the states the folder hasn't had yet, is what is kept.)
    assert_eq!(
        (got.read.first().map(|r| r.id.clone()), got.read.last().map(|r| r.id.clone())),
        (Some(format!("R{}", 10_000)), Some(format!("R{}", past + 9_999)))
    );
    assert!(got.read.iter().all(|r| r.id.starts_with('R')));
    // (Items beside them are still items: they count against their own cap, not the read states'.)
    let mixed: String = (0..1_000)
        .map(|n| format!("<t:Create>{}</t:Create>", bare(&format!("ID{n}"))))
        .chain((0..past + 5_000).map(|n| format!("<t:ReadFlagChange><t:ItemId Id=\"R{n}\"/><t:IsRead>true</t:IsRead></t:ReadFlagChange>")))
        .collect();
    let got = page(&mixed, 512).unwrap_or_else(|why| panic!("items with a long list of read states is a page: {why:?}"));
    assert_eq!((got.messages.len(), got.read.len(), got.read_left_out), (1_000, past, 5_000));
}

#[test]
fn a_page_of_more_items_than_are_taken_in_is_no_page() {
    let changes: String =
        (0..mailcraft_model::sync::MAX_EWS_PAGE_MESSAGES + 1).map(|n| format!("<t:Create>{}</t:Create>", bare(&format!("ID{n}")))).collect();
    let why = page(&changes, 200).err();
    assert!(why.is_some(), "more items than a page takes in is refused");
    // (Read states beside them don't count against the items' cap.)
    let mixed: String = (0..mailcraft_model::sync::MAX_EWS_PAGE_MESSAGES)
        .map(|n| format!("<t:Create>{}</t:Create>", bare(&format!("ID{n}"))))
        .chain((0..5_000).map(|n| format!("<t:ReadFlagChange><t:ItemId Id=\"R{n}\"/><t:IsRead>false</t:IsRead></t:ReadFlagChange>")))
        .collect();
    let got = page(&mixed, 200).unwrap_or_else(|why| panic!("items with read states beside them is a page: {why:?}"));
    assert_eq!((got.messages.len(), got.read.len()), (mailcraft_model::sync::MAX_EWS_PAGE_MESSAGES, 5_000));
}
