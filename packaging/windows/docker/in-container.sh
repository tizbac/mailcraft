#!/usr/bin/env bash
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Runs inside the image made by packaging/windows/docker/Dockerfile. Builds the two Windows x64
# programs with cargo-xwin, then the MSI (and portable zip) with WiX under Wine. The host script
# packaging/windows/docker/build.sh mounts the source and collects what lands in /out.
#
# Only x86_64-pc-windows-msvc is built. Signing is optional: with WINDOWS_CERTIFICATE (a base64
# PFX) and WINDOWS_CERTIFICATE_PASSWORD set, the programs and MSI are signed with osslsigncode;
# otherwise they stay unsigned, as package.ps1 leaves them without signing secrets.
#
# Options:
#   --msi-only   build only the MSI, not the portable zip
#   --no-smoke   do not run mailcraft-cli --version under Wine
set -euo pipefail

# Wine needs a display even for a console program; run the whole build under a virtual one.
# xvfb-run is not exec'd: as PID 1 it can miss the readiness signal from Xvfb and wait forever.
if [ -z "${DISPLAY:-}" ] && command -v xvfb-run >/dev/null 2>&1; then
    xvfb-run -a bash "$0" "$@"
    exit $?
fi

msi_only=0
smoke=1
for arg in "$@"; do
    case "$arg" in
        --msi-only) msi_only=1 ;;
        --no-smoke) smoke=0 ;;
        -h | --help)
            sed -n '5,15p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

export HOME=/home/builder
export CARGO_HOME="$HOME/.cargo"
export CARGO_TARGET_DIR=/work/target
export WINEPREFIX="$HOME/.wine"
export WINEPATH='C:\users\builder\.dotnet\tools'
# The .NET SDK was unpacked here in the image; WiX's apphost finds its runtime through this.
export DOTNET_ROOT='C:\dotnet'
export MAILCRAFT_REQUIRE_WINRES=1
# Static CRT: no Visual C++ redistributable is needed on the target computer, as package.ps1 makes
# it. Scoped to the target so host build scripts and proc-macros are unaffected.
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"

# Version, build stamp and output folder, from the scripts the other packaging uses. DIST must be
# set before sourcing because /src is a read-only bind mount.
export DIST=/out
# shellcheck source=../../env.sh
. /src/packaging/env.sh

[[ -n "${VERSION:-}" ]] || {
    echo "error: could not read [workspace.package] version" >&2
    exit 1
}
# The image's working directory is the build user's home; cargo needs the workspace.
cd "$ROOT"
# MSI's ProductVersion is numeric (major.minor.build); a pre-release tag is dropped.
MSI_VERSION="${VERSION%%-*}"
ARCH=x64
TARGET=x86_64-pc-windows-msvc
BUILD="$CARGO_TARGET_DIR/$TARGET/release"
STAGE="$CARGO_TARGET_DIR/windows-package/stage"
PORTABLE="$CARGO_TARGET_DIR/windows-package/mailcraft-$VERSION-windows-$ARCH-portable"
MSI="$DIST/mailcraft-$VERSION-windows-$ARCH.msi"
ZIP="$DIST/mailcraft-$VERSION-windows-$ARCH-portable.zip"
WIX_EXE='C:\users\builder\.dotnet\tools\wix.exe'

winpath() { winepath -w "$1"; }
wix() { wine "$WIX_EXE" "$@"; }

# Check the PE headers before packaging, exactly as package.ps1 does: Machine must be x64, and
# subsystem 2 = GUI for the app, 3 = console for the CLI.
check_pe() {
    python3 - "$1" "$2" <<'PY'
import struct
import sys

path, subsystem = sys.argv[1], int(sys.argv[2])
with open(path, "rb") as f:
    data = f.read(4096)
if data[:2] != b"MZ":
    raise SystemExit(f"{path}: not a PE file (no MZ header)")
pe = struct.unpack_from("<I", data, 0x3C)[0]
if data[pe : pe + 4] != b"PE\0\0":
    raise SystemExit(f"{path}: not a PE file (no PE signature)")
machine = struct.unpack_from("<H", data, pe + 4)[0]
found = struct.unpack_from("<H", data, pe + 0x5C)[0]
if machine != 0x8664:
    raise SystemExit(f"{path}: machine 0x{machine:X}, expected 0x8664 (x64)")
if found != subsystem:
    raise SystemExit(f"{path}: PE subsystem {found}, expected {subsystem}")
print(f"ok {path.rsplit('/', 1)[-1]}: x64, PE subsystem {found}")
PY
}

