// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Address books of a calendar and contacts server (#209): the books a home lists and the cards
//! in them, read for the engine (`mailcraft_engine::cmd::remote::apply_book_contacts`), and the
//! contacts made, changed or deleted here, written back to them with the condition that the
//! server's copy is still the one the change was made on.
//!
//! Nothing is sent but to the origin of the card's own home, and nothing of an address or a
//! password is logged. A card changed here is written into the document the server sent
//! (`mailcraft_vcard::patch`): its photo, its keys and another program's properties stay.

use std::collections::HashMap;

use mailcraft_dav::answer::Multistatus;
use mailcraft_dav::href::TrustedOrigin;
use mailcraft_dav::http::{HttpRequest, Secret};
use mailcraft_dav::request::{Endpoint, Kind, MAX_MULTIGET};
use mailcraft_model::contact_sync::ContactField;
use mailcraft_model::remote::{
    ContactChangeKind, ContactOp, ContactOutcome, ContactsDownload, ContactsUpload, ContactsUploaded, EventResult, RemoteAddressBook, RemoteContact,
};
use mailcraft_model::{Account, Contact, DavProblem, Rights, origin_of};

/// Most cards kept from one read (a server that lists more is treated as too large).
const MOST_CONTACTS: usize = 20_000;
/// Longest answer read from one request (as the DAV requests themselves allow).
const MAX_ANSWER: usize = 8 * 1024 * 1024;

/// What a read of an account's address books came to: the cards, and why some may be missing.
pub use mailcraft_model::ContactRead;

/// A server's answer: the status, and the body within the request's limit.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// How a request is made: over the real network ([`over_the_network`]), or by a test with the
/// stand-in server.
pub type Send<'a> = &'a dyn Fn(HttpRequest, &str) -> Option<Reply>;

/// Read every contact home of `account`. `passwords` holds the password for each origin its homes
/// are on, and only that origin's is ever sent there.
pub fn read(account: &Account, passwords: &HashMap<String, String>) -> ContactRead {
    read_with(account, passwords, &over_the_network)
}

/// [`read`] with the exchange given.
pub fn read_with(account: &Account, passwords: &HashMap<String, String>, send: Send<'_>) -> ContactRead {
    let mut out = ContactsDownload { account: account.id, email: account.email.clone(), complete: true, ..Default::default() };
    let Some(server) = account.dav.as_ref() else {
        out.complete = false;
        return ContactRead { download: out, problem: DavProblem::ServerErrors };
    };
    let mut problem = DavProblem::None;
    for home in server.contact_homes.iter().take(mailcraft_model::DavServer::MAX_HOMES) {
        // (A home is a collection: the server lists what is under it for a request to its own
        // address, which ends in a slash.)
        let home = if home.ends_with('/') { home.clone() } else { format!("{home}/") };
        let home = home.as_str();
        let Some(origin) = origin_of(home) else {
            out.complete = false;
            continue;
        };
        let Some(password) = passwords.get(&origin) else {
            out.complete = false;
            continue;
        };
        let Some(endpoint) = endpoint(account, home, password) else {
            out.complete = false;
            continue;
        };
        let Some(listing) = fetch(endpoint.propfind_collections(home), &server.trusted, send) else {
            out.complete = false;
            continue;
        };
        let Some(books) = Multistatus::parse(&listing, endpoint.origin(), home).ok() else {
            out.complete = false;
            continue;
        };
        if books.truncated || books.capped {
            out.complete = false;
        }
        for book in books.members().filter(|entry| entry.is_addressbook && !entry.is_base && entry.status < 400) {
            let rights = if book.read_only.unwrap_or(true) { Rights::Read } else { Rights::Edit };
            let name = book.display_name.clone().unwrap_or_default();
            out.books.push(RemoteAddressBook { id: book.href.clone(), name: name.clone(), rights });
            read_book(&endpoint, book.href.as_str(), &name, &server.trusted, &mut out, send);
        }
    }
    if out.contacts.len() > MOST_CONTACTS {
        out.complete = false;
        out.contacts.truncate(MOST_CONTACTS);
    }
    if !out.complete && problem == DavProblem::None {
        problem = DavProblem::ServerErrors;
    }
    ContactRead { download: out, problem }
}

