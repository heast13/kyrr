#!/usr/bin/env python3
"""Downloads the runtime assets into assets/ and verifies their SHA256.

- DPDFNet models and native-engine weights (Ceva, Apache-2.0) from the
  HushMic v0.10.2 release
- ONNX Runtime 1.27.0 for Windows (Microsoft, MIT), the fallback engine on
  CPUs without AVX2/FMA; only with --windows
- sherpa-onnx 1.13.8 static libraries *without* text-to-speech (Apache-2.0),
  so kyrr links no GPL code (the default build bundles espeak-ng);
  only with --windows or --linux. Build with SHERPA_ONNX_LIB_DIR pointing
  at assets/sherpa-onnx/lib (CI and package.py check it)
- two short speech clips from the sherpa-onnx test assets for the dictation
  tests; only with --test

Every file comes from kyrr's own mirror (the build-1 release of
heast13/kyrr-assets) first and from its original home only when that
fails, so a build does not depend on others keeping their releases.

Standard library only, so it runs the same on Linux and Windows.
"""

import hashlib
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
MIRROR = "https://github.com/heast13/kyrr-assets/releases/download/build-1"
HUSHMIC = "https://github.com/Fovty/HushMic/releases/download/v0.10.2"

MODELS = [
    ("dpdfnet8_48khz_hr.onnx", "7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631"),
    ("dpdfnet2_48khz_hr.onnx", "7f0575a5cec0ba4ffd8f8bd657e06d007e4ccdd955d76faab922b9d3291dc14b"),
    ("dpdfnet8_48khz_hr.weights.f32", "5a5bb67a8619090c54dc2793536fe813fe38123b236d445f3433aafe3075529d"),
    ("dpdfnet2_48khz_hr.weights.f32", "cb7248b8fccbff7b32f254ec2ed0061e604514b2ccd98d997cd081a876424e7b"),
]
SHERPA = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"
TEST_AUDIO = [
    ("de.wav", "36d3c4845b9808a1656a2a2e92d884590e2db94389e6fe559643291ae0cd3710"),
    ("en.wav", "148b936b43ce7c546a866e64da059f0458aee2d65e617f16e9d94f06e8d99ed6"),
]
SHERPA_LIB = (
    "sherpa-onnx-v1.13.8-win-x64-static-MT-Release-no-tts-lib.tar.bz2",
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/"
    "sherpa-onnx-v1.13.8-win-x64-static-MT-Release-no-tts-lib.tar.bz2",
    "f0aa074ddc39553b30208b53e84c12275d09702911c9faa8d34fa932428e4641",
)
SHERPA_LIB_LINUX = (
    "sherpa-onnx-v1.13.8-linux-x64-static-no-tts-lib.tar.bz2",
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/"
    "sherpa-onnx-v1.13.8-linux-x64-static-no-tts-lib.tar.bz2",
    "36f2ebd0b9aa09248ff6461a30dcb7edfe7eecf2deb67dfb0ceb3aaf4906051f",
)
# sherpa-onnx-sys always links these text-to-speech libraries; without
# text-to-speech nothing references them, so empty stand-ins do.
SHERPA_STUBS = ["espeak-ng", "piper_phonemize", "ucd"]
ORT_ZIP = (
    "onnxruntime-win-x64-1.27.0.zip",
    "https://github.com/microsoft/onnxruntime/releases/download/v1.27.0/onnxruntime-win-x64-1.27.0.zip",
    "c5c81710938e68079ff1a192b04897faabe4b43830d48f39f27ecd4e16138bfc",
)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def fetch(name: str, url: str, digest: str, folder: Path = ASSETS) -> Path:
    """`name` from kyrr's mirror, else from `url`, checked against `digest`."""
    folder.mkdir(exist_ok=True)
    dest = folder / name
    if dest.is_file() and sha256(dest) == digest:
        print(f"OK  {name}")
        return dest
    tmp = dest.with_suffix(dest.suffix + ".part")
    for source in (f"{MIRROR}/{name}", url):
        print(f"Downloading {name} from {source} ...")
        try:
            with urllib.request.urlopen(source) as r, tmp.open("wb") as f:
                shutil.copyfileobj(r, f)
        except OSError as e:
            print(f"    failed: {e}")
            continue
        if sha256(tmp) == digest:
            tmp.replace(dest)
            print(f"OK  {name}")
            return dest
        print("    checksum mismatch")
    tmp.unlink(missing_ok=True)
    sys.exit(f"could not fetch {name}")


