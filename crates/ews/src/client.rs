// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The client: an endpoint, a credential and a version, and the operations over any
//! [`HttpExchange`].
//!
//! **The version** asked for is Exchange 2013's unless told otherwise. A server older than that
//! (Exchange 2010 knows no version after `Exchange2010_SP2`) refuses the request whole with
//! `ErrorInvalidServerVersion` (#1349), before it does anything with it; the request is then made
//! once more, asking for [`Version::FALLBACK`], and the client asks for that from then on. That is
//! one step down and no more: a server that refuses that version too gets its answer as it was.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::error::{Error, ResponseCode};
use crate::folder::{DistinguishedFolder, FolderInfo, GetFolder};
use crate::http::{HttpExchange, HttpRequest, HttpResponse, Secret, check_endpoint};
use crate::reply::Reply;
use crate::soap::{Element, Version, envelope};

/// Speaks to one Exchange Web Services endpoint.
pub struct Client {
    endpoint: String,
    authorization: Secret,
    /// The version asked for ([`Version::index`]): lowered once when the server doesn't know it.
    version: AtomicU8,
    /// The last answer that couldn't be used, for the run to hand to the owner (never a log).
    bad: Mutex<Option<mailcraft_model::sync::BadAnswer>>,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            authorization: self.authorization.clone(),
            version: AtomicU8::new(self.version().index()),
            bad: Mutex::new(None),
        }
    }
}

impl std::fmt::Debug for Client {
    /// The version alone: no address and no credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("version", &self.version()).finish()
    }
}

impl Client {
    /// A client for the service at `endpoint` (`https://mail.example.org/EWS/Exchange.asmx`; plain
    /// `http` only to this machine), sending `authorization` as the `Authorization` field.
    pub fn new(endpoint: &str, authorization: Secret) -> Result<Self, Error> {
        check_endpoint(endpoint)?;
        Ok(Self { endpoint: endpoint.to_owned(), authorization, version: AtomicU8::new(Version::default().index()), bad: Mutex::new(None) })
    }

    /// Asks for the schema of `version`.
    #[must_use]
    pub fn with_version(self, version: Version) -> Self {
        self.version.store(version.index(), Ordering::Relaxed);
        self
    }

    /// The version the client asks for now: the one it was made with, or [`Version::FALLBACK`]
    /// once the server has said it doesn't know that one.
    pub fn version(&self) -> Version {
        Version::from_index(self.version.load(Ordering::Relaxed))
    }

    /// The answer that couldn't be used, once ([`Client::remember_bad`]): the whole body is kept
    /// (the HTTP client has already held it to the request's own limit), and the run says where
    /// it was written. Never a log line: it may hold mail.
    pub fn take_bad_answer(&self) -> Option<mailcraft_model::sync::BadAnswer> {
        self.bad.lock().ok().and_then(|mut bad| bad.take())
    }

    /// Keeps `response` as the answer a parse could not use. The operation is filled in by the
    /// caller, which knows what it asked.
    fn remember_bad(&self, response: &HttpResponse) {
        // (The whole answer, as it came: the HTTP client already held it to this request's own
        // limit, so it is finite; a body that is not text has its bad bytes replaced.)
        let body = String::from_utf8_lossy(response.body()).into_owned();
        if let Ok(mut bad) = self.bad.lock() {
            *bad = Some(mailcraft_model::sync::BadAnswer { operation: String::new(), status: response.status(), reason: String::new(), body });
        }
    }

    /// The server the endpoint is on, as it is written in the address: lower case, with its port
    /// when there is one (`mail.example.org:444`). Empty when the address names no host. It goes
    /// into the line that says which server could not be reached, which is why the address the
    /// owner typed is allowed to appear there (docs/mail-privacy-rules.md).
    pub fn host(&self) -> String {
        endpoint_host(&self.endpoint)
    }

    /// The request for `operation`, ready to send.
    pub fn request(&self, operation: &Element) -> Result<HttpRequest, Error> {
        self.request_in(self.version(), operation)
    }

    fn request_in(&self, version: Version, operation: &Element) -> Result<HttpRequest, Error> {
        Ok(HttpRequest::soap(&self.endpoint, &self.authorization, envelope(version, operation))?)
    }

    /// Sends `operation` and judges the answer to the point of its response messages.
    pub async fn call<E: HttpExchange>(&self, exchange: &E, operation: &Element) -> Result<Reply, Error> {
        self.negotiated(exchange, operation, false, false).await
    }

    /// [`Client::call`] for an operation that makes or sends something (`CreateItem`, `SendItem`,
    /// a move): the request is marked so that the program never sends it twice
    /// ([`HttpRequest::once`]), and an answer lost after it went out is [`TransportError::Lost`].
    pub async fn call_once<E: HttpExchange>(&self, exchange: &E, operation: &Element) -> Result<Reply, Error> {
        self.negotiated(exchange, operation, true, false).await
    }

