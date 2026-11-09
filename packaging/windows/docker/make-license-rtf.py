#!/usr/bin/env python3
# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Build the MSI's license page from LICENSE-MIT and LICENSE-APACHE.

The same RTF that packaging/windows/package.ps1 builds for WiX's WixUILicenseRtf: the licence
texts are joined into paragraphs (hard-wrapped lines become one line so the dialog wraps them
itself), the first paragraph of each licence is bold, and RTF's metacharacters and non-ASCII
characters are escaped. Reading the two files here keeps the installer's licence from drifting
from the repository's.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


def rtf_escape(text: str) -> str:
    """Escape one paragraph for an RTF document with \\ansicpg1252 and \\uc1."""
    out: list[str] = []
    for ch in text:
        code = ord(ch)
        if ch in "\\{}":
            out.append("\\" + ch)
        elif code > 127:
            # RTF's signed 16-bit \uN: values above 32767 are written as their negative.
            out.append(f"\\u{code - 65536 if code > 32767 else code}?")
        else:
            out.append(ch)
    return "".join(out)


def paragraphs(text: str) -> list[str]:
    """Split on blank lines and rejoin hard-wrapped lines, as package.ps1 does."""
    result: list[str] = []
    for block in re.split(r"\r?\n\s*\r?\n", text):
        joined = " ".join(line.strip() for line in block.splitlines() if line.strip())
        if joined:
            result.append(joined)
    return result


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(f"usage: {argv[0]} <repository root> <license.rtf>", file=sys.stderr)
        return 2

    root = Path(argv[1])
    destination = Path(argv[2])
    parts = ["\\b MailCraft\\b0"]
    parts.append(
        rtf_escape(
            "MailCraft is dual-licensed under the MIT License or the Apache License, Version 2.0, "
            "at your option. Both license texts follow. Bundled fonts and other third-party "
            "material keep their own open licenses, listed in NOTICE and ATTRIBUTION.md at "
            "https://github.com/UCS-Care-Group/mailcraft."
        )
    )

    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        first = True
        for paragraph in paragraphs((root / name).read_text(encoding="utf-8")):
            escaped = rtf_escape(paragraph)
            parts.append(f"\\b {escaped}\\b0" if first else escaped)
            first = False

    rtf = (
        "{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0\\fswiss\\fcharset0 Tahoma;}}"
        "\\viewkind4\\uc1\\pard\\sa120\\f0\\fs16 "
        + "\\par\r\n".join(parts)
        + "\\par\r\n}\r\n"
    )
    destination.write_text(rtf, encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
