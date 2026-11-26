// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Calendars and address books kept in step on the phone, over CalDAV and CardDAV: the host's side
//! of the engine's `sync.davRequest` and `sync.applyDav`, one job at a time on a thread of its own.
//!
//! The engine says what to read and takes in what came back; `mailcraft_net::dav` does the reading.
//! The passwords are held per origin (a password for one server is never sent to another): some
//! were typed in Add Account this session, and the rest are the platform's credential store's.
//!
//! A server that asks for a password nothing holds, or refuses the one it was given, puts its
//! account in Send/Receive's "sign in again" list; it is left alone for a while, so a lockout is
//! not made worse. A read that stopped at its limits goes on at the next run. Nothing of an
//! address, a user name or a password is logged.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, TryRecvError};

use mailcraft_engine::Session;
use crate::dav_contacts::ContactRead;
use mailcraft_model::{Account, DavAsk, DavDownload, DavProblem, Id};

use crate::sync::{Inner, SyncHost};

/// How long between two reads when all is well, and the longest wait after trouble (milliseconds).
const EVERY: f64 = 60_000.0;
const LONGEST: f64 = 900_000.0;
/// How long a server that refused what it was given is left alone (milliseconds).
const REFUSED_FOR: f64 = 4.0 * 3_600_000.0;

/// What one read brought: the calendars' answer, the address books', or both.
#[derive(Default)]
pub(crate) struct DavAnswer {
    pub calendar: Option<DavDownload>,
    pub contacts: Option<ContactRead>,
}

/// The account and passwords an address book read is made with.
#[derive(Clone)]
pub(crate) struct ContactsJob {
    pub account: Account,
    pub passwords: HashMap<String, String>,
}

/// How a DAV read is made: over the network, or as a test says.
pub(crate) type DavRunner = Arc<dyn Fn(DavAsk, ContactsJob) -> DavAnswer + Send + Sync>;

/// The reader a core starts with: a test's build reaches nobody and brings nothing.
pub(crate) fn default_runner() -> DavRunner {
    #[cfg(test)]
    return Arc::new(|ask: DavAsk, job: ContactsJob| {
        let _ = job.account;
        DavAnswer { calendar: (!ask.homes.is_empty()).then_some(DavDownload { account: ask.account, ..Default::default() }), contacts: None }
    });
    #[cfg(not(test))]
    Arc::new(|ask: DavAsk, job: ContactsJob| {
        // (A certificate the owner chose to trust is in the settings; a refusal comes back in the
        // download's problem and is said there.)
        let calendar = (!ask.homes.is_empty()).then(|| mailcraft_net::dav::look(&ask, &|origin| job.passwords.get(origin).cloned()).0);
        let contacts = job
            .account
            .dav
            .as_ref()
            .is_some_and(|server| !server.contact_homes.is_empty())
            .then(|| crate::dav_contacts::read(&job.account, &job.passwords));
        DavAnswer { calendar, contacts }
    })
}

/// What starting a read came to.
pub(crate) enum DavStart {
    None,
    Started,
    /// The account's settings name an origin nothing holds a password for: the owner is asked.
    NeedsSignIn(Id),
}

/// What to read for an account that keeps address books and no calendars: the engine's calendar
/// request is for calendars alone, so the account is named here and the books are read by
/// [`crate::dav_contacts`].
fn contact_only_ask(session: &Session, skip: &[Id]) -> Option<DavAsk> {
    let account = session
        .mailbox
        .accounts
        .iter()
        .filter(|a| !skip.contains(&a.id))
        .filter(|a| a.dav.as_ref().is_some_and(|dav| dav.calendar_homes.is_empty() && !dav.contact_homes.is_empty()))
        .min_by_key(|a| (session.dav_asked.get(&a.id).copied().unwrap_or(i64::MIN), a.id))?;
    let server = account.dav.as_ref()?;
    Some(DavAsk {
        account: account.id,
        email: account.email.trim().to_string(),
        username: if server.username.trim().is_empty() { account.email.trim().to_string() } else { server.username.trim().to_string() },
        trusted: server.trusted.clone(),
        homes: Vec::new(),
        list_homes: false,
        server: server.identity(&account.email),
        collections: Vec::new(),
        no_room: false,
        generation: session.dav_generation.get(&account.id).copied().unwrap_or(0),
    })
}

/// The read in flight, and when the next may start.
#[derive(Default)]
pub(crate) struct DavJobs {
    /// The account and the answer to come.
    job: Option<(Id, Receiver<DavAnswer>, Arc<AtomicBool>)>,
    /// When the next read may start (milliseconds since the core began).
    next: f64,
    /// For each account: reads in a row that came to trouble, and when it may be read again.
    trouble: HashMap<Id, (u32, f64)>,
    /// When each account's server last refused what it was given.
    refused: HashMap<Id, f64>,
    /// Whether the next turn is an account of address books alone's (the engine's calendar
    /// request never names one).
    address_books_next: bool,
}

impl DavJobs {
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Let the next read start at the next wake (the owner asked for one).
    pub(crate) fn soon(&mut self) {
        self.next = 0.0;
    }