/// Read one address book's cards, adding them to `out`. A book that can't be read whole makes the
/// whole download incomplete.
fn read_book(endpoint: &Endpoint, book: &str, name: &str, trusted: &str, out: &mut ContactsDownload, send: Send<'_>) {
    let Some(listing) = fetch(endpoint.propfind_members(book), trusted, send) else {
        out.complete = false;
        return;
    };
    let Some(entries) = Multistatus::parse(&listing, endpoint.origin(), book).ok() else {
        out.complete = false;
        return;
    };
    if entries.truncated || entries.capped {
        out.complete = false;
    }
    let mut hrefs: Vec<&str> =
        entries.members().filter(|entry| !entry.is_collection && !entry.is_base && entry.status < 400).map(|entry| entry.href.as_str()).collect();
    if hrefs.len() > MOST_CONTACTS {
        out.complete = false;
        hrefs.truncate(MOST_CONTACTS);
    }
    for chunk in hrefs.chunks(MAX_MULTIGET) {
        let Ok(request) = endpoint.multiget(Kind::AddressBook, book, chunk) else {
            out.complete = false;
            break;
        };
        let Some(answer) = fetch(Ok(request), trusted, send) else {
            out.complete = false;
            break;
        };
        let Some(cards) = Multistatus::parse(&answer, endpoint.origin(), book).ok() else {
            out.complete = false;
            break;
        };
        if cards.truncated || cards.capped {
            out.complete = false;
        }
        for entry in cards.members() {
            if entry.data_too_large {
                out.complete = false;
                continue;
            }
            let Some(data) = entry.data.as_deref() else {
                // (A member the listing named but this answer left out: nothing is removed for it.)
                if entry.status < 400 {
                    out.complete = false;
                }
                continue;
            };
            for contact in mailcraft_vcard::parse(data) {
                out.contacts.push(remote(&contact, entry.href.as_str(), entry.etag.as_deref().unwrap_or_default(), name, book));
            }
        }
    }
}

/// A card's fields as a server's copy of a contact.
pub fn remote(contact: &Contact, id: &str, etag: &str, name: &str, book: &str) -> RemoteContact {
    RemoteContact {
        id: id.to_string(),
        change_key: etag.to_string(),
        uid: contact.uid.clone(),
        display_name: contact.display_name.clone(),
        first: contact.first.clone(),
        middle: contact.middle.clone(),
        last: contact.last.clone(),
        nickname: contact.nickname.clone(),
        company: contact.company.clone(),
        title: contact.title.clone(),
        department: contact.department.clone(),
        emails: contact.emails.clone(),
        phones: contact.phones.clone(),
        addresses: contact.addresses.clone(),
        websites: contact.websites.clone(),
        birthday: contact.birthday,
        notes: contact.notes.clone(),
        book: name.to_string(),
        book_id: book.to_string(),
        modified: 0,
        categories: Some(contact.categories.clone()),
    }
}

/// Send the contacts made, changed or deleted here to `account`'s address books (#209): each only
/// while the server still has the version the change was made on (`If-Match`; a creation with
/// `If-None-Match: *`). A change the server has changed since is answered 412, which is never
/// retried as an overwrite: the engine holds it for the owner (`sync.davConflicts`). An op marked
/// `force` is the owner's own choice to keep what is here, put over the server's copy.
pub fn push(account: &Account, upload: &ContactsUpload, passwords: &HashMap<String, String>) -> ContactsUploaded {
    push_with(account, upload, passwords, &over_the_network)
}

