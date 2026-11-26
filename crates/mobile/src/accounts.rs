// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Accounts and sign-in for the phone: Add Account (a Microsoft account), the permissions that
//! are asked for by themselves, the message for an administrator, and Remove Account.
//!
//! The owner signs in on Microsoft's own page in the system browser; the page comes back to the
//! app's custom scheme and the native side gives the address it was opened with to
//! [`Core::hand_back_redirect`], whole. Everything about it is read here, in Rust, by the OAuth
//! strategy of `mailcraft-net` (`oauth/scheme.rs`): the scheme and the path to the letter, the
//! state, the fields, the sizes. The native side passes the address through and does nothing else
//! with it. The tokens never cross the binding: the refresh token goes to the platform's
//! credential store and nowhere else, the access token is dropped, and neither appears in an
//! error or a log line.
//!
//! Permissions (To Do, shared mailboxes, sending from one) are asked for by themselves after the
//! app has asked the owner, never on the page of a sign-in, so that a sign-in never waits on an
//! administrator.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::{Mutex, PoisonError};

use mailcraft_host::signin::{
    self, Checked, Ending, Endpoints, FavoritesRefusal, NetError, Page, Permission as NetPermission, Resource, Standing, Why, admin_consent_url,
    permission_endpoints,
};
use mailcraft_model::sync::Tokens;
use mailcraft_model::{AccountKind, Directory};
use serde_json::{Value, json};

use crate::queue::{Queue, Shed, Weight};
use crate::{Core, CoreError};

/// A permission that is asked for by itself, after the app has asked the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Permission {
    /// Reading and changing the owner's Microsoft To Do lists and tasks.
    Tasks,
    /// Reading and changing the shared mailboxes the owner was given access to.
    SharedMail,
    /// Sending from a shared mailbox, as it or on its behalf.
    SharedSend,
}

impl From<Standing> for PermissionState {
    fn from(s: Standing) -> PermissionState {
        match s {
            Standing::Allowed => PermissionState::Allowed,
            Standing::NotAllowed => PermissionState::NotAllowed,
            Standing::SignInNeeded => PermissionState::SignInNeeded,
            Standing::Unknown => PermissionState::Unknown,
        }
    }
}

impl From<Permission> for NetPermission {
    fn from(p: Permission) -> NetPermission {
        match p {
            Permission::Tasks => NetPermission::Tasks,
            Permission::SharedMail => NetPermission::SharedMail,
            Permission::SharedSend => NetPermission::SharedSend,
        }
    }
}

/// How a sign-in for an account ended.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SignInOutcome {
    /// Signed in: the account is in the mailbox (it was there already when the owner signed in
    /// to an account they had).
    SignedIn { account: u64, email: String, name: String },
    /// An administrator of the organisation has to approve MailCraft first.
    NeedsApproval,
    /// The page came back with a no, or the owner left the page that said an administrator has
    /// to approve first (Microsoft sends the same for both).
    Declined,
    /// Nothing came back in the time that is waited.
    TimedOut,
    /// Called off from here.
    Cancelled,
    /// The organisation's rules, Conditional Access among them, refuse this sign-in: not a fault,
    /// and not something an approval changes.
    OrganisationBlocks,
    /// No browser could be started for the page.
    NoBrowser,
    /// Anything else; `reason` is in the program's words.
    Failed { reason: String },
}

/// How asking for a permission ended.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionOutcome {
    /// Given: the account's sign-in covers it now.
    Granted,
    NeedsApproval,
    OrganisationBlocks,
    /// Microsoft has no such permission for this kind of account.
    NotForThisAccount,
    Declined,
    TimedOut,
    Cancelled,
    NoBrowser,
    Failed {
        reason: String,
    },
}

/// Whether an account's sign-in covers a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionState {
    Allowed,
    /// Not given yet: the app can ask the owner.
    NotAllowed,
    /// The account's sign-in is over altogether: the owner signs in again first.
    SignInNeeded,
    /// Microsoft couldn't be asked (no network, a server's "not now").
    Unknown,
}

/// One server of an account that signs in with a password: where it is, and whether the
/// connection is encrypted from the first byte (`tls`, usually port 993 or 465) or with `STARTTLS`
/// (usually 143 or 587). A server without encryption is never offered by the phone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MailServer {
    pub host: String,
    pub port: u16,
    /// `tls` or `starttls`.
    pub security: String,
}

/// A mail account that signs in with a password: IMAP to read, SMTP to send.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ImapAccountSettings {
    pub email: String,
    /// What the folder pane calls it; empty: the address.
    pub display_name: String,
    /// The name the servers know the owner by; empty: the address.
    pub username: String,
    pub password: String,
    pub incoming: MailServer,
    pub outgoing: MailServer,
}

/// An Exchange server account: Exchange Web Services to read, send and keep its calendars and
/// address books in step, with the password the owner types.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ExchangeAccountSettings {
    pub email: String,
    /// What the folder pane calls it; empty: the address.
    pub display_name: String,
    /// The name the server knows the owner by (`DOMAIN\user` or a UPN); empty: the address.
    pub username: String,
    pub password: String,
    /// The `https://` address of the server's Exchange Web Services
    /// (`…/EWS/Exchange.asmx`).
    pub url: String,
}