    /// [`Client::call`] for the request that asks for one message whole: its answer may be as large
    /// as [`HttpRequest::for_whole_message`] says, since the message's text comes as base64 inside
    /// the envelope.
    pub async fn call_whole<E: HttpExchange>(&self, exchange: &E, operation: &Element) -> Result<Reply, Error> {
        self.negotiated(exchange, operation, false, true).await
    }

    /// Sends `operation` in the version asked for now, and once more in [`Version::FALLBACK`] when
    /// the server says it doesn't know that version. (Making it again is safe even for a request
    /// that is never sent twice: a server judges the version in the header before it reads the
    /// operation, so nothing of it was done.)
    async fn negotiated<E: HttpExchange>(&self, exchange: &E, operation: &Element, once: bool, whole: bool) -> Result<Reply, Error> {
        let asked = self.version();
        let request = |version| -> Result<HttpRequest, Error> {
            let r = self.request_in(version, operation)?;
            let r = if whole { r.for_whole_message() } else { r };
            Ok(if once { r.once() } else { r })
        };
        match self.send(exchange, request(asked)?).await {
            Err(Error::Server(ResponseCode::InvalidServerVersion)) if asked > Version::FALLBACK => {
                // (Another request of the same client may have lowered it already: either way it is the fallback.)
                if self.version.compare_exchange(asked.index(), Version::FALLBACK.index(), Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    log::info!("ews: the server doesn't know {}; asking for {} from now on", asked.as_str(), Version::FALLBACK.as_str());
                }
                self.send(exchange, request(Version::FALLBACK)?).await
            }
            other => other,
        }
    }

    async fn send<E: HttpExchange>(&self, exchange: &E, request: HttpRequest) -> Result<Reply, Error> {
        let response = exchange.exchange(&request).await?;
        // A bare 429 or 503 to a request that is never sent twice is no word of the service: it
        // may have been done. Only a SOAP `ErrorServerBusy`, which comes before anything is done,
        // is waited out and asked again.
        if !request.is_repeatable() && matches!(response.status(), 429 | 503) && !matches!(Reply::parse(response.body()), Err(Error::Busy { .. })) {
            self.remember_bad(&response);
            return Err(Error::Status(response.status()));
        }
        let reply = match Reply::judge(&response, request.max_response_bytes()) {
            Ok(reply) => reply,
            Err(error) => {
                // (Only an answer that came and could not be used is kept: a code or a busy
                // server says enough without a body.)
                if matches!(error, Error::Malformed | Error::Unexpected | Error::UnexpectedWhat(_) | Error::Status(_)) {
                    self.remember_bad(&response);
                }
                return Err(error);
            }
        };
        match reply.busy() {
            Some(busy) => Err(busy),
            None => Ok(reply),
        }
    }

    /// The folders `get` asks for, one result each.
    pub async fn get_folders<E: HttpExchange>(&self, exchange: &E, get: &GetFolder) -> Result<Vec<Result<FolderInfo, Error>>, Error> {
        get.answer(&self.call(exchange, &get.element()).await?)
    }

    /// The people of the organisation's address list for one text (#1262): `ResolveNames`, which
    /// reads and changes nothing.
    pub async fn resolve_names<E: HttpExchange>(
        &self,
        exchange: &E,
        lookup: &crate::resolve::ResolveNames,
    ) -> Result<crate::resolve::Resolved, Error> {
        lookup.answer(&self.call(exchange, &lookup.element()).await?)
    }

    /// Tests the account: the Inbox can be had. It is `GetFolder` of the Inbox by its
    /// distinguished name, and nothing is changed by it.
    pub async fn check<E: HttpExchange>(&self, exchange: &E) -> Result<FolderInfo, Error> {
        let get = GetFolder::new(&[DistinguishedFolder::Inbox]);
        self.get_folders(exchange, &get).await?.into_iter().next().ok_or(Error::Unexpected)?
    }
}

/// The most a host held for a line about it may be.
const MAX_HOST: usize = 100;

/// The host of an endpoint address, as written: `mail.example.org:444` of
/// `https://mail.example.org:444/EWS/Exchange.asmx`, lower case. Nothing but letters, digits,
/// hyphens, dots, colons and the brackets of an IPv6 address: an address [`check_endpoint`]
/// already read, held short and clean before it reaches a line the owner sees.
fn endpoint_host(endpoint: &str) -> String {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, rest)| rest);
    let authority: String = rest.split(['/', '?', '#']).next().unwrap_or("").chars().take(MAX_HOST).flat_map(|c| c.to_lowercase()).collect();
    if authority.is_empty() || !authority.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b':' | b'[' | b']')) {
        return String::new();
    }
    authority
}
