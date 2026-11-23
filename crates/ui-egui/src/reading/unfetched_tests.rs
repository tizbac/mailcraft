// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the reading pane says of a message of which only the headers and a preview are here: the
//! plain line, the one for a message bigger than the account downloads in full, and the sizes it
//! names.

use mailcraft_engine::Session;
use mailcraft_model::sync::ServerMessage;
use mailcraft_model::{AccountKind, DownloadInFull, Folder, FolderKind, Mailbox, Message};

use crate::MailApp;

const NOW: i64 = 1_791_993_600;
const NOT_YET: &str = "Only the beginning of this message is here. The rest downloads with the next Send/Receive.";

/// A mailbox with one Microsoft account, one Inbox and one message of `size` on its server.
fn one_message(size: u64, limit: DownloadInFull) -> MailApp {
    let mut mb = Mailbox::with_local_account("ada@example.org", "Ada");
    let account = mb.accounts[0].id;
    mb.accounts[0].kind = AccountKind::Microsoft;
    mb.accounts[0].download_in_full = limit;
    let folder = mb.alloc_id();
    mb.folders.push(Folder { id: folder, account, name: "Inbox".into(), kind: FolderKind::Inbox, ..Default::default() });
    mb.add_message(Message {
        folder,
        received: 60,
        unfetched: true,
        subject: "Big log".into(),
        server: Some(ServerMessage { id: "AAkA1".into(), folder: "F".into(), size, ..Default::default() }),
        ..Default::default()
    })
    .unwrap();
    MailApp::new(Session::at(mb, NOW, 0), crate::Services::default())
}

fn said(app: &MailApp) -> String {
    let m = app.session.mailbox.messages.values().next().unwrap().clone();
    crate::reading::unfetched_said(app, &m)
}

#[test]
fn a_message_bigger_than_the_account_downloads_names_both_sizes() {
    let app = one_message(60 << 20, DownloadInFull::UpTo10Mb);
    let said = said(&app);
    assert!(said.starts_with("This message is 60 MB on the server, more than this account downloads in full (10 MB)"), "{said}");
    assert!(said.contains("Account Settings › Download messages in full"), "{said}");
}

#[test]
fn a_message_within_the_limit_is_simply_not_here_yet() {
    for (size, limit) in [(5 << 20, DownloadInFull::UpTo10Mb), (60 << 20, DownloadInFull::Everything), (0, DownloadInFull::UpTo10Mb)] {
        let app = one_message(size, limit);
        assert_eq!(said(&app), NOT_YET, "{size} bytes with a {limit:?} limit");
    }
}

#[test]
fn a_message_past_what_any_account_downloads_says_so() {
    // An account set to "Everything" still has an end: 200 MB. Past it the line says what the
    // ceiling is rather than telling the owner to raise a setting that is already at its top.
    let app = one_message(300 << 20, DownloadInFull::Everything);
    let said = said(&app);
    assert!(said.starts_with("This message is 300 MB on the server, more than MailCraft downloads whole (200 MB)"), "{said}");
    assert!(said.contains("Open it to read it on the server"), "{said}");
}

#[test]
fn the_offline_line_comes_before_the_size_line() {
    let mut app = one_message(60 << 20, DownloadInFull::UpTo10Mb);
    app.session.driver.fetch.offline = true;
    assert_eq!(said(&app), "This message isn't on this computer yet. It will download when you're online.");
    app.session.driver.fetch.offline = false;
    assert!(said(&app).starts_with("This message is 60 MB on the server"), "the size is said when the server is there");
}
