// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A CalDAV and CardDAV account added on the phone: calendars and address books alone, no mail
//! folders, its password per origin in the platform's store, and what Send/Receive asks of it.

use mailcraft_engine::Session;
use mailcraft_host::sync::SendReceive;

use crate::accounts::DavAccountSettings;
use crate::fake_tests::{FakePlatform, block_on};
use crate::{Core, CoreError};

const PASSWORD: &str = "caldav-secret";

fn settings() -> DavAccountSettings {
    DavAccountSettings {
        email: "ada@example.org".into(),
        display_name: "Ada's calendars".into(),
        username: String::new(),
        password: PASSWORD.into(),
        calendar_homes: vec!["https://dav.example.org/cal/ada/".into()],
        contact_homes: vec!["https://dav.example.org/cards/ada/".into()],
    }
}

#[test]
fn a_dav_account_is_added_with_no_mail_and_its_password_kept_per_origin() {
    let platform = FakePlatform::new();
    let core = Core::start_with(platform.clone(), Session::empty, SendReceive::new()).expect("a core");
    let id = block_on(core.add_dav_account(settings())).expect("the account is added");
    let account = block_on(core.call(move |s| Ok(s.mailbox.account(id).cloned()))).unwrap().expect("the account");
    assert_eq!(account.kind, mailcraft_model::AccountKind::Dav);
    assert!(account.incoming.is_none() && account.outgoing.is_none(), "no mail servers");
    assert_eq!(account.dav.as_ref().map(|dav| dav.calendar_homes.len()), Some(1));
    let folders = block_on(core.call(move |s| Ok(s.mailbox.folders.iter().filter(|f| f.account == id).count()))).unwrap();
    assert_eq!(folders, 0, "it has no mail folders");
    let listed = block_on(core.folders(0u64)).unwrap();
    assert_eq!(
        listed.accounts.iter().find(|a| a.id == id).map(|a| a.kind.as_str()),
        Some("dav"),
        "the app reads it as a calendars and contacts account"
    );
    // The settings say where it is, for the calendar and contacts jobs.
    let said = block_on(core.execute("account.list".into(), "{}".into())).unwrap();
    let listed: serde_json::Value = serde_json::from_str(&said).unwrap();
    let entry = listed["accounts"].as_array().and_then(|all| all.iter().find(|a| a["id"].as_u64() == Some(id))).expect("the account is listed");
    assert_eq!(entry["kind"], "dav");
    assert_eq!(entry["dav"]["calendarHomes"][0], "https://dav.example.org/cal/ada/");
    // The password is kept under a name bound to the origin it goes to, never the account's other.
    let keys = block_on(core.call(move |s| {
        let account = s.mailbox.account(id).expect("the account");
        Ok(mailcraft_host::secrets::dav_secret_keys(account))
    }))
    .unwrap();
    assert_eq!(keys.len(), 1, "one origin names both homes");
    {
        let kept = platform.secrets.kept.lock().unwrap();
        assert!(kept.iter().any(|(k, v)| keys.contains(k) && v == PASSWORD), "the password is not kept: {kept:?}");
    }
    // What a host is asked to read names the homes and the account.
    let asked = block_on(core.execute("sync.davRequest".into(), "{}".into())).unwrap();
    let asked: serde_json::Value = serde_json::from_str(&asked).unwrap();
    assert_eq!(asked["ask"]["account"], id);
    assert_eq!(asked["ask"]["homes"][0], "https://dav.example.org/cal/ada/");
    // It goes with the account, and its stored password too.
    block_on(core.remove_account(id)).unwrap();
    let kept = platform.secrets.kept.lock().unwrap();
    assert!(!kept.iter().any(|(k, _)| keys.contains(k)), "the password is still kept");
    core.shutdown();
}

#[test]
fn a_dav_account_is_changed_and_a_bad_home_is_refused() {
    let platform = FakePlatform::new();
    let core = Core::start_with(platform.clone(), Session::empty, SendReceive::new()).expect("a core");
    let id = block_on(core.add_dav_account(settings())).expect("the account is added");
    let mut changed = settings();
    changed.display_name = "Home".into();
    changed.calendar_homes = vec!["https://cloud.example.org/dav/".into()];
    changed.contact_homes = Vec::new();
    block_on(core.edit_dav_account(id, changed)).expect("the account is changed");
    let account = block_on(core.call(move |s| Ok(s.mailbox.account(id).cloned()))).unwrap().expect("the account");
    assert_eq!(account.display_name, "Home");
    assert_eq!(account.dav.as_ref().map(|dav| dav.calendar_homes.clone()), Some(vec!["https://cloud.example.org/dav/".to_string()]));
    assert!(account.dav.as_ref().is_some_and(|dav| dav.contact_homes.is_empty()));

    let mut plain = settings();
    plain.calendar_homes = vec!["http://dav.example.org/cal/".into()];
    plain.contact_homes = Vec::new();
    let said = block_on(core.add_dav_account(plain)).unwrap_err();
    assert!(matches!(said, CoreError::BadRequest { .. }), "{said:?}");
    core.shutdown();
}

#[test]
fn a_send_and_receive_with_only_calendars_reads_them_and_says_when_it_is_done() {
    use std::sync::Arc;

    use mailcraft_model::{DavDownload, DavProblem};

    let platform = FakePlatform::new();
    let core = Core::start_with(platform.clone(), Session::empty, SendReceive::new()).expect("a core");
    let id = block_on(core.add_dav_account(settings())).expect("the account is added");
    let asked = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
    let seen = asked.clone();
    core.sync.set_dav_runner(Arc::new(move |ask, job| {
        seen.lock().unwrap().push(ask.account);
        assert_eq!(job.passwords.len(), 1, "the one origin's password");
        crate::dav::DavAnswer {
            calendar: Some(DavDownload {
                account: ask.account,
                email: ask.email,
                server: ask.server,
                generation: ask.generation,
                problem: DavProblem::None,
                ..Default::default()
            }),
            contacts: None,
        }
    }));
    let summary = block_on(core.background_sync(5_000)).expect("a round");
    assert!(summary.ran && summary.finished, "{summary:?}");
    assert!(asked.lock().unwrap().contains(&id), "the calendar server was asked: {asked:?}");
    core.shutdown();
}
