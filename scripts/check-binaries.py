#!/usr/bin/env python3
"""Checks the release binaries before they ship (Windows build):

- kyrr.exe contains no text-to-speech code from sherpa-onnx (espeak-ng is
  GPL-3.0; kyrr links the no-TTS libraries, see fetch-assets.py)
- kyrr_apo.dll imports no Visual C++ runtime: inside audiodg.exe a missing
  runtime means the filter silently never loads

Standard library only.
"""

import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RELEASE = ROOT / "target" / "release"
FORBIDDEN_TEXT = [b"espeak", b"piper_phonemize"]
FORBIDDEN_IMPORTS = ("vcruntime", "msvcp", "api-ms-win-crt", "ucrtbase")


def imports(path: Path) -> list[str]:
    """Names of the DLLs a PE32+ file imports."""
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    sections = struct.unpack_from("<H", data, pe + 6)[0]
    optional = struct.unpack_from("<H", data, pe + 20)[0]
    opt = pe + 24
    import_rva = struct.unpack_from("<I", data, opt + 120)[0]
    table = opt + optional

    def offset(rva: int) -> int:
        for i in range(sections):
            s = table + 40 * i
            size, va, _, raw = struct.unpack_from("<IIII", data, s + 8)
            if va <= rva < va + max(size, 1):
                return rva - va + raw
        raise ValueError(f"rva {rva:#x} outside sections")

    names = []
    at = offset(import_rva)
    while True:
        name_rva = struct.unpack_from("<I", data, at + 12)[0]
        if name_rva == 0:
            break
        start = offset(name_rva)
        names.append(data[start : data.index(b"\0", start)].decode("ascii"))
        at += 20
    return names


def main() -> None:
    problems = []
    exe = (RELEASE / "kyrr.exe").read_bytes()
    for text in FORBIDDEN_TEXT:
        if text in exe:
            problems.append(f"kyrr.exe contains {text.decode()}: build with SHERPA_ONNX_LIB_DIR (no-TTS libraries)")
    for name in imports(RELEASE / "kyrr_apo.dll"):
        if name.lower().startswith(FORBIDDEN_IMPORTS):
            problems.append(f"kyrr_apo.dll imports {name}: static CRT (.cargo/config.toml) missing")
    if problems:
        sys.exit("\n".join(problems))
    print("OK  kyrr.exe without text-to-speech, kyrr_apo.dll without C runtime imports")


if __name__ == "__main__":
    main()
