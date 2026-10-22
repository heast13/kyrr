# Building kyrr

Building downloads kyrr's assets and libraries, from kyrr's mirror on GitHub
first.

## Windows

With Rust (MSVC), the Visual Studio Build Tools and Python 3; for
the installer Inno Setup 6.

```powershell
python scripts/fetch-assets.py --windows
$env:SHERPA_ONNX_LIB_DIR = "$PWD\assets\sherpa-onnx\lib"
cargo build --release -p kyrr -p kyrr-apo
python scripts/package.py
```

`SHERPA_ONNX_LIB_DIR` points the build at sherpa-onnx's libraries without
text-to-speech; the default ones bundle espeak-ng (GPL-3.0).
`scripts/check-binaries.py` (run by `package.py` and CI) refuses a build
that contains it, or a filter DLL that needs the Visual C++ runtime.

## Linux

With the packages libpipewire-0.3-dev, libclang-dev, libasound2-dev,
libfontconfig1-dev, libxkbcommon-dev, libxkbcommon-x11-0 and libssl-dev, or
their names elsewhere installed:

```sh
python3 scripts/fetch-assets.py --linux
SHERPA_ONNX_LIB_DIR=$PWD/assets/sherpa-onnx/lib cargo build --release -p kyrr
python3 scripts/package-linux.py   # dist/: AppImage and tarball, each with a .sha256
```

`package-linux.py` downloads linuxdeploy and appimagetool (pinned by SHA256)
and needs `libxkbcommon-x11` installed to bundle it. The AppImage runs on the
glibc of the machine that built it or newer (the release is built on
Ubuntu 24.04: glibc 2.39).

## Without the full app

The cores (`kyrr-denoise`, `kyrr-voice`, `kyrr-dictate`) build and test on Linux too;
`cargo run -p kyrr --example preview -- <dir>` renders the window headless.
