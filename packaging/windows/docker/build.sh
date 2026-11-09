#!/usr/bin/env bash
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Build MailCraft's Windows x64 MSI on Linux with Docker.
#
#   packaging/windows/docker/build.sh
#
# It builds (or reuses) the image in this folder, mounts the repository read-only, builds the two
# Windows programs for x86_64-pc-windows-msvc, and runs WiX v5 under Wine to make
#
#   dist/release/mailcraft-<version>-windows-x64.msi
#   dist/release/mailcraft-<version>-windows-x64-portable.zip
#
# Only Intel/AMD x64 Windows is produced; there is no x86 or arm64. The image and the caches live
# in Docker's volumes, so the host needs no Rust Windows target, Wine, WiX or .NET.
#
# Options:
#   --output DIR    where the release files go (default: dist/release)
#   --msi-only      build the MSI but not the portable zip
#   --no-smoke      do not run mailcraft-cli --version under Wine
#   --no-cache      rebuild the image without Docker's layer cache
#   --clean         discard the target, Cargo and xwin caches before building
#   --keep          keep the container (named on failure) for inspection
#   -h, --help      show this help
#
# Signing (optional): export WINDOWS_CERTIFICATE (the .pfx in base64) and
# WINDOWS_CERTIFICATE_PASSWORD, optionally WINDOWS_TIMESTAMP_URL; the programs and the MSI are
# then signed with osslsigncode. Without them the build is unsigned, as package.ps1 leaves a
# build with no signing secrets. Azure Artifact Signing and signtool stay on Windows/CI.
#
# Environment overrides for the tool versions (they default to the versions this script pins):
#   MAILCRAFT_RUST_VERSION, MAILCRAFT_WINE_PACKAGE, MAILCRAFT_WINE_VERSION,
#   MAILCRAFT_DOTNET_VERSION, MAILCRAFT_WIX_VERSION, MAILCRAFT_CARGO_XWIN_VERSION, MAILCRAFT_DOCKER_IMAGE,
#   MAILCRAFT_DOCKER_VOLUME_PREFIX, MAILCRAFT_DOCKER_PLATFORM (default linux/amd64).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"

image="${MAILCRAFT_DOCKER_IMAGE:-mailcraft-windows-x64-builder}"
rust_version="${MAILCRAFT_RUST_VERSION:-1.98.1}"
wine_package="${MAILCRAFT_WINE_PACKAGE:-winehq-devel}"
wine_version="${MAILCRAFT_WINE_VERSION:-11.21~trixie-1}"
dotnet_version="${MAILCRAFT_DOTNET_VERSION:-10.0.401}"
wix_version="${MAILCRAFT_WIX_VERSION:-5.0.2}"
cargo_xwin_version="${MAILCRAFT_CARGO_XWIN_VERSION:-0.23.1}"
volume_prefix="${MAILCRAFT_DOCKER_VOLUME_PREFIX:-mailcraft-win64}"
# The Wine prefix and the Windows .NET SDK in it are x64 programs. An x64 host needs no
# emulation; an ARM host can run this image through Docker's binfmt/QEMU support.
platform="${MAILCRAFT_DOCKER_PLATFORM:-linux/amd64}"
output="$root/dist/release"
msi_only=0
smoke=1
no_cache=0
clean=0
keep=0

usage() {
    sed -n '5,35p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --output)
            [ $# -ge 2 ] || {
                echo "error: --output needs a folder" >&2
                exit 2
            }
            output="$2"
            shift 2
            ;;
        --output=*)
            output="${1#--output=}"
            shift
            ;;
        --msi-only)
            msi_only=1
            shift
            ;;
        --no-smoke)
            smoke=0
            shift
            ;;
        --no-cache)
            no_cache=1
            shift
            ;;
        --clean)
            clean=1
            shift
            ;;
        --keep)
            keep=1
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

command -v docker >/dev/null 2>&1 || {
    echo "error: docker is not on PATH" >&2
    exit 1
}
docker info >/dev/null 2>&1 || {
    echo "error: cannot talk to the Docker daemon (is it running, and is this user in the docker group?)" >&2
    exit 1
}

# The build stamps the commit and date it was made from, as package.ps1 does. Export them because
# `docker create -e NAME` reads the value from this process's environment.
if [ -z "${MAILCRAFT_BUILD_SHA:-}" ]; then
    MAILCRAFT_BUILD_SHA="$(git -C "$root" rev-parse HEAD 2>/dev/null || true)"
fi
export MAILCRAFT_BUILD_SHA
export MAILCRAFT_BUILD_DATE="${MAILCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"

echo "==> docker build $image"
build_args=(
    --build-arg "RUST_VERSION=$rust_version"
    --build-arg "WINE_PACKAGE=$wine_package"
    --build-arg "WINE_VERSION=$wine_version"
    --build-arg "DOTNET_VERSION=$dotnet_version"
    --build-arg "WIX_VERSION=$wix_version"
    --build-arg "CARGO_XWIN_VERSION=$cargo_xwin_version"
)
if [ "$no_cache" -eq 1 ]; then build_args+=(--no-cache); fi
docker build --platform "$platform" "${build_args[@]}" -f "$here/Dockerfile" -t "$image" "$here"

target_volume="${volume_prefix}-target"
cargo_volume="${volume_prefix}-cargo"
xwin_volume="${volume_prefix}-xwin"
if [ "$clean" -eq 1 ]; then
    echo "==> discarding the build caches"
    docker volume rm -f "$target_volume" "$cargo_volume" "$xwin_volume" >/dev/null 2>&1 || true
fi
for volume in "$target_volume" "$cargo_volume" "$xwin_volume"; do
    docker volume create "$volume" >/dev/null
done

# Pass the values the build reads from the environment, but only the ones that are set. This
# keeps a normal build free of any signing material.
env_args=()
for name in \
    MAILCRAFT_VERSION MAILCRAFT_BUILD_SHA MAILCRAFT_BUILD_DATE \
    MAILCRAFT_GOOGLE_CLIENT_ID MAILCRAFT_GOOGLE_CLIENT_SECRET MAILCRAFT_MS_CLIENT_ID \
    WINDOWS_CERTIFICATE WINDOWS_CERTIFICATE_PASSWORD WINDOWS_TIMESTAMP_URL; do
    if [ -n "${!name:-}" ]; then env_args+=(-e "$name"); fi
done

pass=()
if [ "$msi_only" -eq 1 ]; then pass+=(--msi-only); fi
if [ "$smoke" -eq 0 ]; then pass+=(--no-smoke); fi

container="mailcraft-win64-build-$$"
docker rm -f "$container" >/dev/null 2>&1 || true

echo "==> docker run (source read-only; caches in volumes)"
cid="$(docker create --platform "$platform" --init --name "$container" \
    -v "$root:/src:ro" \
    -v "$target_volume:/work" \
    -v "$cargo_volume:/home/builder/.cargo" \
    -v "$xwin_volume:/home/builder/.cache/cargo-xwin" \
    "${env_args[@]}" \
    "$image" "${pass[@]}")"

status=0
docker start -a "$cid" || status=$?

if [ "$status" -eq 0 ]; then
    mkdir -p "$output"
    docker cp "$cid:/out/." "$output/"
    echo
    echo "release files in $output:"
    ls -l "$output"/mailcraft-*-windows-x64*
else
    echo "build failed (exit $status)" >&2
fi

if [ "$keep" -eq 1 ]; then
    echo "container kept for inspection: $cid"
    echo "  docker logs $cid"
    echo "  docker cp $cid:/out/. ./out"
else
    docker rm -f "$cid" >/dev/null
fi
exit "$status"