/// An account's settings as the phone's Edit form reads them out: its address and names, its
/// mail servers (empty for an account kept in step by a provider's API), and an Exchange server
/// account's Exchange Web Services address. The password is never here.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AccountSettings {
    pub email: String,
    pub display_name: String,
    pub username: String,
    pub incoming: Option<MailServer>,
    pub outgoing: Option<MailServer>,
    /// An Exchange server account's Exchange Web Services address; empty for another.
    pub url: String,
}

/// What the sign-ins of a core hold: the host's (where the redirect lands, and the one that is
/// under way), and for the tests a token server and a page of their own.
#[derive(Default)]
pub(crate) struct SignIns {
    host: signin::SignIns,
    /// A token server and a page of the tests' own, for a sign-in at them (never in a build that
    /// isn't a test's: nothing sets it).
    #[cfg(test)]
    pub(crate) server: Mutex<Option<(String, String)>>,
}

impl SignIns {
    pub(crate) fn begin(&self) -> Option<signin::Place<'_>> {
        self.host.begin()
    }

    /// Call off the sign-in that is open, if one is.
    pub(crate) fn cancel(&self) {
        self.host.cancel();
    }

    /// `endpoints`, at the tests' own server when a test set one.
    fn at(&self, endpoints: Endpoints) -> Endpoints {
        #[cfg(test)]
        if let Some((authorize, token)) = self.server.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return Endpoints { authorize, token, ..endpoints };
        }
        endpoints
    }
}

/// A reply to one call: the queue holds one, never reaches its bound.
impl Shed for Page {
    fn weight(&self) -> Weight {
        Weight::Actionable
    }
}

impl Shed for Checked {
    fn weight(&self) -> Weight {
        Weight::Actionable
    }
}

fn failed(reason: &str) -> SignInOutcome {
    SignInOutcome::Failed { reason: reason.to_string() }
}

fn not_granted(reason: &str) -> PermissionOutcome {
    PermissionOutcome::Failed { reason: reason.to_string() }
}

impl Core {
    /// Run a sign-in on the page on a thread of its own, which waits as long as the owner takes
    /// (five minutes at most), and give what it came to. The page and the waiting are the host's
    /// (`mailcraft_host::signin`); this only lends it a thread and an answer to wait for.
    async fn page(&self, endpoints: Endpoints, hint: String, why: Why, cancel: Arc<AtomicBool>) -> Page {
        let armer = self.signins.host.armer();
        let landed = armer.arm();
        let reply = Queue::<Page>::new();
        let platform = self.native.clone();
        let job_reply = reply.clone();
        let job = move || signin::page(platform.as_ref(), landed, &armer, &endpoints, &hint, why, &cancel);
        // (A panic in the job is an answer, not a wait that never ends.)
        let spawned = std::thread::Builder::new().name("mailcraft-sign-in".to_string()).spawn(move || {
            let page = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or(Page::Unable("the sign-in stopped unexpectedly"));
            job_reply.push(page);
        });
        if spawned.is_err() {
            return Page::Unable("a thread for the sign-in could not be started");
        }
        reply.next().await.unwrap_or(Page::Ended(Ending::Cancelled))
    }

    /// The address and kind of an account of the mailbox.
    async fn email_of(&self, account: u64) -> Result<String, CoreError> {
        self.call(move |s| {
            let found = s.mailbox.accounts.iter().find(|a| a.id == account && a.kind == AccountKind::Microsoft);
            found.map(|a| a.email.clone()).ok_or_else(|| CoreError::Failed { reason: "there is no such account".to_string() })
        })
        .await
    }
}

fn ending_outcome(end: Ending) -> SignInOutcome {
    match end {
        Ending::Approval => SignInOutcome::NeedsApproval,
        Ending::Declined => SignInOutcome::Declined,
        Ending::TimedOut => SignInOutcome::TimedOut,
        Ending::Cancelled => SignInOutcome::Cancelled,
        Ending::Blocked => SignInOutcome::OrganisationBlocks,
        Ending::Other(e) => failed(&e.to_string()),
    }
}

fn refusal_outcome(refusal: FavoritesRefusal) -> PermissionOutcome {
    match refusal {
        FavoritesRefusal::Approval => PermissionOutcome::NeedsApproval,
        FavoritesRefusal::Organisation => PermissionOutcome::OrganisationBlocks,
        FavoritesRefusal::Account => PermissionOutcome::NotForThisAccount,
        FavoritesRefusal::Other(NetError::SignInCancelled) => PermissionOutcome::Cancelled,
        FavoritesRefusal::Other(NetError::SignInTimedOut) => PermissionOutcome::TimedOut,
        // (The owner's own no, or the page left: Microsoft says the same for both.)
        FavoritesRefusal::Other(e) if e.is_refused_sign_in() => PermissionOutcome::Declined,
        FavoritesRefusal::Other(e) => not_granted(&e.to_string()),
    }
}