    /// Take in the read that came, and say what it brought: its account, why it ended and how long
    /// the server asked to be left alone for. `None` when nothing has come.
    pub(crate) fn take(&mut self, session: &mut Session) -> Option<(Id, DavProblem, u64)> {
        let (account, reply) = match self.job.as_ref() {
            Some((account, reply, _)) => (*account, reply),
            None => return None,
        };
        let answer = match reply.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                return Some((account, DavProblem::Unreachable, 0));
            }
        };
        self.job = None;
        let mut problem = DavProblem::None;
        let mut wait = 0;
        if let Some(download) = answer.calendar {
            match session.run("sync.applyDav", &serde_json::json!({"result": download})) {
                Ok(applied) => {
                    problem = applied
                        .get("problem")
                        .and_then(|problem| serde_json::from_value::<DavProblem>(problem.clone()).ok())
                        .unwrap_or(DavProblem::None);
                    wait = applied.get("wait").and_then(serde_json::Value::as_u64).unwrap_or(0);
                }
                Err(_) => {
                    // (An answer that isn't one: trouble like any other, so that the account is left
                    // alone for a while. Nothing of it is held.)
                    log::warn!("a calendar server's read couldn't be taken in");
                    problem = DavProblem::Unreachable;
                }
            }
        }
        if let Some(contacts) = answer.contacts {
            session.dav_asked.insert(account, session.now());
            if let Err(e) = mailcraft_engine::cmd::remote::apply_book_contacts(session, &contacts.download) {
                log::warn!("an address book read couldn't be taken in: {e}");
                if problem == DavProblem::None {
                    problem = DavProblem::ServerErrors;
                }
            } else if problem == DavProblem::None {
                problem = contacts.problem;
            }
        }
        Some((account, problem, wait))
    }

    /// Start the read the engine asks for, if one is due.
    pub(crate) fn start(&mut self, host: &SyncHost, inner: &Inner, session: &mut Session, now: f64) -> DavStart {
        if now < self.next || self.job.is_some() {
            return DavStart::None;
        }
        self.next = now + EVERY;
        // Accounts in their back-off, or whose servers refused what they were given, are left out.
        let skip: Vec<Id> = session
            .mailbox
            .accounts
            .iter()
            .filter(|a| a.dav.is_some())
            .filter(|a| {
                self.trouble.get(&a.id).is_some_and(|(_, next)| now < *next) || self.refused.get(&a.id).is_some_and(|at| now < at + REFUSED_FOR)
            })
            .map(|a| a.id)
            .collect();
        let asked = session.run("sync.davRequest", &serde_json::json!({"skip": skip}));
        let calendars = asked.ok().and_then(|asked| asked.get("ask").and_then(|ask| serde_json::from_value::<DavAsk>(ask.clone()).ok()));
        let books = contact_only_ask(session, &skip);
        // (An account of address books alone takes every other turn, so that it is read too.)
        let ask = if self.address_books_next && books.is_some() { books } else { calendars.or(books) };
        self.address_books_next = !self.address_books_next;
        let Some(ask) = ask else { return DavStart::None };
        let account = ask.account;
        let Some(server) = session.mailbox.account(account).and_then(|a| a.dav.as_ref()) else { return DavStart::None };
        let mut passwords: HashMap<String, String> = HashMap::new();
        for origin in server.home_origins() {
            match host.dav_password(inner, session, account, &origin) {
                Some(password) => {
                    passwords.insert(origin, password);
                }
                None => {
                    // (Without a password the server would refuse the read for nothing: the owner
                    // is asked, and the account waits.)
                    self.trouble.insert(account, (1, now + EVERY));
                    return DavStart::NeedsSignIn(account);
                }
            }
        }
        let runner = host.dav_runner();
        let account_for_job = session.mailbox.account(account).cloned();
        let Some(account_for_job) = account_for_job else { return DavStart::None };
        let job = ContactsJob { account: account_for_job, passwords };
        let (sender, reply) = std::sync::mpsc::channel::<DavAnswer>();
        let cancel = Arc::new(AtomicBool::new(false));
        let spawned = std::thread::Builder::new().name("mailcraft-dav".to_string()).spawn(move || {
            // (A panic in the job is an answer that brought nothing, never a wait that never ends.)
            let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner(ask, job))).unwrap_or_else(|_| DavAnswer {
                calendar: Some(DavDownload { account, problem: DavProblem::Unreachable, ..Default::default() }),
                contacts: None,
            });
            let _ = sender.send(answer);
        });
        if spawned.is_err() {
            log::warn!("a thread for a calendar server read could not be started");
            return DavStart::None;
        }
        self.job = Some((account, reply, cancel));
        DavStart::Started
    }

    /// What an answer means for the next read: a refused password is left alone for a while, and
    /// trouble backs off up to [`LONGEST`].
    pub(crate) fn noted(&mut self, account: Id, problem: DavProblem, wait: u64, now: f64) {
        match problem {
            DavProblem::None | DavProblem::Limits => {
                self.trouble.remove(&account);
                self.refused.remove(&account);
            }
            DavProblem::WrongPassword | DavProblem::NeedsPassword | DavProblem::UnsupportedSignIn => {
                self.refused.insert(account, now);
                self.trouble.insert(account, (1, now + REFUSED_FOR));
            }
            DavProblem::Full | DavProblem::ServerErrors => {
                self.trouble.remove(&account);
            }
            _ => {
                let failing = self.trouble.get(&account).map_or(0, |(n, _)| *n).saturating_add(1);
                let backoff = (EVERY * 2f64.powi(i32::try_from(failing).unwrap_or(10).min(10))).min(LONGEST);
                let wait = (wait as f64 * 1000.0).max(backoff);
                self.trouble.insert(account, (failing, now + wait));
            }
        }
    }
}
