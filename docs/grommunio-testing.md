# Grommunio, for testing an Exchange server account

> A throwaway Grommunio server on this computer, and how the Exchange Web Services (EWS) side of
> MailCraft is tried against it: contacts, calendars and to-dos made, changed, read back and
> deleted. Everything is made up; never point any of this at a real mailbox.

`scripts/grommunio-e2e.sh` does all of it: starts the server (pulling the images the first time),
makes the domain and one mailbox, reads the certificate's fingerprint, and runs
`grommunio_e2e_tests` in `apps/mailcraft`. `--down` removes the server and its volumes, `--logs`
follows its log. The first start takes a few minutes and about 4 GB of memory; later runs reuse
the volumes.

## What was tried, and what a run needs

| Fact | Value |
|---|---|
| Deployment | [`grommunio/gromox-container`](https://github.com/grommunio/gromox-container), images `container.grommunio.com/community/gromox-{core,archive,office}` (no build), MariaDB 11 mirrors |
| Server seen | Grommunio `2026.11` on openSUSE Leap 16, Gromox with EWS at `/EWS/Exchange.asmx` |
| Ports | only `127.0.0.1:28443` (HTTPS/EWS and web) and `127.0.0.1:29443` (admin API); the rest are not published, so nothing collides with a mail server on this computer |
| Name | `mail.grommunio.test`; its certificate is the container's own CA, `CN=mail.grommunio.test` |
| Certificate | the test trusts it by its SHA-256 fingerprint (`MAILCRAFT_E2E_PIN`), never by switching a check off |
| Sign-in | Basic over TLS (Grommunio offers no NTLM by default) |
| Users | `ada@grommunio.test` and `bob@grommunio.test`, made with `grommunio-admin user create --no-defaults` and `grommunio-admin passwd --password-stdin` |
| Domain | `grommunio.test`, made with `grommunio-admin domain create -u 100` |
| Name resolution | the test runs in a mount namespace with its own `/etc/hosts` and `hosts: files dns` in `nsswitch.conf`: systemd-resolved answers before `/etc/hosts` and would not know the name |
| Test | `apps/mailcraft/src/grommunio_e2e_tests.rs`, ignored by default, run with `-- --ignored` |

The test creates and then deletes a contact, an event and a to-do through the same
`EwsDirectoryRequest` / `EwsDirectoryResult` path the desktop's Send/Receive uses, and checks each
answer is `Done` and each item comes back from a fresh read (a changed contact with its company, a
moved event, a completed to-do) before deleting it. It then makes a contact **through the engine**
(`people.new` with the account's writable Exchange address book), sends the request
`sync.ewsDirectoryRequest` builds, and checks the contact is in a fresh read of the server —
the path that failed while Exchange contacts were wrongly taken for read-only.

## Doing it by hand

```sh
git clone --depth 1 https://github.com/grommunio/gromox-container.git /tmp/grommunio
# replace the three `build:` blocks with the community images, and in gromox-core publish only:
#   127.0.0.1:28443:8443   (HTTPS/EWS)
#   127.0.0.1:29443:9443   (admin)
cat >/tmp/grommunio/var.local.env <<'EOF'
FQDN=mail.grommunio.test
DOMAIN=grommunio.test
ADMIN_PASS=ChangeMe-e2e-1
CHAT_ADMIN_PASS=ChangeMe-e2e-1
FILES_ADMIN_PASS=ChangeMe-e2e-1
SSL_INSTALL_TYPE=2
EOF
docker compose -f /tmp/grommunio/docker-compose.yml --project-directory /tmp/grommunio up -d --no-build \
    gromox-db chat-db files-db office-db archive-db gromox-core
docker exec gromox-core grommunio-admin domain create -u 100 grommunio.test
docker exec gromox-core grommunio-admin user create --no-defaults ada@grommunio.test
printf 'ChangeMe-e2e-1\n' | docker exec -i gromox-core grommunio-admin passwd --password-stdin ada@grommunio.test
```

Then the certificate and the test (from the repository root):

```sh
pin=$(echo | openssl s_client -connect 127.0.0.1:28443 -servername mail.grommunio.test 2>/dev/null |
    openssl x509 -noout -fingerprint -sha256 | cut -d= -f2 | tr -d ':' | tr 'A-F' 'a-f')
printf '127.0.0.1 localhost\n127.0.0.1 mail.grommunio.test\n' >/tmp/hosts
printf 'hosts: files dns\n' >/tmp/nsswitch.conf
export MAILCRAFT_E2E_EWS_URL=https://mail.grommunio.test:28443/EWS/Exchange.asmx
export MAILCRAFT_E2E_USER=ada@grommunio.test
export MAILCRAFT_E2E_PASS=ChangeMe-e2e-1
export MAILCRAFT_E2E_PIN="$pin"
unshare -rm sh -c '
  mount --bind /tmp/hosts /etc/hosts
  mount --bind /tmp/nsswitch.conf /etc/nsswitch.conf
  cargo test -p mailcraft --bin mailcraft grommunio -- --ignored --nocapture
'
```

## Limits seen on this server

- Grommunio answers `FindItem` without a contact's physical addresses even when asked for them.
- It refuses `FindFolder` on a shared mailbox's top, so only that mailbox's default calendar and
  address book are found.
- EWS is offered with Basic only: NTLM cannot be tried here.
- A `webcal`/Internet calendar subscription and shared mailboxes beyond the default folders are
  not covered by this test.

## A flooded Inbox (#252)

Grommunio answers a folder it is catching up on with the read-state changes it already sent, at the
front of every page. Put twenty-odd thousand messages in the Inbox and mark them read, and one page
of `SyncFolderItems` carries that whole list: at `MaxChangesReturned` 512 the largest page is about
120,000 elements and 4 MB, whatever the page asked for. That is what the reader's ceiling of
elements in an answer is set for, and what the run does with such a page (keeps the newest of the
read states, takes the state the server gave, says once in the log what was left out) is covered by
`crates/ews/src/mail_tests.rs` against the stand-in server, which repeats the flags the same way.

The full table of what works and what does not, for mail as well as calendars and contacts, is in
[`exchange-servers.md`](exchange-servers.md#grommunio).
