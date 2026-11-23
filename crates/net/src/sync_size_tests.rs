// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! #300: an IMAP message too large to download keeps its size on the server in a field of its
//! own, which no sender can write.

use super::*;

const HEADER: &[u8] = b"From: Bob <bob@example.org>\r\nTo: ada@example.org\r\nSubject: Big file\r\nMessage-ID: <big@example.org>\r\n\
    X-MailCraft-Not-Downloaded: 1\r\n\r\n";

#[test]
fn a_message_too_large_has_its_size_on_the_server() {
    let fetched = Fetched { uid: 7, body: Some(HEADER.to_vec()), ..Default::default() };
    let m = too_large(fetched, 62_914_560, MAX_MESSAGE);
    assert_eq!((m.not_downloaded, m.list_size()), (62_914_560, Some(62_914_560)));
    // The sender's header of that name is replaced by the size the server gave.
    let marks: Vec<&str> = m.headers.iter().filter(|(name, _)| name.eq_ignore_ascii_case(NOT_DOWNLOADED)).map(|(_, v)| v.as_str()).collect();
    assert_eq!(marks, ["62914560"]);
    // Whatever size a server claims is kept as it said.
    assert_eq!(too_large(Fetched { uid: 8, ..Default::default() }, u64::MAX, 0).list_size(), Some(u64::MAX));
}

#[test]
fn a_message_downloaded_whole_has_no_size_from_the_server_whatever_its_header_says() {
    let whole = [HEADER, b"The text.\r\n".as_slice()].concat();
    let m = message_from(Fetched { uid: 7, body: Some(whole), ..Default::default() });
    assert_eq!(m.not_downloaded, 0);
    assert!(m.headers.iter().any(|(name, _)| name.eq_ignore_ascii_case(NOT_DOWNLOADED)));
    assert_eq!(m.list_size(), None, "a sender's header is not believed");
}

#[test]
fn the_account_says_how_large_a_message_it_wants_whole() {
    let mut a = mailcraft_model::Account::default();
    // An IMAP run that was never given a setting keeps the size it has always kept.
    assert_eq!(Limits::in_pieces().for_account(&a).largest, MAX_MESSAGE);
    // A setting of its own wins, whatever it is.
    a.download_in_full = mailcraft_model::DownloadInFull::UpTo50Mb;
    assert_eq!(Limits::in_pieces().for_account(&a).largest, 50 << 20);
    // "Everything" is not without an end: it is the ceiling every account has.
    a.download_in_full = mailcraft_model::DownloadInFull::Everything;
    assert_eq!(Limits::in_pieces().for_account(&a).largest, mailcraft_model::MAX_WHOLE_MESSAGE);
    // A message past it keeps its size on the server and a note in place of its text.
    let m = too_large(Fetched { uid: 9, body: Some(HEADER.to_vec()), ..Default::default() }, 300 * 1024 * 1024, mailcraft_model::MAX_WHOLE_MESSAGE);
    assert_eq!(m.not_downloaded, 300 * 1024 * 1024);
    assert!(m.list_size().is_some());
}
