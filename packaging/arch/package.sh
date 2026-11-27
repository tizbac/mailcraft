#!/usr/bin/env bash
# Build MailCraft's Arch Linux package (a source PKGBUILD) with makepkg.
#
#   packaging/arch/package.sh [--install] [--no-check] [--nodeps] [--keep]
#
# It stages the repository at HEAD as mailcraft-<version>.tar.gz (the prefix makepkg extracts),
# puts the PKGBUILD and its install script beside it, and builds
#
#   dist/release/mailcraft-<version>-1-<arch>.pkg.tar.zst
#
# --install runs `pacman -U` on the result (the owner's own machine, and the owner's yes).
# --no-check skips the engine smoke test (makepkg --nocheck).
# --nodeps builds without checking dependencies, for a machine whose Rust is rustup's rather
#   than the `rust` package (pacman doesn't see rustup's cargo as `rust`).
# --keep leaves makepkg's work directory (CARGO_TARGET_DIR/arch-package) for inspection.
#
# Needs: makepkg and fakeroot (base-devel), Rust >= 1.95 and git, and the runtime libraries the
# window and GPU load (libxkbcommon, libxkbcommon-x11, Vulkan or EGL); see the PKGBUILD.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/arch"
ARCH="${CROSS_ARCH:-$(uname -m)}"

INSTALL=0
NOCHECK=0
NODEPS=0
KEEP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --install) INSTALL=1; shift ;;
    --no-check) NOCHECK=1; shift ;;
    --nodeps) NODEPS=1; shift ;;
    --keep) KEEP=1; shift ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

command -v makepkg >/dev/null || { echo "error: makepkg not found (pacman -S base-devel)" >&2; exit 1; }
command -v git >/dev/null || { echo "error: git not found" >&2; exit 1; }

WORK="$CARGO_TARGET_DIR/arch-package"
rm -rf "$WORK"
mkdir -p "$WORK" "$DIST"

echo "==> MailCraft $VERSION for Arch Linux $ARCH"
# The source tarball: the checkout exactly as it is (a version ahead of the last tag is fine).
git -C "$ROOT" archive --format=tar.gz --prefix="mailcraft-$VERSION/" -o "$WORK/mailcraft-$VERSION.tar.gz" HEAD
cp "$HERE/PKGBUILD" "$HERE/mailcraft.install" "$WORK/"
# The committed PKGBUILD carries the version it was last built for; the package follows Cargo.toml.
sed -i "s/^pkgver=.*/pkgver=$VERSION/" "$WORK/PKGBUILD"

args=(--noconfirm --force --clean)
[[ $NOCHECK == 1 ]] && args+=(--nocheck)
[[ $NODEPS == 1 ]] && args+=(--nodeps)
( cd "$WORK" && MAILCRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD)" PKGDEST="$DIST" makepkg "${args[@]}" )

PKG="$(ls -1t "$DIST"/mailcraft-"$VERSION"-1-"$ARCH".pkg.tar.* 2>/dev/null | head -1)"
[[ -n "$PKG" ]] || { echo "error: no package was written to $DIST" >&2; exit 1; }
echo "wrote $PKG"

if [[ $INSTALL == 1 ]]; then
  echo "==> pacman -U $PKG"
  sudo pacman -U --noconfirm "$PKG"
fi

if [[ $KEEP == 0 ]]; then rm -rf "$WORK"; else echo "kept $WORK"; fi