#[uniffi::export]
impl Core {
    /// Give the address the system opened the app with (the redirect's) to the sign-in that waits
    /// for it, whole and unread. True when a sign-in took it; false when none waits, one already
    /// had an address, or it is too long. Anything else about it is judged in Rust.
    pub fn hand_back_redirect(&self, address: String) -> bool {
        self.signins.host.deliver(address)
    }

    /// Call off the sign-in that is open, if one is: it ends as `Cancelled`.
    pub fn cancel_sign_in(&self) {
        self.signins.host.cancel();
    }

    /// Add a Microsoft account: the provider's page opens in the system browser through the
    /// platform, and what comes back is checked here. `email` is only a hint for the page and
    /// may be empty. On success the account is in the mailbox, its sign-in is in the platform's
    /// credential store (when the owner lets sign-ins be remembered) and Send/Receive has it for
    /// the session; nothing is kept if the store refuses, nor if the call is dropped before the
    /// account is in. `settings` is what the owner chose in Add Account of what the phone keeps
    /// of a new account's mail (its window, and the phone's storage cap); an account the owner
    /// had already keeps its own.
    pub async fn sign_in_microsoft(&self, email: String, settings: crate::limits::DownloadSettings) -> Result<SignInOutcome, CoreError> {
        let (address, tokens) = match self.account_sign_in(signin::hint_of(&email)).await? {
            Ok(signed) => signed,
            Err(outcome) => return Ok(outcome),
        };
        let name = tokens.name.clone();
        let directory = match Directory::of_tenant(&tokens.tenant) {
            Directory::Organisation => Some("organisation"),
            Directory::Personal => Some("personal"),
            Directory::Unknown => None,
        };
        let new_account = json!({
            "email": address, "name": name, "displayName": name, "kind": "microsoft", "auth": "oauth2", "provider": "microsoft", "directory": directory,
            "keepOffline": crate::settings::MOBILE_KEEP_OFFLINE,
        });
        // One job on the session's thread: the sign-in is kept, the account is added and
        // Send/Receive takes the sign-in, or none of it. A call dropped (or a core shut down) before
        // the job runs keeps nothing, and a job that runs needs nobody waiting for it to finish.
        let (platform, sync, for_call, lang) = (self.native.clone(), self.sync.clone(), address.clone(), self.lang());
        let added = self
            .call(move |s| {
                // (An account the owner has signs in again: its settings stay as they are. A shared
                // mailbox is read with another account's sign-in and never signs in itself.)
                let known = s
                    .mailbox
                    .accounts
                    .iter()
                    .find(|a| a.kind == AccountKind::Microsoft && !a.is_shared() && a.email.trim().eq_ignore_ascii_case(&for_call));
                let known = known.map(|a| a.id);
                // Kept in the credential store only when the owner lets sign-ins be remembered; else
                // Send/Receive holds it for the session alone. (What was kept before for an account
                // the owner has goes back if Send/Receive refuses the new sign-in.)
                let remember = sync.remembers();
                let before = if remember && known.is_some() { signin::kept_refresh(platform.as_ref(), &for_call) } else { None };
                if remember && let Err(why) = signin::keep_refresh(platform.as_ref(), &for_call, &tokens.refresh) {
                    return Ok(Err(why.to_string()));
                }
                let undo_kept = |platform: &dyn mailcraft_host::Platform| {
                    if remember {
                        signin::restore_refresh(platform, &for_call, before.as_deref());
                    }
                };
                // (The storage cap is the mailbox's, not the account's: a new account's choice of it
                // goes again with the account when its setup fails.)
                let limits_before = s.mailbox.device_limits;
                let restore_limits = |s: &mut mailcraft_engine::Session| {
                    if s.mailbox.device_limits != limits_before {
                        s.mailbox.device_limits = limits_before;
                        s.mailbox_changed();
                    }
                };
                let account = match known {
                    Some(id) => id,
                    None => {
                        let made = s.run("account.add", &new_account).map_err(CoreError::from);
                        // The phone keeps what the owner chose of a new account's mail (MB3.16).
                        let made = made.and_then(|m| m.get("id").and_then(Value::as_u64).ok_or(CoreError::Internal)).and_then(|id| {
                            match crate::limits::apply_to(s, id, settings) {
                                Ok(()) => Ok(id),
                                Err(e) => {
                                    let _ = s.run("account.remove", &json!({"id": id}));
                                    restore_limits(s);
                                    Err(CoreError::from(e))
                                }
                            }
                        });
                        match made {
                            Ok(id) => id,
                            Err(error) => {
                                // (Nothing is kept of a sign-in that made no account.)
                                undo_kept(platform.as_ref());
                                return Err(error);
                            }
                        }
                    }
                };
                // Send/Receive has the sign-in for the session at once, and an account that had to
                // be signed in again no longer does. When it refuses the sign-in, nothing of it is
                // left: an account made for it goes again, and the credential store has what it had.
                let adopted = sync.adopt(s, account, tokens);
                if let Some(why) = crate::sync_api::adopt_refusal(lang, adopted) {
                    if known.is_none() && s.run("account.remove", &json!({"id": account})).is_err() {
                        log::warn!("accounts: an account added for a refused sign-in couldn't be removed again");
                    }
                    if known.is_none() {
                        restore_limits(s);
                    }
                    undo_kept(platform.as_ref());
                    return Ok(Err(why));
                }
                Ok(Ok(account))
            })
            .await;
        match added {
            Ok(Ok(account)) => Ok(SignInOutcome::SignedIn { account, email: address, name }),
            Ok(Err(reason)) => Ok(SignInOutcome::Failed { reason }),
            Err(CoreError::Cancelled) => Err(CoreError::Cancelled),
            Err(CoreError::Failed { reason } | CoreError::BadRequest { reason }) => Ok(SignInOutcome::Failed { reason }),
            Err(_) => Ok(failed("the account could not be added")),
        }
    }