SIGN_TMP=""
cleanup() {
    if [ -n "$SIGN_TMP" ]; then rm -rf "$SIGN_TMP"; fi
}
trap cleanup EXIT

sign_files() {
    if [ -z "${WINDOWS_CERTIFICATE:-}" ]; then
        return 0
    fi
    if [ -z "$SIGN_TMP" ]; then
        SIGN_TMP="$(mktemp -d)"
    fi
    local pfx="$SIGN_TMP/cert.pfx"
    local f signed
    printf '%s' "$WINDOWS_CERTIFICATE" | base64 -d >"$pfx"
    for f in "$@"; do
        signed="$f.signed"
        osslsigncode sign \
            -pkcs12 "$pfx" \
            -pass "${WINDOWS_CERTIFICATE_PASSWORD:-}" \
            -n "MailCraft" \
            -i "https://github.com/UCS-Care-Group/mailcraft" \
            -ts "${WINDOWS_TIMESTAMP_URL:-http://timestamp.digicert.com}" \
            -h sha256 \
            -in "$f" -out "$signed"
        mv "$signed" "$f"
        echo "signed $f"
    done
}

echo "MailCraft $VERSION for Windows $ARCH ($TARGET)"

# ---- programs ----------------------------------------------------------------------------------
echo "==> cargo xwin build ($TARGET)"
cargo xwin build --release --locked --target "$TARGET" -p mailcraft -p mailcraft-cli

check_pe "$BUILD/mailcraft.exe" 2
check_pe "$BUILD/mailcraft-cli.exe" 3

rm -rf "$STAGE" "$PORTABLE"
mkdir -p "$STAGE" "$DIST"
cp "$BUILD/mailcraft.exe" "$BUILD/mailcraft-cli.exe" "$STAGE/"

# ---- signing (optional) ------------------------------------------------------------------------
if [ -n "${WINDOWS_CERTIFICATE:-}" ]; then
    sign_files "$STAGE/mailcraft.exe" "$STAGE/mailcraft-cli.exe"
else
    warn "WINDOWS_CERTIFICATE is not set: the programs and MSI stay unsigned"
fi

# ---- MSI ---------------------------------------------------------------------------------------
# The welcome page's licence comes from the two licence files, as package.ps1 makes it.
python3 /src/packaging/windows/docker/make-license-rtf.py /src "$STAGE/license.rtf"

echo "==> wix build (MSZIP: Wine has no LZX cabinet support)"
wix build "$(winpath /src/packaging/windows/mailcraft.wxs)" \
    -arch "$ARCH" \
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext \
    -d "Version=$MSI_VERSION" \
    -d "CompressionLevel=mszip" \
    -d "BinDir=$(winpath "$STAGE")" \
    -d "IconPath=$(winpath /src/assets/app-icon/mailcraft.ico)" \
    -d "LicenseRtf=$(winpath "$STAGE/license.rtf")" \
    -o "$(winpath "$MSI")"
# wix writes debug symbols next to the MSI; they are not release files.
rm -f "${MSI%.msi}.wixpdb"

sign_files "$MSI"

# ---- portable zip ------------------------------------------------------------------------------
if [ "$msi_only" -eq 0 ]; then
    echo "==> portable zip"
    mkdir -p "$PORTABLE"
    cp "$STAGE/mailcraft.exe" "$STAGE/mailcraft-cli.exe" "$PORTABLE/"
    for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE; do
        if [ -f "/src/$f" ]; then cp "/src/$f" "$PORTABLE/"; fi
    done
    rm -f "$ZIP"
    (cd "$(dirname "$PORTABLE")" && zip -qr "$ZIP" "$(basename "$PORTABLE")")
fi

# ---- smoke test --------------------------------------------------------------------------------
if [ "$smoke" -eq 1 ]; then
    echo "==> mailcraft-cli --version (Wine)"
    wine "$(winpath "$STAGE/mailcraft-cli.exe")" --version
fi

echo
echo "built:"
ls -l "$MSI"
if [ "$msi_only" -eq 0 ]; then ls -l "$ZIP"; fi
