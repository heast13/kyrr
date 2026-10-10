#!/usr/bin/env python3
"""Prints the release notes for one version: its section of CHANGELOG.md and
a few lines about the installer.

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
        f"`kyrr-setup-{version}.exe.sha256` first. See the README for what the installer changes.\n"
    )
    if out:
        Path(out).write_text(notes, encoding="utf-8")
    else:
        sys.stdout.write(notes)


if __name__ == "__main__":
    main()