    /// Ask the owner, on Microsoft's page, for `permission` for `account`, by itself and never
    /// with a sign-in. Ask the owner first in the app: this opens the page. The new sign-in
    /// replaces the old in the credential store only if it is the account's own.
    pub async fn grant_permission(&self, account: u64, permission: Permission) -> Result<PermissionOutcome, CoreError> {
        let email = self.email_of(account).await?;
        let Some(place) = self.signins.begin() else { return Ok(not_granted("a sign-in is already open")) };
        // (Looked at with the place held: `shutdown` calls off the one that is open, and none opens after it.)
        if self.closing.load(Ordering::Acquire) {
            return Err(CoreError::Cancelled);
        }
        let endpoints = match signin::client_oauth() {
            Some(o) => self.signins.at(permission_endpoints(&o, permission.into())),
            None => return Ok(not_granted("this build of MailCraft has no Microsoft sign-in set up")),
        };
        let page = self.page(endpoints, email.clone(), Why::Permission, place.cancel_flag()).await;
        let tokens = match page {
            Page::Tokens(tokens) => tokens,
            Page::Refused(refusal) => return Ok(refusal_outcome(refusal)),
            Page::NoBrowser => return Ok(PermissionOutcome::NoBrowser),
            Page::Unable(why) => return Ok(not_granted(why)),
            Page::Ended(_) => return Ok(not_granted("the sign-in stopped unexpectedly")),
        };
        // The owner picks the account on Microsoft's page: somebody else's sign-in is not this account's.
        if tokens.someone_else(&email, "") {
            return Ok(not_granted("that sign-in was for another account"));
        }
        // Kept on the session's thread, after a look that the account is still there: removing it
        // is on that thread too, so a sign-in finishing after a removal never restores its credential.
        // (With sign-ins not remembered nothing is written: the permission is Microsoft's to keep.)
        let (platform, tokens_refresh, for_email, sync) = (self.native.clone(), tokens.refresh.clone(), email.clone(), self.sync.clone());
        let kept = self
            .call(move |s| {
                let there = s.mailbox.accounts.iter().any(|a| a.id == account && a.kind == AccountKind::Microsoft);
                let kept = match (there, sync.remembers()) {
                    (false, _) => None,
                    (true, true) => Some(signin::keep_refresh(platform.as_ref(), &for_email, &tokens_refresh)),
                    (true, false) => Some(Ok(())),
                };
                if matches!(kept, Some(Ok(()))) {
                    // The sign-in that covers it is the account's newest: the session's too, so that
                    // a job has it with sign-ins not remembered (never its access token as a mail one).
                    let theirs = Tokens { refresh: tokens_refresh.clone(), ..Default::default() };
                    sync.adopt_refresh(s, account, &theirs);
                    // Allowed, as far as is known: the next to-do job finds out, at once (MB3.18).
                    if permission == Permission::Tasks {
                        s.driver.tasks.allowed(account);
                    }
                    sync.pacer.poke();
                }
                Ok(kept)
            })
            .await?;
        match kept {
            Some(Ok(())) => Ok(PermissionOutcome::Granted),
            Some(Err(why)) => Ok(not_granted(why)),
            None => Ok(not_granted("there is no such account")),
        }
    }

