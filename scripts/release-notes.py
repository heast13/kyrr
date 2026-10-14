#!/usr/bin/env python3
"""Prints the release notes for one version: its section of CHANGELOG.md and
a few lines about the installer. When NEWS.md has a section for the version,
its summary (English and German, for Windows and for Linux) is added as
hidden comments; kyrr shows it under "What's new" next to the update note.

  python scripts/release-notes.py 0.2.0 [--signed] [--out notes.md]
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
POLICY = "https://github.com/heast13/kyrr#code-signing-policy"


def section(version: str) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    match = re.search(rf"^## {re.escape(version)}\b.*?\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if match is None:
        sys.exit(f"CHANGELOG.md has no section for {version}")
    return match.group(1).strip()


MARK = re.compile(r"^- \((Windows|Linux)\) ")


def for_system(body: str, system: str) -> str:
    """The items of a NEWS.md section for one system: a bullet marked
    "(Windows)" or "(Linux)" stays only there, without the mark, and the
    lines that continue it go with it. kyrr reads the same way (update.rs)."""
    out, keep = [], True
    for line in body.splitlines():
        if line.startswith("- "):
            mark = MARK.match(line)
            keep = mark is None or mark.group(1) == system
            if mark is not None:
                line = "- " + line[mark.end():]
        elif not line.strip():
            keep = True
        if keep:
            out.append(line)
    return "\n".join(out).strip()


def news(version: str) -> str:
    """The hidden blocks for kyrr's "What's new"; empty without a NEWS.md
    section for this version. Plain words, only what a user notices. Windows
    keeps the block names older versions read; Linux has its own."""
    path = ROOT / "NEWS.md"
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8")
    match = re.search(rf"^## {re.escape(version)}\b.*?\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if match is None:
        return ""
    english, _, german = match.group(1).partition("### Deutsch")
    blocks = ""
    for system, tag in (("Windows", "kyrr-news"), ("Linux", "kyrr-linux-news")):
        for lang, body in (("en", english), ("de", german)):
            body = for_system(body, system)
            if body:
                if "-->" in body:
                    sys.exit("NEWS.md: a summary must not contain '-->'")
                blocks += f"<!-- {tag}:{lang}\n{body}\n-->\n"
    return blocks


def main() -> None:
    args = sys.argv[1:]
    out = args[args.index("--out") + 1] if "--out" in args else None
    version = next(a for a in args if not a.startswith("--") and a != out)
    if "--signed" in args:
        signing = (
            "The installer and kyrr's programs are signed: free code signing provided by "
            f"SignPath.io, certificate by SignPath Foundation ([code signing policy]({POLICY}))."
        )
    else:
        signing = (
            "This installer is not code-signed yet, so Windows SmartScreen and your browser may "
            f"warn about it ([code signing policy]({POLICY}))."
        )
    notes = (
        f"{section(version)}\n\n"
        f"Windows 10 22H2 or Windows 11, 64-bit. {signing} Compare the installer with "
        f"`kyrr-setup-{version}.exe.sha256` first. See the README for what the installer changes.\n\n"
        f"Linux (64-bit Intel or AMD, glibc 2.39 or newer: Ubuntu 24.04, Fedora 40, Debian 13, Arch; "
        f"needs PipeWire; the noise filter needs AVX2): "
        f"make `kyrr-{version}-x86_64.AppImage` executable and run it. "
        f"`kyrr-{version}-linux-x86_64.tar.gz` holds the same files as an install tree for packagers. "
        f"Compare them with their `.sha256` files.\n"
    )
    blocks = news(version)
    if blocks:
        notes += f"\n{blocks}"
    if out:
        Path(out).write_text(notes, encoding="utf-8")
    else:
        sys.stdout.write(notes)


if __name__ == "__main__":
    main()
