#!/usr/bin/env python3
"""Downloads the runtime assets into assets/ and verifies their SHA256.

- DPDFNet models and native-engine weights (Ceva, Apache-2.0) from the
  HushMic v0.10.2 release
- ONNX Runtime 1.27.0 for Windows (Microsoft, MIT), the fallback engine on
  CPUs without AVX2/FMA; only with --windows
- two short speech clips from the sherpa-onnx test assets for the dictation
  tests; only with --test

Standard library only, so it runs the same on Linux and Windows.
"""

import hashlib
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
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
    folder.mkdir(exist_ok=True)
    dest = folder / name
    if dest.is_file() and sha256(dest) == digest:
        print(f"OK  {name}")
        return dest
    print(f"Downloading {name} ...")
    tmp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(url) as r, tmp.open("wb") as f:
        shutil.copyfileobj(r, f)
    if sha256(tmp) != digest:
        tmp.unlink()
        sys.exit(f"checksum mismatch: {name}")
    tmp.replace(dest)
    print(f"OK  {name}")
    return dest


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


def main() -> None:
    ASSETS.mkdir(exist_ok=True)
    for name, digest in MODELS:
        fetch(name, f"{HUSHMIC}/{name}", digest)
    if "--windows" in sys.argv[1:]:
        extract_ort(fetch(*ORT_ZIP))
    if "--test" in sys.argv[1:]:
        for name, digest in TEST_AUDIO:
            fetch(name, f"{SHERPA}/{name}", digest, ASSETS / "test-audio")


if __name__ == "__main__":
    main()