    /// Whether `account`'s sign-in covers `permission`, asked of Microsoft with the account's
    /// refresh token as Send/Receive has it: the session's first (a sign-in the owner doesn't let
    /// be remembered is in memory alone), then the credential store's. Nothing is opened and the
    /// owner is not asked. Nothing is stored of the answer.
    pub async fn permission_state(&self, account: u64, permission: Permission) -> Result<PermissionState, CoreError> {
        let email = self.email_of(account).await?;
        let Some(o) = signin::client_oauth() else { return Ok(PermissionState::Unknown) };
        // (Only the tokens' scopes are asked for here; the phone's endpoints all the same, #661.)
        let mail = self.signins.at(Endpoints::for_resource_without_sync(&o, Resource::GraphMail));
        let graph = self.signins.at(Endpoints::for_resource_without_sync(&o, Resource::Graph));
        let sync = self.sync.clone();
        let refresh = self.call(move |s| Ok(sync.refresh_for(s, account))).await?;
        let reply = Queue::<Checked>::new();
        let job_reply = reply.clone();
        let for_job = email.clone();
        let job = move || signin::permission_standing(refresh.as_deref(), &for_job, permission.into(), &mail, &graph);
        let spawned = std::thread::Builder::new().name("mailcraft-permission".to_string()).spawn(move || {
            let checked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or(Checked { standing: Standing::Unknown, renewed: None });
            job_reply.push(checked);
        });
        if spawned.is_err() {
            return Ok(PermissionState::Unknown);
        }
        let Some(Checked { standing, renewed }) = reply.next().await else { return Ok(PermissionState::Unknown) };
        // The check writes nothing itself. A sign-in Microsoft renewed in its answer is kept here, on
        // the session's thread, and only while its account is still there: removing it is on that
        // thread too, so an answer that comes after a removal never puts its sign-in back, and a
        // call dropped meanwhile (or a core shutting down) keeps nothing either.
        if let Some(renewed) = renewed {
            let (platform, sync) = (self.native.clone(), self.sync.clone());
            let _ = self
                .call(move |s| {
                    // The very account that was checked (one removed and added again is another, with
                    // its own sign-in), and only in place of the sign-in the check began with (the
                    // session's, and the stored one when the owner lets sign-ins be remembered): one
                    // forgotten, or replaced by a newer sign-in, meanwhile is never written over.
                    let there = s
                        .mailbox
                        .accounts
                        .iter()
                        .any(|a| a.id == account && a.kind == AccountKind::Microsoft && a.email.trim().eq_ignore_ascii_case(&email));
                    if there {
                        // The session's sign-in, whether or not it is remembered, and the stored
                        // one when the owner lets it be.
                        sync.take_renewed(s, account, &renewed);
                        if sync.remembers() {
                            renewed.keep(platform.as_ref(), &email);
                        }
                    }
                    Ok(())
                })
                .await;
        }
        Ok(PermissionState::from(standing))
    }

    /// The address of Microsoft's page where an administrator approves MailCraft for an
    /// organisation, once for everybody in it, for the owner to pass on. Nothing is opened or
    /// sent. `email` names the organisation's directory by its domain when it can; it may be empty.
    /// The address names the application, the directory and the permissions, and nothing of the owner.
    pub fn admin_approval_link(&self, email: String) -> Result<String, CoreError> {
        let o = signin::client_oauth()
            .ok_or_else(|| CoreError::Failed { reason: "this build of MailCraft has no Microsoft sign-in set up".to_string() })?;
        admin_consent_url(&o, &signin::tenant_of(&email))
            .map_err(|_| CoreError::Failed { reason: "the address of the approval page could not be made".to_string() })
    }

    /// Remove an account: its mail and what is kept for it on this phone (`account.remove`), and
    /// its sign-in from the platform's credential store. Nothing on the server is touched. The
    /// sign-in is removed only once the account is.
    /// Add a mail account that signs in with a password: IMAP to read, SMTP to send. The
    /// account is checked by the engine (its address, its servers), and its password is kept in
    /// memory for this session and, when the owner lets sign-ins be remembered, in the phone's
    /// secure store. Nothing is fetched here: the caller's first Send/Receive does that.
    pub async fn add_imap_account(&self, settings: ImapAccountSettings) -> Result<u64, CoreError> {
        let sync = self.sync.clone();
        let platform = self.platform.clone();
        let remember = sync.remembers();
        self.call(move |s| {
            let email = settings.email.trim().chars().take(254).collect::<String>();
            if email.is_empty() {
                return Err(CoreError::BadRequest { reason: "the account's address is empty".to_string() });
            }
            if settings.password.is_empty() || settings.password.len() > 1024 {
                return Err(CoreError::BadRequest { reason: "the account's password is empty or too long".to_string() });
            }
            let server = |server: &MailServer| {
                json!({"host": server.host.trim().chars().take(255).collect::<String>(), "port": server.port, "security": server.security.trim()})
            };
            let username = if settings.username.trim().is_empty() { email.clone() } else { settings.username.trim().to_string() };
            let mut params = json!({
                "email": email,
                "username": username,
                "password": settings.password.clone(),
                "incoming": server(&settings.incoming),
                "outgoing": server(&settings.outgoing),
                "auth": "password",
                "kind": "imap",
            });
            if !settings.display_name.trim().is_empty() {
                params["displayName"] = json!(settings.display_name.trim().chars().take(254).collect::<String>());
            }
            let id = s.run("account.add", &params)?.get("id").and_then(Value::as_u64).ok_or_else(|| CoreError::Failed {
                reason: "the account was added but the core didn't say which".to_string(),
            })?;
            let account = s.mailbox.account(id).cloned().ok_or_else(|| CoreError::Failed {
                reason: "the account was added but is not in the mailbox".to_string(),
            })?;
            sync.keep_password(id, settings.password.clone());
            if remember
                && let Ok(Some(store)) = platform.secrets()
            {
                // (A store that refuses doesn't undo the account: the password stays in memory
                // for this session, and the owner is asked again at the next start.)
                let _ = store.set(mailcraft_host::secrets::secret_key("password", &account), settings.password);
            }
            Ok(id)
        })
        .await
    }

