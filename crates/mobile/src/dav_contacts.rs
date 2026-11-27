// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Address books read from a CardDAV server on the phone: the books a home lists, and the cards
//! in them, as the engine takes contacts in (`mailcraft_engine::cmd::remote::apply_book_contacts`).
//!
//! Nothing here is written anywhere, and nothing is sent but the sign-in of the card's own home.
//! A home or a book that can't be read makes the download incomplete, so the engine removes no
//! contact for not being in it.

use std::collections::HashMap;

use mailcraft_dav::answer::Multistatus;
use mailcraft_dav::href::TrustedOrigin;
use mailcraft_dav::http::{HttpRequest, Secret};
use mailcraft_dav::request::{Endpoint, Kind, MAX_MULTIGET};
use mailcraft_model::remote::{ContactsDownload, RemoteAddressBook, RemoteContact};
use mailcraft_model::{Account, Contact, DavProblem, Rights, origin_of};

/// Most cards kept from one read (a server that lists more is treated as too large).
const MOST_CONTACTS: usize = 20_000;
/// Longest answer read from one request (as the DAV requests themselves allow).
const MAX_ANSWER: usize = 8 * 1024 * 1024;

/// What a read of an account's address books came to: the cards, and why some may be missing.
pub(crate) struct ContactRead {
    pub download: ContactsDownload,
    pub problem: DavProblem,
}

/// Read every contact home of `account`. `passwords` holds the password for each origin its homes
/// are on, and only that origin's is ever sent there.
pub(crate) fn read(account: &Account, passwords: &HashMap<String, String>) -> ContactRead {
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
        let Ok(trusted) = TrustedOrigin::from_account_settings(home) else {
            out.complete = false;
            continue;
        };
        let user = account.dav_login();
        let Ok(secret) = Secret::basic(user, password) else {
            out.complete = false;
            continue;
        };
        let endpoint = Endpoint::new(trusted, Some(secret));
        let Some(listing) = fetch(endpoint.propfind_collections(home), &server.trusted) else {
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
            read_book(&endpoint, book.href.as_str(), &name, &server.trusted, &mut out);
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
fn read_book(endpoint: &Endpoint, book: &str, name: &str, trusted: &str, out: &mut ContactsDownload) {
    let Some(listing) = fetch(endpoint.propfind_members(book), trusted) else {
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
        let Some(answer) = fetch(Ok(request), trusted) else {
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
pub(crate) fn remote(contact: &Contact, id: &str, etag: &str, name: &str, book: &str) -> RemoteContact {
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

/// Send one request and give its body, when the server answered successfully.
fn fetch(request: Result<HttpRequest, mailcraft_dav::request::RequestError>, trusted: &str) -> Option<Vec<u8>> {
    let request = request.ok()?;
    let fields: Vec<(&str, &str)> = request.fields().collect();
    let sent = mailcraft_net::https::Request {
        method: request.method().as_str(),
        url: request.url(),
        fields: &fields,
        body: request.body(),
        max_response: request.max_response_bytes().min(MAX_ANSWER),
        trusted,
    };
    let answer = mailcraft_net::https::send(&sent).ok()?;
    (200..300).contains(&answer.status).then_some(answer.body)
}
