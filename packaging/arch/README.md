# Arch Linux package

A source package (`PKGBUILD`) for Arch Linux and the AUR: it builds `mailcraft` and
`mailcraft-cli` from this repository with the system's Rust and installs them under `/usr`, with
the desktop entry, the MIME types, the AppStream metadata and the icons the other Linux packages
carry.

## Build and install

```sh
packaging/arch/package.sh            # dist/release/mailcraft-<version>-1-<arch>.pkg.tar.zst
packaging/arch/package.sh --install  # and pacman -U it
```

The script stages the checkout at `HEAD` as `mailcraft-<version>.tar.gz` (any commit, not only a
tag), copies `PKGBUILD` and `mailcraft.install` beside it, and runs `makepkg`. It needs
`base-devel` (makepkg, fakeroot), Rust 1.95 or later and git.

By hand, with the same three files in one directory:

```sh
git archive --format=tar.gz --prefix=mailcraft-0.15.70/ -o mailcraft-0.15.70.tar.gz HEAD
cp packaging/arch/PKGBUILD packaging/arch/mailcraft.install .
sed -i "s/^pkgver=.*/pkgver=0.15.70/" PKGBUILD
makepkg -si
```

If your Rust comes from `rustup` rather than the `rust` package (so pacman doesn't see rustup's
cargo as `rust`), add `--nodeps` to `package.sh`, or `makepkg --nodeps` by hand. With the
distro's `rust` package, no flag is needed.

`--no-check` skips the engine's hostile-parameters smoke test; `--keep` leaves makepkg's work
directory (`target/arch-package`) for inspection.

## What it installs

| Path | What |
|---|---|
| `/usr/bin/mailcraft` | the window |
| `/usr/bin/mailcraft-cli` | the headless importer, exporter and command runner (`mailcraft-cli mcp` for MCP) |
| `/usr/share/applications/io.github.billsecond.mailcraft.desktop` | the menu entry, offering MailCraft for `mailto:`, `.eml`, `.mbox`, `.ics` and `.vcf` |
| `/usr/share/mime/packages/io.github.billsecond.mailcraft.xml` | its own `.mailcraft` file type |
| `/usr/share/metainfo/io.github.billsecond.mailcraft.metainfo.xml` | AppStream metadata for software centres |
| `/usr/share/icons/hicolor/…` | the icon in every size |
| `/usr/share/doc/mailcraft/` | the readme and the licences |

Runtime libraries: `glibc`, `gcc-libs`, `libxkbcommon`, `libxkbcommon-x11` (the window can't start
on X11 without it), `libx11`, `libxcursor`, `libxi`, `libxrandr`, `libxcb`, `wayland`,
`vulkan-icd-loader` and `libglvnd`. Optional: `xdg-desktop-portal` (the system's own open and save
dialogs), `xdg-utils` (opening an attachment or a link elsewhere), `gnome-keyring` (remembering
sign-ins in the credential store), `vulkan-swrast` (software Vulkan).

The package installs nothing that runs at start and registers no service. Remembered sign-ins go
to the desktop's Secret Service over D-Bus, if one is running.

## The AUR

The `PKGBUILD` builds from a tarball this repository makes, so an AUR submission wants the tarball
and its checksum published first: use `$url/archive/refs/tags/v$pkgver.tar.gz` with its
`sha256sum`, and regenerate `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO`.