    /// Add an Exchange server account: Exchange Web Services for its mail, calendars and address
    /// books. Its password is kept in memory for this session and, when the owner lets sign-ins
    /// be remembered, in the phone's secure store. Nothing is read here: the caller's Send/Receive
    /// does that.
    pub async fn add_exchange_account(&self, settings: ExchangeAccountSettings) -> Result<u64, CoreError> {
        let sync = self.sync.clone();
        let platform = self.platform.clone();
        let remember = sync.remembers();
        self.call(move |s| {
            let email = settings.email.trim().chars().take(254).collect::<String>();
            if email.is_empty() {
                return Err(CoreError::BadRequest { reason: "the account's address is empty".to_string() });
            }
            if settings.password.is_empty() || settings.password.len() > 1024 {
                return Err(CoreError::BadRequest { reason: "the account's password is empty or too long".to_string() });
            }
            let url = settings.url.trim().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1:") || url.starts_with("http://[::1]:")) {
                return Err(CoreError::BadRequest { reason: "the server's address is an https:// address".to_string() });
            }
            let server = mailcraft_model::EwsServer { url: url.clone(), ..Default::default() };
            if server.host().is_some_and(mailcraft_model::is_exchange_online) {
                return Err(CoreError::BadRequest { reason: "this server is Microsoft's: add it as a Microsoft account".to_string() });
            }
            let username = if settings.username.trim().is_empty() { email.clone() } else { settings.username.trim().to_string() };
            let mut params = json!({
                "email": email,
                "username": username,
                "auth": "password",
                "kind": "exchange",
                "ews": {"url": url, "username": username},
            });
            if !settings.display_name.trim().is_empty() {
                params["displayName"] = json!(settings.display_name.trim().chars().take(254).collect::<String>());
            }
            let id = s
                .run("account.add", &params)?
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| CoreError::Failed { reason: "the account was added but the core didn't say which".to_string() })?;
            let account = s
                .mailbox
                .account(id)
                .cloned()
                .ok_or_else(|| CoreError::Failed { reason: "the account was added but is not in the mailbox".to_string() })?;
            sync.keep_password(id, settings.password.clone());
            if remember && let Ok(Some(store)) = platform.secrets() {
                let _ = store.set(mailcraft_host::secrets::secret_key("password", &account), settings.password);
            }
            Ok(id)
        })
        .await
    }

    /// An account's settings, for the Edit form: its address and names, its servers, and an
    /// Exchange server account's Exchange Web Services address. Never its password.
    pub async fn account_settings(&self, account: u64) -> Result<AccountSettings, CoreError> {
        self.call(move |s| {
            let a = s.mailbox.account(account).ok_or_else(|| CoreError::BadRequest { reason: "there is no such account".to_string() })?;
            let server = |server: &Option<mailcraft_model::Server>| {
                server.as_ref().map(|server| MailServer {
                    host: server.host.clone(),
                    port: server.port,
                    security: match server.security {
                        mailcraft_model::Security::Tls => "tls",
                        mailcraft_model::Security::StartTls => "starttls",
                        mailcraft_model::Security::None => "none",
                    }
                    .to_string(),
                })
            };
            Ok(AccountSettings {
                email: a.email.clone(),
                display_name: a.display_name.clone(),
                username: a.username.clone(),
                incoming: server(&a.incoming),
                outgoing: server(&a.outgoing),
                url: a.ews.as_ref().map(|server| server.url.clone()).unwrap_or_default(),
            })
        })
        .await
    }

    /// Try an IMAP account's servers and password before it is added or changed: the host opens
    /// the connection and signs in, and answers what the server said (never the password). The
    /// account is not touched.
    pub async fn check_imap_account(&self, settings: ImapAccountSettings) -> Result<(), CoreError> {
        let (email, username, password) = checked_password(&settings)?;
        let incoming = model_server(&settings.incoming)?;
        let outgoing = model_server(&settings.outgoing)?;
        let account = mailcraft_model::Account {
            email,
            name: String::new(),
            username,
            kind: AccountKind::Imap,
            incoming: Some(incoming),
            outgoing: Some(outgoing),
            ..Default::default()
        };
        let answer = Queue::<Result<(), String>>::new();
        let sender = answer.clone();
        let spawned = std::thread::Builder::new().name("mailcraft-check".to_string()).spawn(move || {
            sender.push(mailcraft_net::test_account(&account, &password).map_err(|e| e.said().to_string()));
        });
        if spawned.is_err() {
            return Err(CoreError::Failed { reason: "the check for the account couldn't be started".to_string() });
        }
        match answer.next().await {
            Some(Ok(())) => Ok(()),
            Some(Err(why)) => Err(CoreError::Failed { reason: why }),
            None => Err(CoreError::Failed { reason: "the check for the account stopped unexpectedly".to_string() }),
        }
    }

    /// Change an IMAP account's settings: its address, names and servers, and its password when
    /// one is given (an empty one keeps the password it has). The password is kept in memory for
    /// this session and, when the owner lets sign-ins be remembered, in the phone's secure store
    /// under the account's new address. Nothing is read from the server here.
    pub async fn edit_imap_account(&self, account: u64, settings: ImapAccountSettings) -> Result<(), CoreError> {
        let sync = self.sync.clone();
        let platform = self.platform.clone();
        let remember = sync.remembers();
        self.call(move |s| {
            let before =
                s.mailbox.account(account).cloned().ok_or_else(|| CoreError::BadRequest { reason: "there is no such account".to_string() })?;
            if before.kind != AccountKind::Imap {
                return Err(CoreError::BadRequest { reason: "this account isn't an IMAP account".to_string() });
            }
            let email = settings.email.trim().chars().take(254).collect::<String>();
            if email.is_empty() {
                return Err(CoreError::BadRequest { reason: "the account's address is empty".to_string() });
            }
            let params = json!({
                "id": account,
                "email": email,
                "displayName": settings.display_name.trim().chars().take(254).collect::<String>(),
                "username": if settings.username.trim().is_empty() { email } else { settings.username.trim().chars().take(254).collect::<String>() },
                "incoming": server_json(&settings.incoming),
                "outgoing": server_json(&settings.outgoing),
            });
            s.run("account.edit", &params).map_err(CoreError::from)?;
            let after = s
                .mailbox
                .account(account)
                .cloned()
                .ok_or_else(|| CoreError::Failed { reason: "the account was changed but is not in the mailbox".to_string() })?;
            keep_changed_password(sync.as_ref(), platform.as_ref(), &before, &after, &settings.password, remember);
            Ok(())
        })
        .await
    }

    /// Change an Exchange server account's settings: its address, names and Exchange Web
    /// Services address, and its password when one is given (an empty one keeps the one it has).
    pub async fn edit_exchange_account(&self, account: u64, settings: ExchangeAccountSettings) -> Result<(), CoreError> {
        let sync = self.sync.clone();
        let platform = self.platform.clone();
        let remember = sync.remembers();
        self.call(move |s| {
            let before =
                s.mailbox.account(account).cloned().ok_or_else(|| CoreError::BadRequest { reason: "there is no such account".to_string() })?;
            if before.kind != AccountKind::Exchange {
                return Err(CoreError::BadRequest { reason: "this account isn't an Exchange server account".to_string() });
            }
            let email = settings.email.trim().chars().take(254).collect::<String>();
            if email.is_empty() {
                return Err(CoreError::BadRequest { reason: "the account's address is empty".to_string() });
            }
            let url = settings.url.trim().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://127.0.0.1:") || url.starts_with("http://[::1]:")) {
                return Err(CoreError::BadRequest { reason: "the server's address is an https:// address".to_string() });
            }
            let username = if settings.username.trim().is_empty() { email.clone() } else { settings.username.trim().to_string() };
            let mut ews = json!({"url": url, "username": username});
            // (A certificate the owner already trusted for this account stays trusted.)
            if let Some(trusted) = before.ews.as_ref().map(|server| server.trusted.clone()).filter(|trusted| !trusted.is_empty()) {
                ews["trusted"] = json!(trusted);
            }
            let params = json!({
                "id": account,
                "email": email,
                "displayName": settings.display_name.trim().chars().take(254).collect::<String>(),
                "username": username,
                "ews": ews,
            });
            s.run("account.edit", &params).map_err(CoreError::from)?;
            let after = s
                .mailbox
                .account(account)
                .cloned()
                .ok_or_else(|| CoreError::Failed { reason: "the account was changed but is not in the mailbox".to_string() })?;
            keep_changed_password(sync.as_ref(), platform.as_ref(), &before, &after, &settings.password, remember);
            Ok(())
        })
        .await
    }

    pub async fn remove_account(&self, account: u64) -> Result<(), CoreError> {
        // One job: the account goes and, when no other account has its address, so does its sign-in,
        // whatever becomes of the caller (a shutdown between two calls would leave the credential).
        let native = self.native.clone();
        let platform = self.platform.clone();
        let sync = self.sync.clone();
        self.call(move |s| {
            let before = s.mailbox.account(account).cloned();
            let email = before.as_ref().map(|a| a.email.clone()).unwrap_or_default();
            s.run("account.remove", &json!({"id": account})).map_err(CoreError::from)?;
            // What was being fetched for it is called off, and nothing it brings is taken in.
            sync.cancel_fetches(Some(account));
            sync.forget_password(account);
            // (Nothing else of the account's is held under its address: a mailbox with another account
            // of the same address keeps it.)
            let still = s.mailbox.accounts.iter().any(|a| a.kind == AccountKind::Microsoft && a.email.trim().eq_ignore_ascii_case(&email));
            let microsoft = before.as_ref().is_none_or(|a| a.kind == AccountKind::Microsoft);
            // (The account stays removed when the store refuses: the owner is told its sign-in is still kept.)
            if microsoft && !still && signin::forget(native.as_ref(), &email).is_err() {
                return Err(CoreError::Failed {
                    reason: "the account was removed, but its sign-in couldn't be removed from this phone's secure store".to_string(),
                });
            }
            // An account that signed in with a password keeps it under a name of its own; another
            // account with the same servers and address still needs it.
            if let Some(before) = before.filter(|a| a.kind != AccountKind::Microsoft) {
                let key = mailcraft_host::secrets::secret_key("password", &before);
                let still = s.mailbox.accounts.iter().any(|a| mailcraft_host::secrets::secret_key("password", a) == key);
                if !still
                    && let Ok(Some(store)) = platform.secrets()
                    && store.delete(key).is_err()
                {
                    return Err(CoreError::Failed {
                        reason: "the account was removed, but its password couldn't be removed from this phone's secure store".to_string(),
                    });
                }
            }
            Ok(())
        })
        .await?;
        Ok(())
    }
}

