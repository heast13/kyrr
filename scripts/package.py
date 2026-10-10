#!/usr/bin/env python3
"""Stages the release files and builds the installer (Windows).

Run `python scripts/fetch-assets.py --windows` and
`cargo build --release -p kyrr -p kyrr-apo` first.

  dist/stage/                       everything the installer ships
  dist/kyrr-setup-<version>.exe     the installer (needs Inno Setup 6)
  dist/kyrr-setup-<version>.exe.sha256
"""

import hashlib
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
DIST = ROOT / "dist"
STAGE = DIST / "stage"
ISCC = [
    Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Inno Setup 6" / "ISCC.exe",
    Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Inno Setup 6" / "ISCC.exe",
]


def version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return re.search(r'^version = "(.+)"', text, re.M).group(1)


def stage() -> None:
    shutil.rmtree(STAGE, ignore_errors=True)
    licenses = STAGE / "licenses"
    licenses.mkdir(parents=True)
    release = ROOT / "target" / "release"
    for f in ("kyrr.exe", "kyrr_apo.dll"):
        shutil.copy2(release / f, STAGE)
    for f in (
        "onnxruntime.dll",
        "dpdfnet8_48khz_hr.onnx",
        "dpdfnet8_48khz_hr.weights.f32",
        "dpdfnet2_48khz_hr.onnx",
        "dpdfnet2_48khz_hr.weights.f32",
    ):
        shutil.copy2(ASSETS / f, STAGE)
    for f in ("LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY.md"):
        shutil.copy2(ROOT / f, STAGE)
    for f in ("LICENSE-onnxruntime.txt", "ThirdPartyNotices-onnxruntime.txt"):
        shutil.copy2(ASSETS / f, licenses)
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "collect-licenses.py"), str(licenses / "crates")],
        check=True,
    )


def build_installer(ver: str) -> Path:
    iscc = next((p for p in ISCC if p.is_file()), None)
    if iscc is None:
        sys.exit("Inno Setup 6 not found (ISCC.exe)")
    subprocess.run(
        [str(iscc), f"/DAppVersion={ver}", f"/DStage={STAGE}", str(ROOT / "installer" / "kyrr.iss")],
        check=True,
    )
    exe = DIST / f"kyrr-setup-{ver}.exe"
    digest = hashlib.sha256(exe.read_bytes()).hexdigest()
    # LF line ending so `sha256sum -c` works everywhere.
    (DIST / f"{exe.name}.sha256").write_text(f"{digest}  {exe.name}\n", encoding="utf-8", newline="\n")
    return exe


def main() -> None:
    stage()
    if "--stage-only" in sys.argv[1:]:
        print(STAGE)
        return
    print(build_installer(version()))


if __name__ == "__main__":
    main()
