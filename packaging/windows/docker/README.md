# Windows MSI on Linux, with Docker

This folder builds the Windows **Intel/AMD x64** release of MailCraft on a Linux machine:
`mailcraft.exe` and `mailcraft-cli.exe` for `x86_64-pc-windows-msvc`, then the WiX v5 MSI and the
portable zip, exactly named as the release assets are:

```text
dist/release/mailcraft-<version>-windows-x64.msi
dist/release/mailcraft-<version>-windows-x64-portable.zip
```

It is a local convenience, not the release pipeline: releases are still built and signed on
Windows by `.github/workflows/package.yml` with `packaging/windows/package.ps1`.

## Quick start

```sh
packaging/windows/docker/build.sh
```

The first run builds the image (Rust, cargo-xwin, Wine and WiX; several minutes and about 8 GB),
then downloads the Microsoft CRT/Windows SDK and compiles MailCraft. Later runs reuse the image
layers and the cached `target` folder.

```sh
packaging/windows/docker/build.sh --output /tmp/mailcraft-release
packaging/windows/docker/build.sh --msi-only
packaging/windows/docker/build.sh --clean          # full rebuild, keeps the image
packaging/windows/docker/build.sh --no-cache       # rebuild the image layers too
packaging/windows/docker/build.sh --keep           # keep the container after a failure
```

Needs Docker and network access (for the image, the Microsoft CRT/Windows SDK that cargo-xwin
downloads, and the NuGet packages WiX installs). Nothing else is installed on the host; the
repository is mounted read-only. The image is `linux/amd64` (Wine and the Windows .NET SDK in it
are x64 programs), so an x64 Linux host needs nothing extra; an ARM host needs Docker's QEMU
support and will be slower. `MAILCRAFT_DOCKER_PLATFORM` overrides the platform.

## What is in the image

| Piece | Why |
| --- | --- |
| `rust:1.98.1-trixie` plus `x86_64-pc-windows-msvc` | the same Rust version the release workflow pins |
| `cargo-xwin` | cross-compiles the MSVC target on Linux: clang-cl, `lld-link`, and the Microsoft CRT/Windows SDK it downloads |
| WineHQ's Wine 11.21 (development build) | runs the WiX command-line tool, which is a Windows .NET program; stable 11.0 predates the .NET 10 loader fix (Wine bug 59062) |
| the Windows .NET SDK, unpacked under Wine | the runtime WiX needs |
| WiX v5.0.2 with `WixToolset.UI` and `WixToolset.Util` | the MSI, its welcome/licence dialog and the "Launch MailCraft" finish page |

The Rust and WiX versions are the release workflow's; .NET is the SDK the WiX tool runs on. They
can be overridden with environment variables, e.g.
`MAILCRAFT_WINE_VERSION=... packaging/windows/docker/build.sh` (see the top of `build.sh`).

The MSI embeds the license, built from `LICENSE-MIT` and `LICENSE-APACHE` by
`make-license-rtf.py` exactly as `package.ps1` builds it, so it cannot drift from the repository.
The two programs are checked for the x64 machine type and the expected PE subsystem (GUI for the
app, console for the CLI) before packaging, and `mailcraft-cli --version` is run under Wine as a
smoke test.

## Signing

A local build is unsigned, like a build with no signing secrets on Windows. To sign what this
script makes, export a base64 `.pfx` and its password:

```sh
export WINDOWS_CERTIFICATE="$(base64 -w0 my-cert.pfx)"
export WINDOWS_CERTIFICATE_PASSWORD='...'
# optional: export WINDOWS_TIMESTAMP_URL='http://timestamp.digicert.com'
packaging/windows/docker/build.sh
```

`osslsigncode` signs both programs and the MSI. The secrets are passed to the container through
its environment and are never written to the image, the source tree or a log. Azure Artifact
Signing (the release workflow's other path, `sign.ps1`) uses signtool and Windows identity and is
not available here; sign on Windows with the release workflow when that is the certificate in use.

## Notes and limits

- Only Intel/AMD x64. There is no x86, no arm64 and no macOS/Linux output here.
- The MSI is built with MSZIP-compressed cabinets rather than the release build's `high` (LZX):
  Wine's cabinet support has no LZX (Wine bug 56558). It installs the same files and is only a
  little larger. `mailcraft.wxs` keeps `high` as its default, so the Windows-built release is
  unchanged.
- The output is meant for testing and for a Linux-driven build of the same sources; an MSI that
  ships should remain the signed Windows build from the release workflow.
- The image's version of the Microsoft CRT/Windows SDK comes from cargo-xwin's default xwin
  payload. If Microsoft changes it, `--clean` refreshes the download.
- The build caches are Docker volumes named `mailcraft-win64-target`, `mailcraft-win64-cargo` and
  `mailcraft-win64-xwin`; `--clean` drops them. `docker volume ls` and `docker system df` show
  what they take.
- The repository is mounted read-only at `/src`; the build writes only under `/work` and `/out`
  inside the container. Files come back through `docker cp`, so the host UID does not have to
  match anything in the image.