impl Core {
    /// The one sign-in on the page of a phone, for Add Account and for an account signing in
    /// again ([`Core::sign_in`]): one at a time, its redirect handed over by
    /// [`Core::hand_back_redirect`] and called off by [`Core::cancel_sign_in`]. Gives who signed in
    /// (as Microsoft names them, never the hint) and the tokens, or how it ended.
    pub(crate) async fn account_sign_in(&self, hint: String) -> Result<Result<(String, Tokens), SignInOutcome>, CoreError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(CoreError::Cancelled);
        }
        let Some(place) = self.signins.begin() else { return Ok(Err(failed("a sign-in is already open"))) };
        // (Looked at again with the place held: `shutdown` calls off the one that is open.)
        if self.closing.load(Ordering::Acquire) {
            return Err(CoreError::Cancelled);
        }
        let endpoints = match signin::client_oauth() {
            // (The page of a phone's sign-in asks for nothing of the to-dos, the rules, the automatic
            // replies and the categories: the to-dos are asked for by themselves, and the rest the
            // phone doesn't keep in step, #661.)
            Some(o) => self.signins.at(Endpoints::for_resource_without_sync(&o, Resource::GraphMail)),
            None => return Ok(Err(failed("this build of MailCraft has no Microsoft sign-in set up"))),
        };
        let page = self.page(endpoints, hint, Why::Account, place.cancel_flag()).await;
        // (The place is given up here, before the account is dealt with: the page is over.)
        drop(place);
        let tokens = match page {
            Page::Tokens(tokens) => tokens,
            Page::Ended(end) => return Ok(Err(ending_outcome(end))),
            Page::NoBrowser => return Ok(Err(SignInOutcome::NoBrowser)),
            Page::Unable(why) => return Ok(Err(failed(why))),
            Page::Refused(_) => return Ok(Err(failed("the sign-in stopped unexpectedly"))),
        };
        // Who signed in is who Microsoft says (the owner picks the account on its page), never
        // what was typed: the typed address is only a hint for the page.
        match signin::signed_in_address(&tokens) {
            Some(address) => Ok(Ok((address, tokens))),
            None => Ok(Err(failed("Microsoft didn't say which account signed in"))),
        }
    }
}

