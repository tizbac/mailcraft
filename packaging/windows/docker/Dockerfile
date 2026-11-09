# syntax=docker/dockerfile:1.7
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# A Linux image that builds MailCraft's Windows x64 MSI without a Windows machine.
#
# It holds three things the build needs and Linux does not provide:
#   * Rust's x86_64-pc-windows-msvc target, through cargo-xwin (clang-cl + the Microsoft
#     CRT/Windows SDK that xwin downloads, and lld-link);
#   * Wine, to run the WiX v5 command-line tool (a Windows .NET program);
#   * the Windows .NET SDK inside that Wine prefix, which the WiX tool needs.
#
# Build it through packaging/windows/docker/build.sh, which also mounts the source and collects
# the MSI. Rust and WiX are the release workflow's versions. Wine is a development build: stable
# 11.0 cannot load .NET 10's framework assemblies. Override any of them with --build-arg.

ARG RUST_VERSION=1.98.1
FROM rust:${RUST_VERSION}-trixie

# Debian 13 (trixie) is the base of rust:1.98.1-trixie; the WineHQ repository follows the
# Debian suite name.
ARG DEBIAN_FRONTEND=noninteractive
# Wine 11.0 cannot load .NET 10's framework assemblies as IL-only images (Wine bug 59062); 11.x
# fixed it. The development build carries that fix (the stable 11.0.0.0 does not yet).
ARG WINE_PACKAGE=winehq-devel
ARG WINE_VERSION=11.21~trixie-1
# The Windows .NET SDK. WiX 5 targets net6.0 with rollForward=Major, so a newer runtime is fine.
ARG DOTNET_VERSION=10.0.401
# The version the release workflow pins as well (package.yml).
ARG WIX_VERSION=5.0.2
ARG CARGO_XWIN_VERSION=0.23.1

ENV WINEPREFIX=/home/builder/.wine \
    WINEARCH=win64 \
    WINEDEBUG=-all \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    DOTNET_GENERATE_ASPNET_CERTIFICATE=false \
    WINEPATH='C:\users\builder\.dotnet\tools' \
    CARGO_TERM_COLOR=always

# WineHQ's Wine (a 64-bit prefix with the 32-bit support its packages expect), the X server Wine
# needs even for console programs, the cross-compiler toolchain, and the archive/licence tools
# the packaging uses.
RUN set -eux; \
    dpkg --add-architecture i386; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      ca-certificates curl xz-utils unzip zip p7zip-full \
      xvfb xauth python3 \
      clang llvm cmake ninja-build \
      osslsigncode; \
    install -d -m 0755 /etc/apt/keyrings; \
    curl -fsSLo /etc/apt/keyrings/winehq-archive.key https://dl.winehq.org/wine-builds/winehq.key; \
    curl -fsSLo /etc/apt/sources.list.d/winehq.sources \
      https://dl.winehq.org/wine-builds/debian/dists/trixie/winehq-trixie.sources; \
    apt-get update; \
    apt-get install -y --no-install-recommends "${WINE_PACKAGE}=${WINE_VERSION}"; \
    apt-get clean; \
    rm -rf /var/lib/apt/lists/*

# The cross-compiler the workspace's Windows target needs. cargo-xwin supplies the Microsoft
# CRT and Windows SDK (downloaded on the first build and cached in the mounted volume).
RUN rustup target add x86_64-pc-windows-msvc \
 && cargo install cargo-xwin --version "${CARGO_XWIN_VERSION}" --locked

# The build runs as a normal user: Wine keeps a per-user prefix, and nothing should be written as
# root. The image carries the prefix; the volumes hold the caches.
RUN useradd --create-home --uid 1000 --shell /bin/bash builder \
 && install -d -o builder -g builder /work /out /home/builder/.cargo /home/builder/.cache/cargo-xwin

USER builder
ENV HOME=/home/builder
WORKDIR /home/builder

# A Windows .NET SDK, unpacked under Wine, and WiX's tool and its two extensions. `wix` is a
# Windows program, so everything here goes through Wine. The SDK's zip is used rather than its
# installer: the installer is a managed bootstrapper that needs Wine's Mono, while the SDK zip is
# plain files and the CoreCLR in it runs directly (the same runtime WiX runs on).
#
# Mono and Gecko are not needed; letting Wine look for them during wineboot can stall a headless
# prefix. The override is only for these two steps: .NET's assemblies need Wine's mscoree present,
# which is why it is unset before the SDK is run.
RUN set -eux; \
    WINEDLLOVERRIDES=mscoree,mshtml= wineboot --init; \
    WINEDLLOVERRIDES=mscoree,mshtml= wine reg add 'HKCU\Software\Wine\WineDbg' /v ShowCrashDialog /t REG_DWORD /d 0 /f; \
    unset WINEDLLOVERRIDES; \
    curl -fsSLo /tmp/dotnet-sdk.zip \
      "https://builds.dotnet.microsoft.com/dotnet/Sdk/${DOTNET_VERSION}/dotnet-sdk-${DOTNET_VERSION}-win-x64.zip"; \
    mkdir -p "$WINEPREFIX/drive_c/dotnet"; \
    unzip -q /tmp/dotnet-sdk.zip -d "$WINEPREFIX/drive_c/dotnet"; \
    rm -f /tmp/dotnet-sdk.zip; \
    export DOTNET_ROOT='C:\dotnet'; \
    wine 'C:\dotnet\dotnet.exe' --info; \
    wine 'C:\dotnet\dotnet.exe' tool install --global wix --version "${WIX_VERSION}"; \
    wine 'C:\users\builder\.dotnet\tools\wix.exe' --version; \
    wine 'C:\users\builder\.dotnet\tools\wix.exe' extension add -g "WixToolset.UI.wixext/${WIX_VERSION}"; \
    wine 'C:\users\builder\.dotnet\tools\wix.exe' extension add -g "WixToolset.Util.wixext/${WIX_VERSION}"; \
    wine 'C:\users\builder\.dotnet\tools\wix.exe' extension list -g

USER root

# Docker creates the cache volumes as root; the entrypoint gives them to `builder` and then runs
# the command as `builder`, so the host and the image need not share a UID.
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 0755 /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh", "bash", "/src/packaging/windows/docker/in-container.sh"]
CMD []
