#!/bin/sh
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Run as root in the image: hand Docker's root-owned volumes to the build user, then become that
# user. Files the build writes stay owned by builder inside the volumes; the artifacts leave
# through `docker cp`, so the host's UID never has to match the image's.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    exec "$@"
fi

for dir in /work /out /home/builder/.cargo /home/builder/.cache/cargo-xwin; do
    mkdir -p "$dir"
    chown builder:builder "$dir"
done

exec setpriv --reuid=1000 --regid=1000 --init-groups "$@"