/// [`push`] with the exchange given.
pub fn push_with(account: &Account, upload: &ContactsUpload, passwords: &HashMap<String, String>, send: Send<'_>) -> ContactsUploaded {
    let mut out = ContactsUploaded { account: upload.account, email: upload.email.clone(), ..Default::default() };
    let Some(server) = account.dav.as_ref() else { return out };
    for op in upload.ops.iter().take(mailcraft_model::remote::MAX_CONTACT_OPS) {
        let origin = origin_of(&op.book);
        let password = origin.as_ref().and_then(|o| passwords.get(o));
        let Some((_origin, password)) = origin.zip(password) else {
            out.outcomes.push(failed(op, 0, "the address book's server has no password here"));
            continue;
        };
        let Some(endpoint) = endpoint(account, &op.book, password) else {
            out.outcomes.push(failed(op, 0, "the address book's address can't be used"));
            continue;
        };
        out.outcomes.push(one(&endpoint, op, &server.trusted, send));
    }
    out
}

/// One contact's change, done or not.
fn one(endpoint: &Endpoint, op: &ContactOp, trusted: &str, send: Send<'_>) -> ContactOutcome {
    let outcome = |result: EventResult, status: u16, remote: String, error: &str| ContactOutcome {
        op: op.clone(),
        result,
        remote,
        status,
        error: error.to_string(),
    };
    match op.kind {
        ContactChangeKind::Create => {
            let name = resource_name(op);
            let Ok((url, request)) = endpoint.put_create(Kind::AddressBook, &op.book, &name, &op.card) else {
                return outcome(EventResult::Failed, 0, String::new(), "the card can't be named from here");
            };
            match send(request, trusted) {
                Some(reply) if (200..300).contains(&reply.status) => outcome(EventResult::Done, reply.status, url, ""),
                // (A resource of that name is there already: a creation tried again whose answer
                // was lost. The card is the one it was made from, so it is taken as made.)
                Some(reply) if reply.status == 412 => outcome(EventResult::Done, reply.status, url, ""),
                Some(reply) if reply.status == 401 || reply.status == 403 => {
                    outcome(EventResult::Denied, reply.status, String::new(), "the server refused the sign-in")
                }
                Some(reply) => outcome(EventResult::Failed, reply.status, String::new(), "the server wouldn't take the contact now"),
                None => outcome(EventResult::Failed, 0, String::new(), "the server couldn't be reached"),
            }
        }
        ContactChangeKind::Update => {
            let Some(raw) = get(endpoint, &op.remote, trusted, send) else {
                return outcome(EventResult::Failed, 0, op.remote.clone(), "the card couldn't be read from the server");
            };
            match patched(&raw, op) {
                Some(body) => {
                    let request = if op.force {
                        endpoint.put_over(Kind::AddressBook, &op.book, &op.remote, &body)
                    } else {
                        endpoint.put_change(Kind::AddressBook, &op.book, &op.remote, &op.etag, &body)
                    };
                    let Ok(request) = request else {
                        return outcome(EventResult::Failed, 0, op.remote.clone(), "the change can't be sent from here");
                    };
                    match send(request, trusted) {
                        Some(reply) if (200..300).contains(&reply.status) => outcome(EventResult::Done, reply.status, op.remote.clone(), ""),
                        Some(reply) if reply.status == 404 => outcome(EventResult::Gone, reply.status, op.remote.clone(), ""),
                        Some(reply) if reply.status == 401 || reply.status == 403 => {
                            outcome(EventResult::Denied, reply.status, op.remote.clone(), "the server refused the change")
                        }
                        Some(reply) if reply.status == 412 || reply.status == 409 => {
                            outcome(EventResult::Failed, reply.status, op.remote.clone(), "the server's copy changed since")
                        }
                        Some(reply) => outcome(EventResult::Failed, reply.status, op.remote.clone(), "the server wouldn't take the change now"),
                        None => outcome(EventResult::Failed, 0, op.remote.clone(), "the server couldn't be reached"),
                    }
                }
                // (Not one card that can be written back without losing what the server has:
                // nothing is put, and the owner is told.)
                None => outcome(EventResult::Failed, 0, op.remote.clone(), "the server's card isn't one that can be changed from here"),
            }
        }
        ContactChangeKind::Delete => {
            let request = if op.force { endpoint.delete_over(&op.book, &op.remote) } else { endpoint.delete(&op.book, &op.remote, &op.etag) };
            let Ok(request) = request else {
                return outcome(EventResult::Failed, 0, op.remote.clone(), "the deletion can't be sent from here");
            };
            match send(request, trusted) {
                Some(reply) if (200..300).contains(&reply.status) || reply.status == 404 => {
                    outcome(EventResult::Gone, reply.status, op.remote.clone(), "")
                }
                Some(reply) if reply.status == 401 || reply.status == 403 => {
                    outcome(EventResult::Denied, reply.status, op.remote.clone(), "the server refused the deletion")
                }
                Some(reply) if reply.status == 412 || reply.status == 409 => {
                    outcome(EventResult::Failed, reply.status, op.remote.clone(), "the server's copy changed since")
                }
                Some(reply) => outcome(EventResult::Failed, reply.status, op.remote.clone(), "the server wouldn't take the deletion now"),
                None => outcome(EventResult::Failed, 0, op.remote.clone(), "the server couldn't be reached"),
            }
        }
    }
}

