# This fork: what is ours on top of upstream

> **Last reviewed:** 2026-11-27 · **Last updated:** 2026-11-27 · **Change:** minor

MailCraft here is built from [UCS-Care-Group/mailcraft](https://github.com/UCS-Care-Group/mailcraft)
and published from [tizbac/mailcraft](https://github.com/tizbac/mailcraft). Upstream is the base; this
fork adds the work below on top. **A rebase onto `upstream/main` must never drop any of it.** When a
rebase is made, walk this list before pushing (the last section is the checklist).

## How the tree is kept

- `main` is a linear history: upstream's commits, then this fork's (rebased, never merged in the
  other direction). `git rebase upstream/main` is the workflow; `rerere` is on, and conflicts are
  expected in the Android app, `crates/mobile`, the catalogues and `docs/`.
- Upstream's newer implementation always wins where both sides have the same feature, and the fork's
  work is adapted on top of it (that is the rule the rebases follow).
- After a rebase: `cargo xtask i18n`, `cd apps/mailcraft-android && ./gradlew recordPaparazziDebug`
  when a phone screen changed, then `cargo xtask mobile` and `cargo xtask ci`.
- Local builds are unmasked from the store's copy: `mailcraft.applicationId` in
  `~/.gradle/gradle.properties` (see [android.md](android.md), "Building beside the store's copy").

## The fork's own work, by area

Each line names what it does and where its tests live; if a rebase touches those files, look here.

### Android app (`apps/mailcraft-android`)

| Ours | Where |
|---|---|
| The package can be named per build (`mailcraft.applicationId`), so a local build installs beside the store's copy | `app/build.gradle.kts` (`val appId`), `docs/android.md` |
| The phone's own Calendar and Contacts apps mirror every account that has a calendar server (CalDAV/CardDAV), whatever its kind — a Linchange or IMAP account with calendars, not only a calendars-only account | `SystemSync.kt`, `MainActivity.kt`, `crates/mobile/src/queries.rs` (`has_calendars`), `Inbox.kt` (`AccountItem.calendars`) |
| IMAP accounts added from Add Account, tried before adding, edited afterwards, and Send/Receive covering them (Graph round, then IMAP, then Exchange) | `AccountsController.kt`, `ImapAccountScreen.kt`, `MailCraftApplication.kt` (`syncAll`) |
| Exchange server accounts on the phone (EWS) | `Accounts.kt`, `crates/mobile/src` (EWS), `MailCraftApplication.kt` |
| Linchange sign-in through the server's page, with the server found from the domain's mail exchangers | `crates/mobile/src/accounts.rs` (`add_linchange_account`, `linchange_target`), `crates/mobile/src/linchange_tests.rs` |
| Swipe actions with an off switch, folded accounts in the drawer, and their preferences | `SwipePrefs.kt`, `FolderPrefs.kt`, `Shell.kt`, `MainActivity.kt` |
| detekt/Android Lint as errors kept green: IO dispatcher injected, `SyncService` abstract with its reason | `MailCraftApplication.kt`, `MailCraftSyncAdapter.kt` |

### Calendar and contacts servers (#209, on both platforms)

Upstream's D5 (calendars down), D6 (appointments up), D7 (a server's own password, and
File › Info › Calendars and Contacts with Find) are the base. This fork adds:

| Ours | Where |
|---|---|
| Calendar and contacts servers are **on** (the desktop too; `MAILCRAFT_DAV=0` turns them off) | `apps/mailcraft/src/main.rs` |
| A Linchange account is **added with the calendar and contact homes its server names**, found at sign-in from the server's own mail host with the application password the approval made (desktop and phone) | `crates/net/src/dav_find.rs` (`find_linchange_homes`), `apps/mailcraft/src/main.rs` (`linchange_sign_in`), `crates/ui-egui/src/{account_setup.rs,lib.rs}` (`LinchangeApproval`), `crates/mobile/src/accounts.rs` |
| The subscribed calendars section of File › Info lists each account's calendar server with Set Up…/Change…, opening the same Calendars and Contacts window | `crates/ui-egui/src/backstage.rs`, `dav_settings.rs` |
| **Conflicts never overwrite the server's copy**: a change made here on a version the server has changed since is held, for a calendar or a contact, and File › Info › Calendars and Contacts asks which copy to keep ("Keep Mine" / "Keep the Server's"); a setting keeps both instead (the server's read, what changed here made as a copy of its own) | `crates/engine/src/cmd/dav_sync.rs` (`take_updated`, `keep_both`, `sync.davConflicts`, `sync.davResolve`), `crates/engine/src/cmd/remote.rs` (`apply_book_contact_outcomes`, `keep_both_contact`), `Prefs::dav_conflicts` (`file.options` `davConflicts`), `crates/ui-egui/src/dav_settings.rs` |

Writes are conditional throughout (`If-Match` on a change or a deletion, `If-None-Match: *` on a
create), so a server's newer copy is never put over; the tests are in
`crates/engine/src/cmd/dav_sync_tests.rs` and `crates/net/src/dav_tests.rs`.

### Address books on a card server (#209)

Both ways on both platforms: the books a home lists and their cards are read
(`crates/net/src/dav_contacts.rs`; the desktop job in `crates/ui-egui/src/dav_job.rs`, the phone's
in `crates/mobile/src/dav.rs`), and a contact made, changed or deleted here is written back with
`If-Match` (`If-None-Match: *` on a create). A change is written into the card the server sent
(`mailcraft_vcard::patch` in `crates/vcard/src/patch.rs`), so a photo, keys and another program's
properties stay. A conflict is held and asked the same way as a calendar's
(`sync.davConflicts`, `sync.davResolve`), and the same setting keeps both.

### Exchange server accounts (upstream's #252, #962, #1274)

The desktop keeps an Exchange server account's calendars, address books and to-do lists in step
both ways: changes made here go up before the read (`crates/ews/src/directory_run.rs`), then the
calendars, books and lists come down (`crates/engine/src/cmd/ews_directory.rs`,
`crates/ui-egui/src/ews_calendar.rs`). This is upstream's, unmodified here: if a rebase changes it,
keep it.

### Interface texts

The catalogues (`crates/i18n/po/`) are generated from this tree with `cargo xtask i18n`; after a
rebase they are merged from upstream's and this fork's, then regenerated. Never resolve them by hand.

## Checklist after a rebase

Run these from the repository root; each should still find something.

```bash
grep -q 'mailcraft.applicationId' apps/mailcraft-android/app/build.gradle.kts
grep -q 'fun has_calendars' crates/mobile/src/queries.rs
grep -q 'has_calendars' apps/mailcraft-android/app/src/main/kotlin/work/mailcraft/android/SystemSync.kt
grep -q 'find_linchange_homes' crates/net/src/dav_find.rs crates/mobile/src/accounts.rs
grep -q 'LinchangeApproval' crates/ui-egui/src/account_setup.rs
grep -q 'sync.davConflicts' crates/engine/src/cmd/dav_sync.rs
grep -q 'dav_conflicts' crates/engine/src/session.rs
grep -q 'fn push' crates/net/src/dav_contacts.rs
grep -q 'pub mod patch' crates/vcard/src/lib.rs
grep -q 'internal suspend fun syncAll' apps/mailcraft-android/app/src/main/kotlin/work/mailcraft/android/MailCraftApplication.kt
grep -q 'Calendars and contacts from a server' crates/ui-egui/src/backstage.rs
```

Then the gates: `cargo xtask mobile` and `cargo xtask ci` (plus `recordPaparazziDebug` when a phone
screen changed), and a signed APK into `dist/release/` as [android.md](android.md) says.

## What this fork does not have yet

- Repeating appointments (series) cannot be made or changed on a calendar server yet (upstream's D6
  first part: single appointments).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-27 | minor | First written: what this fork carries on top of upstream, and how a rebase keeps it |
