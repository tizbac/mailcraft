#!/usr/bin/env bash
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# One throwaway Grommunio server and the Exchange Web Services end-to-end test against it:
# contacts, calendars and to-dos made, changed, read back and deleted. Everything here is made up
# and runs on this computer; never point it at a real mailbox.
#
#   scripts/grommunio-e2e.sh            start (if needed) and run the test
#   scripts/grommunio-e2e.sh --down     stop the server and remove its volumes
#   scripts/grommunio-e2e.sh --logs     follow the server's log
#
# The facts behind it are in docs/grommunio-testing.md. Needs Docker with Compose v2.24 and
# about 4 GB of memory free; the first run pulls several hundred megabytes.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
work="${GROM_WORK:-/tmp/mailcraft-grommunio}"
compose="$work/repo/docker-compose.yml"
fqdn="mail.grommunio.test"
domain="grommunio.test"
user="ada@$domain"
password="${GROM_PASS:-ChangeMe-e2e-1}"
https_port="${GROM_HTTPS_PORT:-28443}"
admin_port="${GROM_ADMIN_PORT:-29443}"

down=0
logs=0
for arg in "$@"; do
    case "$arg" in
        --down) down=1 ;;
        --logs) logs=1 ;;
        -h | --help)
            sed -n '5,14p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

command -v docker >/dev/null || {
    echo "error: docker is not on PATH" >&2
    exit 1
}
docker info >/dev/null 2>&1 || {
    echo "error: cannot talk to the Docker daemon" >&2
    exit 1
}

if [ "$down" -eq 1 ]; then
    [ -f "$compose" ] || exit 0
    docker compose -f "$compose" --project-directory "$work/repo" down -v
    exit 0
fi
if [ "$logs" -eq 1 ]; then
    docker compose -f "$compose" --project-directory "$work/repo" logs -f gromox-core
    exit 0
fi

# The official deployment, with its containers pulled rather than built and only the HTTPS port
# published: it is one throwaway server on this computer, not a mail host.
if [ ! -f "$compose" ]; then
    mkdir -p "$work"
    git clone --depth 1 https://github.com/grommunio/gromox-container.git "$work/repo"
    python3 - "$work/repo/docker-compose.yml" "$compose" "$https_port" "$admin_port" <<'PY'
import re
import sys

source, target, https_port, admin_port = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
text = open(source).read()
for name in ("core", "archive", "office"):
    text = re.sub(
        rf"    build:\n      context: \./\n      dockerfile: \./gromox-{name}/Dockerfile",
        f"    image: container.grommunio.com/community/gromox-{name}:latest",
        text,
    )
start = text.index('    ports:\n      - "25:2525"')
end = text.index("    networks:", start)
text = text[:start] + (
    "    ports:\n"
    f'      - "127.0.0.1:{https_port}:8443"   # HTTPS / EWS and the web client\n'
    f'      - "127.0.0.1:{admin_port}:9443"   # the admin API\n'
) + text[end:]
open(target, "w").write(text)
PY
    cat >"$work/repo/var.local.env" <<EOF
FQDN=$fqdn
DOMAIN=$domain
ADMIN_PASS=$password
CHAT_ADMIN_PASS=$password
FILES_ADMIN_PASS=$password
SSL_INSTALL_TYPE=2
EOF
fi

echo "==> starting grommunio (first start takes a few minutes)"
docker compose -f "$compose" --project-directory "$work/repo" up -d --no-build gromox-db chat-db files-db office-db archive-db gromox-core
for _ in $(seq 1 60); do
    state="$(docker inspect -f '{{.State.Health.Status}}' gromox-core 2>/dev/null || echo starting)"
    [ "$state" = "healthy" ] && break
    sleep 10
done
[ "$(docker inspect -f '{{.State.Health.Status}}' gromox-core)" = "healthy" ] || {
    echo "error: gromox-core did not become healthy (scripts/grommunio-e2e.sh --logs)" >&2
    exit 1
}

# A domain and the one made-up mailbox the test uses, made once.
docker exec gromox-core grommunio-admin domain list 2>/dev/null | grep -q "grommunio.test" ||
    docker exec gromox-core grommunio-admin domain create -u 100 "$domain" >/dev/null
docker exec gromox-core grommunio-admin user list 2>/dev/null | grep -q "$user" ||
    docker exec gromox-core grommunio-admin user create --no-defaults "$user" >/dev/null
printf '%s\n' "$password" | docker exec -i gromox-core grommunio-admin passwd --password-stdin "$user" >/dev/null

# The certificate the server made for itself, trusted for this test by its fingerprint.
pin="$(echo | openssl s_client -connect "127.0.0.1:$https_port" -servername "$fqdn" 2>/dev/null |
    openssl x509 -noout -fingerprint -sha256 | cut -d= -f2 | tr -d ':' | tr 'A-F' 'a-f')"
[ -n "$pin" ] || {
    echo "error: could not read the server's certificate" >&2
    exit 1
}

# The name must resolve to the published port. systemd-resolved answers first and would not look
# at /etc/hosts, so the test runs in a mount namespace with a hosts file and an nsswitch of its
# own.
printf '127.0.0.1 localhost\n127.0.0.1 %s\n' "$fqdn" >"$work/hosts"
printf 'hosts: files dns\n' >"$work/nsswitch.conf"
echo "==> running the Exchange Web Services end-to-end test"
exec unshare -rm sh -c '
    mount --bind "$0/hosts" /etc/hosts
    mount --bind "$0/nsswitch.conf" /etc/nsswitch.conf
    cd "$1"
    MAILCRAFT_E2E_EWS_URL="https://'"$fqdn:$https_port"'/EWS/Exchange.asmx" \
    MAILCRAFT_E2E_USER="'"$user"'" \
    MAILCRAFT_E2E_PASS="'"$password"'" \
    MAILCRAFT_E2E_PIN="'"$pin"'" \
    cargo test -p mailcraft --bin mailcraft grommunio -- --ignored --nocapture
' "$work" "$root"
