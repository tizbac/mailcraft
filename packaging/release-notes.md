**MailCraft 0.10.1 in short**: Exchange calendars and contacts, Linchange DAV, and the fixes from testing them
- Exchange server accounts now keep calendars, contacts and to-dos in step, and open shared calendars and address books by address ([#252](https://github.com/GITHUB_REPOSITORY/issues/252)).
- Linchange accounts keep CalDAV calendars and CardDAV address books in step, with application-password sign-in on the server's page.
- Exchange reminders stay dismissed after a sync, and Exchange folders keep their place and can be deleted ([#252](https://github.com/GITHUB_REPOSITORY/issues/252)).
- The browser build is fixed and is now part of the wasm check.
- Rebased on current upstream `main`, including Google OAuth, PST import, conversations by thread and the recent fixes.
- And the fixes below.

**What's new**
- **Exchange calendars, address books and to-dos.** An Exchange server account reads its calendars, contacts and to-dos after its mail, sends changes up, and can open another person's calendar or address book when they have shared it. Shared calendars are listed under Shared Calendars and shared address books with the account's books. Tried against Grommunio; Exchange Server itself has been tried for mail and reading.
- **Linchange.** Add Account recognises a Linchange server from the address or its mail exchangers. Sign-in goes through the server's own page for an application password, so the account password and second factor stay there. Mail stays on IMAP and SMTP; calendars and address books are kept in step over CalDAV and CardDAV.
- **Shared and owner calendars.** CalDAV calendars carry their owner and rights. The account's own calendar is chosen by `DAV:owner`, not by being the first one listed, and an event with no calendar waits for a writable target.
- **CardDAV contacts.** A new contact can go into a chosen writable address book. Contacts from a server can be edited and deleted, with the change queued until the server takes it. Read-only books refuse changes, and a book that is no longer listed says so instead of saving the contact only here.
- **Exchange folders and reminders.** The Exchange folder list now asks where each folder lies, so folders stay under Inbox and deletes move them to Deleted Items before removing them for good. A reminder dismissed here stays dismissed when a sync reads the same event again.
- **Browser build.** The web app builds again, and `cargo xtask wasm` now checks the web app itself as well as the libraries.
- **Upstream.** This release includes the current upstream work: Google OAuth desktop sign-in, the PST reader and PST import, conversations by thread, the refused-delete and refused-undo fixes, Reply-To handling and more.

**Fixes**
- DAV requests accept an absolute address only on the account's own server; an approval page on another server is refused.
- CalDAV and CardDAV addresses are part of the mailbox confirmation and the credential-store key.
- Event and card addresses are matched by their resource path, so a server that names one resource by its whole address in one answer and by its path in the next does not list it twice.
- A CardDAV answer that settles an older change no longer clears a newer edit; a contact deleted while its creation was on its way queues a deletion for the address the server returned.
- Exchange folder deletion works: the first delete moves the folder to Deleted Items, the second removes it for good.
- A dismissed Exchange reminder no longer comes back after a sync.
- The browser build's file picker reads sizes and contents through rfd's web file handle.
- The wasm check now builds the actual web application, not only the crates under it.