def extract_ort(zip_path: Path) -> None:
    # Only onnxruntime.dll is needed; its license and notices ship with it.
    wanted = {
        "lib/onnxruntime.dll": "onnxruntime.dll",
        "LICENSE": "LICENSE-onnxruntime.txt",
        "ThirdPartyNotices.txt": "ThirdPartyNotices-onnxruntime.txt",
    }
    with zipfile.ZipFile(zip_path) as z:
        for entry in z.namelist():
            for suffix, target in wanted.items():
                if entry.endswith("/" + suffix):
                    (ASSETS / target).write_bytes(z.read(entry))
                    wanted[suffix] = None
    missing = [k for k, v in wanted.items() if v is not None]
    if missing:
        sys.exit(f"missing in ONNX Runtime archive: {missing}")
    print("OK  onnxruntime.dll")


def msvc_tool(name: str) -> Path:
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / (
        "Microsoft Visual Studio/Installer/vswhere.exe"
    )
    found = subprocess.run(
        [str(vswhere), "-latest", "-products", "*", "-find", f"VC/Tools/MSVC/**/bin/Hostx64/x64/{name}"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    if not found:
        sys.exit(f"MSVC {name} not found (Visual Studio Build Tools with C++ needed)")
    return Path(found[-1])


def extract_sherpa(archive: Path) -> None:
    windows = archive.name.startswith("sherpa-onnx-v1.13.8-win")
    suffix = ".lib" if windows else ".a"
    lib = ASSETS / "sherpa-onnx" / "lib"
    shutil.rmtree(lib.parent, ignore_errors=True)
    lib.mkdir(parents=True)
    with tarfile.open(archive) as t:
        for member in t.getmembers():
            if member.isfile() and member.name.endswith(suffix):
                member.name = Path(member.name).name
                t.extract(member, lib, filter="data")
    for name in SHERPA_STUBS:
        if windows:
            make_stub_lib(lib / f"{name}.lib")
        else:
            # An archive with no members.
            (lib / f"lib{name}.a").write_bytes(b"!<arch>\n")
    print(f"OK  sherpa-onnx without text-to-speech: SHERPA_ONNX_LIB_DIR={lib}")


def make_stub_lib(dest: Path) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        definition = Path(tmp) / "stub.def"
        definition.write_text("LIBRARY kyrr_stub\nEXPORTS\n", encoding="ascii")
        subprocess.run(
            [str(msvc_tool("lib.exe")), "/NOLOGO", f"/DEF:{definition}", "/MACHINE:X64", f"/OUT:{dest}"],
            check=True,
            capture_output=True,
        )


def main() -> None:
    ASSETS.mkdir(exist_ok=True)
    for name, digest in MODELS:
        fetch(name, f"{HUSHMIC}/{name}", digest)
    if "--windows" in sys.argv[1:]:
        extract_ort(fetch(*ORT_ZIP))
        extract_sherpa(fetch(*SHERPA_LIB))
    if "--linux" in sys.argv[1:]:
        extract_sherpa(fetch(*SHERPA_LIB_LINUX))
    if "--test" in sys.argv[1:]:
        for name, digest in TEST_AUDIO:
            fetch(name, f"{SHERPA}/{name}", digest, ASSETS / "test-audio")


if __name__ == "__main__":
    main()
