// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An Exchange server account's mail on the phone: a message whose body is on the server is
//! fetched whole with the account's password when the owner opens it. The runner is a stand-in
//! that asks no server; everything it answers is invented.

use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_engine::cmd::ews_mail::apply_ews_mail;
use mailcraft_model::sync::{
    EwsFolderDeltas, EwsMailResult, EwsMessagePage, GraphFetchResult, GraphFetched, GraphFolder, GraphMessage, ServerMessage,
};
use mailcraft_model::{AccountKind, Address, FolderKind};
use serde_json::json;

use crate::fake_tests::{FakePlatform, block_on, now};
use crate::fetch::{Answer, FetchDone, FetchRunner, Work};
use crate::fetch_tests::{id_of, is_whole, mime, until};
use crate::{Core, Event, OpenState};

const EMAIL: &str = "owner@example.org";
const URL: &str = "https://exchange.example.org/EWS/Exchange.asmx";

fn piece(account: u64) -> EwsMailResult {
    EwsMailResult { account, email: EMAIL.into(), url: URL.into(), partial: true, ..Default::default() }
}

/// A session with one Exchange account whose Inbox has one message as headers and a preview: its
/// body is the server's still.
fn session() -> Session {
    let mut s = Session::empty();
    let account =
        s.run("account.add", &json!({"email": EMAIL, "kind": "exchange", "auth": "password", "ews": {"url": URL}})).unwrap()["id"].as_u64().unwrap();
    let folders = EwsFolderDeltas {
        changed: vec![GraphFolder { id: "F1".into(), name: "Inbox".into(), kind: FolderKind::Inbox, ..Default::default() }],
        afresh: true,
        ..Default::default()
    };
    apply_ews_mail(&mut s, EwsMailResult { folders: Some(folders), hierarchy: "H1".into(), ..piece(account) }).unwrap();
    let messages = vec![GraphMessage {
        server: ServerMessage { id: "A1".into(), size: 900, change_key: "CK1".into(), ..Default::default() },
        message_id: "a1@example.org".into(),
        subject: "Kickoff".into(),
        from: Address::new("Zoe Park", "zoe@example.org"),
        to: vec![Address::new("Ada", EMAIL)],
        sent: now() - 120,
        received: now() - 60,
        preview: "About Kickoff".into(),
        ..Default::default()
    }];
    let page = EwsMessagePage { folder: "F1".into(), messages, afresh: true, last: true, state: "S1".into(), ..Default::default() };
    apply_ews_mail(&mut s, EwsMailResult { pages: vec![page], ..piece(account) }).unwrap();
    apply_ews_mail(&mut s, EwsMailResult { whole: true, partial: false, ..piece(account) }).unwrap();
    s
}

#[test]
fn opening_an_exchange_message_fetches_it_whole_with_the_accounts_password() {
    let core = Core::start(FakePlatform::new(), session).unwrap();
    let account = block_on(core.call(|s| Ok(s.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Exchange).map(|a| a.id)))).unwrap().unwrap();
    // Typed into Add Account this session.
    core.sync.keep_password(account, "hunter2".into());

    let asked: Arc<Mutex<Vec<(Vec<String>, String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let run: FetchRunner = Arc::new(move |access, work, _cancel| {
        let Work::EwsMessages { request, password } = work else {
            return FetchDone { answer: None, renewal: Default::default() };
        };
        assert!(access.is_none(), "an Exchange run signs in with its password, not a Graph token");
        let ids = request.ids.clone();
        let background = request.background;
        log.lock().unwrap().push((ids.clone(), password, background));
        let fetched = ids.iter().map(|id| GraphFetched { id: id.clone(), mime: mime("Kickoff", "The whole of Kickoff.") }).collect();
        let result = GraphFetchResult { account: request.account, email: request.email.clone(), fetched, ..Default::default() };
        FetchDone { answer: Some(Answer::Messages(result)), renewal: Default::default() }
    });
    core.sync.set_fetch_runner(run);

    let message = id_of(&core, "A1");
    assert!(!is_whole(&core, message), "only the preview is on the phone");
    assert_eq!(block_on(core.open_message(message)).unwrap(), OpenState::Fetching);
    let events = until(&core, |e| matches!(e, Event::MessageFetched { message: m } if *m == message));
    assert!(is_whole(&core, message), "the whole message came");
    assert!(!events.iter().any(|e| matches!(e, Event::FetchFailed { .. })), "{events:?}");

    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1, "one message, asked once");
    assert_eq!(asked[0].0, ["A1"]);
    assert_eq!(asked[0].1, "hunter2", "the password the owner typed");
    assert!(!asked[0].2, "the owner waits on this one");
    let said = format!("{events:?}");
    assert!(!said.contains("hunter2"), "no password in an event: {said}");
    // Open again: it is here, and nothing is asked.
    assert_eq!(block_on(core.open_message(message)).unwrap(), OpenState::Here);
    assert_eq!(asked.len(), 1);
}

#[test]
fn an_exchange_fetch_reads_the_password_kept_in_the_credential_store() {
    let platform = FakePlatform::new();
    let core = Core::start(platform.clone(), session).unwrap();
    let account = block_on(core.call(|s| Ok(s.mailbox.accounts.iter().find(|a| a.kind == AccountKind::Exchange).map(|a| a.id)))).unwrap().unwrap();
    // Kept when the account was added, and the app has restarted since: it is the store's now.
    let key = {
        let a = block_on(core.call(move |s| Ok(s.mailbox.account(account).cloned()))).unwrap().unwrap();
        mailcraft_host::secrets::secret_key("password", &a)
    };
    crate::platform::SecretStore::set(&*platform.secrets, key, "from-store".into()).unwrap();

    let used: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = used.clone();
    let run: FetchRunner = Arc::new(move |_access, work, _cancel| {
        let Work::EwsMessages { request, password } = work else {
            return FetchDone { answer: None, renewal: Default::default() };
        };
        log.lock().unwrap().push(password);
        let fetched = request.ids.iter().map(|id| GraphFetched { id: id.clone(), mime: mime("Kickoff", "The whole of Kickoff.") }).collect();
        let result = GraphFetchResult { account: request.account, email: request.email.clone(), fetched, ..Default::default() };
        FetchDone { answer: Some(Answer::Messages(result)), renewal: Default::default() }
    });
    core.sync.set_fetch_runner(run);

    let message = id_of(&core, "A1");
    assert_eq!(block_on(core.open_message(message)).unwrap(), OpenState::Fetching);
    until(&core, |e| matches!(e, Event::MessageFetched { message: m } if *m == message));
    assert!(is_whole(&core, message));
    assert_eq!(*used.lock().unwrap(), ["from-store"]);
}