/// The card the server has, with the fields this change made written into it: `None` when either
/// document isn't one card (then nothing is put, rather than lose what the server has).
fn patched(raw: &str, op: &ContactOp) -> Option<String> {
    let theirs = mailcraft_vcard::parse(raw).into_iter().next()?;
    let ours = mailcraft_vcard::parse(&op.card).into_iter().next()?;
    let changed: std::collections::BTreeSet<ContactField> = ContactField::changed(&theirs, &ours);
    if changed.is_empty() {
        // (Nothing that is kept here differs: the server's card is kept as it is, rather than
        // written from a copy that doesn't hold what MailCraft doesn't read.)
        return Some(raw.to_string());
    }
    mailcraft_vcard::patch::patch(raw, &ours, &changed)
}

/// The name a new card is put under, without its `.vcf`: its UID when that is one that can be a
/// name, else the change's token.
fn resource_name(op: &ContactOp) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    let from_uid: String = op.uid.trim().chars().take(64).filter(|c| plain(*c)).collect();
    let name = if from_uid.is_empty() || from_uid.starts_with('.') { op.token.clone() } else { from_uid };
    let name: String = name.chars().take(64).filter(|c| plain(*c)).collect();
    if name.is_empty() || name.starts_with('.') { "contact".to_string() } else { name }
}

fn failed(op: &ContactOp, status: u16, why: &str) -> ContactOutcome {
    ContactOutcome { op: op.clone(), result: EventResult::Failed, remote: String::new(), status, error: why.to_string() }
}

fn endpoint(account: &Account, url: &str, password: &str) -> Option<Endpoint> {
    let trusted = TrustedOrigin::from_account_settings(url).ok()?;
    let secret = Secret::basic(account.dav_login(), password).ok()?;
    Some(Endpoint::new(trusted, Some(secret)))
}

/// The exchange over the real network: the system's certificates and the one the account trusts,
/// over TLS.
fn over_the_network(request: HttpRequest, trusted: &str) -> Option<Reply> {
    let fields: Vec<(&str, &str)> = request.fields().collect();
    let sent = crate::https::Request {
        method: request.method().as_str(),
        url: request.url(),
        fields: &fields,
        body: request.body(),
        max_response: request.max_response_bytes().min(MAX_ANSWER),
        trusted,
    };
    crate::https::send(&sent).ok().map(|reply| Reply { status: reply.status, body: reply.body })
}

/// Send one request and give its body, when the server answered successfully.
fn fetch(request: Result<HttpRequest, mailcraft_dav::request::RequestError>, trusted: &str, send: Send<'_>) -> Option<Vec<u8>> {
    let reply = send(request.ok()?, trusted)?;
    (200..300).contains(&reply.status).then_some(reply.body)
}

/// The server's card at `href`, as it is.
fn get(endpoint: &Endpoint, href: &str, trusted: &str, send: Send<'_>) -> Option<String> {
    let reply = send(endpoint.get(href).ok()?, trusted)?;
    if !(200..300).contains(&reply.status) {
        return None;
    }
    String::from_utf8(reply.body).ok()
}

#[cfg(test)]
#[path = "dav_contacts_tests.rs"]
mod tests;
