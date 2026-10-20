#!/usr/bin/env python3
"""Stages the release files and builds the installer (Windows).

Run `python scripts/fetch-assets.py --windows` and, with SHERPA_ONNX_LIB_DIR
set to assets/sherpa-onnx/lib, `cargo build --release -p kyrr -p kyrr-apo`
first. The binaries are checked (scripts/check-binaries.py) before staging.

  --stage-only      only stage (CI signs the binaries in dist/stage next)
  --installer-only  only build the installer from dist/stage
  --checksum-only   only write the .sha256 (CI, after signing the installer)

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


# The Visual C++ runtime onnxruntime.dll needs (only on CPUs without AVX2/FMA);
# shipped next to it, app-local, as the Visual Studio license allows.
VC_RUNTIME = ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll"]


def vc_runtime_dir() -> Path:
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / (
        "Microsoft Visual Studio/Installer/vswhere.exe"
    )
    found = subprocess.run(
        [str(vswhere), "-latest", "-products", "*", "-find", "VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    if not found:
        sys.exit("Visual C++ redistributable files not found (Visual Studio Build Tools)")
    return Path(found[-1])


def stage() -> None:
    subprocess.run([sys.executable, str(ROOT / "scripts" / "check-binaries.py")], check=True)
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
    runtime = vc_runtime_dir()
    for f in VC_RUNTIME:
        shutil.copy2(runtime / f, STAGE)
    for f in ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "THIRD_PARTY.md"):
        shutil.copy2(ROOT / f, STAGE)
    for f in ("LICENSE-onnxruntime.txt", "ThirdPartyNotices-onnxruntime.txt"):
        shutil.copy2(ASSETS / f, licenses)
    # The font of the words in the dictation wave, built into kyrr.exe.
    shutil.copy2(ROOT / "crates" / "kyrr-common" / "fonts" / "OFL.txt", licenses / "OFL-Inter.txt")
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
    return DIST / f"kyrr-setup-{ver}.exe"


def write_checksum(exe: Path) -> None:
    digest = hashlib.sha256(exe.read_bytes()).hexdigest()
    # LF line ending so `sha256sum -c` works everywhere.
    (DIST / f"{exe.name}.sha256").write_text(f"{digest}  {exe.name}\n", encoding="utf-8", newline="\n")


def main() -> None:
    args = sys.argv[1:]
    if "--checksum-only" in args:
        write_checksum(DIST / f"kyrr-setup-{version()}.exe")
        return
    if "--installer-only" not in args:
        stage()
        if "--stage-only" in args:
            print(STAGE)
            return
    exe = build_installer(version())
    if "--installer-only" not in args:
        write_checksum(exe)
    print(exe)


if __name__ == "__main__":
    main()