/// What the add and check forms must say: the address, the name the servers know, and a password.
fn checked_password(settings: &ImapAccountSettings) -> Result<(String, String, String), CoreError> {
    let email = settings.email.trim().chars().take(254).collect::<String>();
    if email.is_empty() {
        return Err(CoreError::BadRequest { reason: "the account's address is empty".to_string() });
    }
    if settings.password.is_empty() || settings.password.len() > 1024 {
        return Err(CoreError::BadRequest { reason: "the account's password is empty or too long".to_string() });
    }
    let username = if settings.username.trim().is_empty() { email.clone() } else { settings.username.trim().chars().take(254).collect::<String>() };
    Ok((email, username, settings.password.clone()))
}

/// A server of the form as the model has it.
fn model_server(server: &MailServer) -> Result<mailcraft_model::Server, CoreError> {
    let host = server.host.trim().chars().take(255).collect::<String>();
    if host.is_empty() {
        return Err(CoreError::BadRequest { reason: "the server's address is empty".to_string() });
    }
    let security = mailcraft_model::Security::parse(&server.security)
        .ok_or_else(|| CoreError::BadRequest { reason: "a server is encrypted from the first byte (tls) or with STARTTLS (starttls)".to_string() })?;
    Ok(mailcraft_model::Server { host, port: server.port, security })
}

fn server_json(server: &MailServer) -> Value {
    json!({
        "host": server.host.trim().chars().take(255).collect::<String>(),
        "port": server.port,
        "security": server.security.trim(),
    })
}

/// Keep the password an edit brought, under the account's new address: in memory for this
/// session, and in the store when the owner lets sign-ins be remembered. A password the edit
/// didn't give is the one the account has, kept as it was (the account's address may have
/// changed, and the key with it). The old key is removed when the address changed.
fn keep_changed_password(
    sync: &crate::sync::SyncHost,
    platform: &dyn crate::platform::Platform,
    before: &mailcraft_model::Account,
    after: &mailcraft_model::Account,
    given: &str,
    remember: bool,
) {
    let old_key = mailcraft_host::secrets::secret_key("password", before);
    let new_key = mailcraft_host::secrets::secret_key("password", after);
    let password = if given.is_empty() {
        sync.password(after.id)
            .or_else(|| if remember { platform.secrets().ok().flatten().and_then(|store| store.get(old_key.clone()).ok().flatten()) } else { None })
    } else {
        Some(given.to_string())
    };
    if let Some(password) = password.filter(|password| !password.is_empty()) {
        sync.keep_password(after.id, password.clone());
        if remember && let Ok(Some(store)) = platform.secrets() {
            let _ = store.set(new_key.clone(), password);
        }
    }
    if old_key != new_key
        && let Ok(Some(store)) = platform.secrets()
    {
        let _ = store.delete(old_key);
    }
}
