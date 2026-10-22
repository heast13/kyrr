#!/usr/bin/env python3
"""Stages the Linux install tree and builds the AppImage and a tarball.

Run `python3 scripts/fetch-assets.py --linux` and, with SHERPA_ONNX_LIB_DIR
set to assets/sherpa-onnx/lib, `cargo build --release -p kyrr` first. The
binary is checked (scripts/check-binaries.py --linux) before staging.

  dist/stage-linux/kyrr-<version>-linux-x86_64/   the install tree (usr/...)
  dist/kyrr-<version>-linux-x86_64.tar.gz         the same tree (AUR, manual installs)
  dist/kyrr-<version>-x86_64.AppImage
  (each with a .sha256)

The AppImage is the tree plus the libraries a desktop may lack, bundled by
linuxdeploy. It is built on the CI runner's glibc, so it runs on that glibc
or newer. libpipewire, libspa and libasound stay out: they must match the
PipeWire and ALSA setup of the machine kyrr runs on. Where libpipewire is
missing (a system still on PulseAudio), AppRun puts a stand-in for it on the
library path, so kyrr starts, finds no PipeWire and offers to install it.

The tools and the AppImage runtime are downloaded into target/linux-tools/ and checked against the
SHA256 below. They are AppImages and run extracted, so no FUSE is needed.

Standard library only.
"""

import gzip
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
DIST = ROOT / "dist"
TOOLS = ROOT / "target" / "linux-tools"
APP_ID = "io.github.heast13.kyrr"
ARCH = "x86_64"
# The glibc the release runner has (see release.yml); AppRun checks for it.
GLIBC_MIN = "2.39"

# name, download url, sha256
LINUXDEPLOY = (
    "linuxdeploy",
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage",
    "c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d",
)
APPIMAGETOOL = (
    "appimagetool",
    "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage",
    "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0",
)
# The AppImage runtime (the part that mounts the image); appimagetool would
# otherwise download whatever "continuous" is today.
RUNTIME = (
    "runtime",
    "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64",
    "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d",
)
# Only the native engine's weights: the .onnx files would only be read by
# ONNX Runtime, which the AppImage and the tarball do not have (sherpa-onnx
# links it in statically).
MODELS = [
    "dpdfnet8_48khz_hr.weights.f32",
    "dpdfnet2_48khz_hr.weights.f32",
]
# libpipewire, libspa and libasound are never bundled (see above);
# linuxdeploy's own list already keeps out libc, libGL, fontconfig and such.
# OpenSSL stays out too: every distro with this glibc has libssl.so.3, and the
# system's copy knows where its certificates are (for the model download).
EXCLUDE = [
    "libpipewire-0.3.so*", "libspa*", "libasound.so*", "libssl.so*", "libcrypto.so*",
    # Only dependencies of system libraries (freetype, libxcb), which load their own.
    "libpng16.so*", "libbrotli*", "libbz2.so*", "libXau.so*", "libXdmcp.so*", "libbsd.so*", "libmd.so*",
]
# Loaded with dlopen by the window system code, so linuxdeploy does not see
# them; the libxkbcommon-x11 package is missing on some Wayland-only installs.
EXTRA_LIBRARIES = ["libxkbcommon-x11.so.0"]
# Debian packages of the libraries the AppImage bundles, for their notices.
BUNDLED_NOTICES = ["libxkbcommon0", "libxkbcommon-x11-0", "libxcb-xkb1"]


# Where the stand-in for libpipewire lies in the AppImage.
NO_PIPEWIRE = "usr/lib/kyrr-no-pipewire"

# linuxdeploy's default AppRun is a symlink to the program, which then shows
# up as "AppRun" in process lists; exec makes it "kyrr" again. The quick
# look in the usual library folders spares the loader's check on the start
# a dictation key makes.
APPRUN = f"""#!/bin/sh
here="$(dirname "$(readlink -f "$0")")"
# The binary needs the glibc of the build machine ({GLIBC_MIN}) or newer. On an older
# system it would only stop with a loader message nobody sees after a double
# click: say what is wrong, in a dialog where the desktop has one.
have="$(getconf GNU_LIBC_VERSION 2>/dev/null | cut -d' ' -f2)"
if [ -n "$have" ] && [ "$(printf '%s\\n%s\\n' {GLIBC_MIN} "$have" | sort -V | head -n 1)" != {GLIBC_MIN} ]; then
    text="kyrr needs a newer Linux: glibc {GLIBC_MIN} or newer (Ubuntu 24.04, Fedora 40, Debian 13, Linux Mint 22, Arch). This system has glibc $have."
    echo "$text" >&2
    if command -v zenity >/dev/null 2>&1; then zenity --error --title=kyrr --text="$text" 2>/dev/null
    elif command -v kdialog >/dev/null 2>&1; then kdialog --title kyrr --error "$text" 2>/dev/null
    elif command -v notify-send >/dev/null 2>&1; then notify-send kyrr "$text" 2>/dev/null
    fi
    exit 1
fi
found=
for dir in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib /lib/x86_64-linux-gnu; do
    [ -e "$dir/libpipewire-0.3.so.0" ] && found=1 && break
done
if [ -z "$found" ] && LD_TRACE_LOADED_OBJECTS=1 /lib64/ld-linux-x86-64.so.2 "$here/usr/bin/kyrr" 2>/dev/null | grep -q 'libpipewire-0.3.so.0 => not found'; then
    export KYRR_PIPEWIRE_STAND_IN="$here/{NO_PIPEWIRE}"
    export LD_LIBRARY_PATH="$KYRR_PIPEWIRE_STAND_IN${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
fi
exec "$here/usr/bin/kyrr" "$@"
"""


def version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    return re.search(r'^version = "(.+)"', text, re.M).group(1)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def tool(name: str, url: str, digest: str) -> Path:
    """The downloaded tool, checked against its pinned SHA256."""
    dest = TOOLS / name / url.rsplit("/", 1)[1]
    if not (dest.is_file() and sha256(dest) == digest):
        dest.parent.mkdir(parents=True, exist_ok=True)
        print(f"Downloading {url} ...")
        tmp = dest.with_suffix(".part")
        with urllib.request.urlopen(url) as r, tmp.open("wb") as f:
            shutil.copyfileobj(r, f)
        if sha256(tmp) != digest:
            tmp.unlink()
            sys.exit(f"checksum mismatch for {url}")
        tmp.replace(dest)
    dest.chmod(0o755)
    return dest


def write_checksum(path: Path) -> None:
    # LF line ending so `sha256sum -c` works everywhere.
    (DIST / f"{path.name}.sha256").write_text(f"{sha256(path)}  {path.name}\n", encoding="utf-8", newline="\n")


def copy(src: Path, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dest)
    dest.chmod(0o644)


def stage(root: Path) -> None:
    subprocess.run([sys.executable, str(ROOT / "scripts" / "check-binaries.py"), "--linux"], check=True)
    shutil.rmtree(root.parent, ignore_errors=True)
    usr = root / "usr"
    (usr / "bin").mkdir(parents=True)
    shutil.copyfile(ROOT / "target" / "release" / "kyrr", usr / "bin" / "kyrr")
    # Cargo's release profile keeps symbols (30 MB); the tree ships without.
    subprocess.run(["strip", "--strip-unneeded", str(usr / "bin" / "kyrr")], check=True)
    (usr / "bin" / "kyrr").chmod(0o755)
    for f in MODELS:
        copy(ASSETS / f, usr / "share" / "kyrr" / f)
    linux = ROOT / "packaging" / "linux"
    copy(linux / f"{APP_ID}.desktop", usr / "share" / "applications" / f"{APP_ID}.desktop")
    copy(linux / f"{APP_ID}.metainfo.xml", usr / "share" / "metainfo" / f"{APP_ID}.metainfo.xml")
    icons = ROOT / "crates" / "kyrr-app" / "ui" / "icons"
    hicolor = usr / "share" / "icons" / "hicolor"
    copy(icons / "kyrr-256.png", hicolor / "256x256" / "apps" / f"{APP_ID}.png")
    copy(icons / "kyrr.svg", hicolor / "scalable" / "apps" / f"{APP_ID}.svg")
    # Where the app's Setup, About finds THIRD_PARTY.md.
    doc = usr / "share" / "doc" / "kyrr"
    for f in ("THIRD_PARTY.md", "NOTICE", "LICENSE-MIT", "LICENSE-APACHE"):
        copy(ROOT / f, doc / f)
    # ONNX Runtime is linked in with sherpa-onnx.
    for f in ("LICENSE-onnxruntime", "ThirdPartyNotices-onnxruntime"):
        copy(ASSETS / f"{f}-1.28.2.txt", doc / "licenses" / f"{f}.txt")
    # The font of the words in the dictation wave, built into kyrr.
    copy(ROOT / "crates" / "kyrr-common" / "fonts" / "OFL.txt", doc / "licenses" / "OFL-Inter.txt")
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "collect-licenses.py"), "--linux", str(doc / "licenses" / "crates")],
        check=True,
    )
    for path in (doc / "licenses").rglob("*"):
        path.chmod(0o755 if path.is_dir() else 0o644)


def epoch() -> int:
    """A fixed time for the archive: the last commit, else the environment's."""
    if "SOURCE_DATE_EPOCH" in os.environ:
        return int(os.environ["SOURCE_DATE_EPOCH"])
    out = subprocess.run(["git", "log", "-1", "--format=%ct"], cwd=ROOT, capture_output=True, text=True)
    return int(out.stdout.strip() or 0)


def make_tarball(root: Path, dest: Path) -> None:
    mtime = epoch()

    def normalize(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mtime = mtime
        info.mode = 0o755 if info.isdir() or info.mode & 0o100 else 0o644
        return info

    # gzip with a zero timestamp, so the same tree gives the same file.
    with dest.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tar:
            tar.add(root, arcname=root.name, recursive=False, filter=normalize)
            for path in sorted(root.rglob("*")):
                tar.add(path, arcname=f"{root.name}/{path.relative_to(root)}", recursive=False, filter=normalize)


def make_appimage(root: Path, work: Path, dest: Path) -> None:
    linuxdeploy = tool(*LINUXDEPLOY)
    appimagetool = tool(*APPIMAGETOOL)
    runtime = tool(*RUNTIME)
    appdir = work / "kyrr.AppDir"
    shutil.rmtree(appdir, ignore_errors=True)
    shutil.copytree(root, appdir, symlinks=True)
    usr = appdir / "usr"
    env = {
        **os.environ,
        "APPIMAGE_EXTRACT_AND_RUN": "1",  # no FUSE on CI
        "NO_STRIP": "1",  # the runner's strip does not know every distro library's format
        "ARCH": ARCH,
        "LINUXDEPLOY_OUTPUT_VERSION": version(),
    }
    apprun = work / "AppRun"
    apprun.write_text(APPRUN, encoding="utf-8")
    apprun.chmod(0o755)
    cmd = [str(linuxdeploy), f"--appdir={appdir}", f"--executable={usr / 'bin' / 'kyrr'}", f"--custom-apprun={apprun}"]
    cmd += [f"--desktop-file={usr / 'share' / 'applications' / f'{APP_ID}.desktop'}"]
    cmd += [f"--icon-file={usr / 'share' / 'icons' / 'hicolor' / '256x256' / 'apps' / f'{APP_ID}.png'}"]
    cmd += [f"--exclude-library={pattern}" for pattern in EXCLUDE]
    for lib in EXTRA_LIBRARIES:
        cmd += [f"--library={find_library(lib)}"]
    subprocess.run(cmd, check=True, env=env)
    make_pipewire_stand_in(usr / "bin" / "kyrr", appdir / NO_PIPEWIRE)
    # The bundled libraries' notices (MIT), from the build system's packages.
    licenses = usr / "share" / "doc" / "kyrr" / "licenses"
    for package in BUNDLED_NOTICES:
        notice = Path("/usr/share/doc") / package / "copyright"
        if not notice.is_file():
            sys.exit(f"{notice} not found (install {package})")
        copy(notice, licenses / f"{package}.txt")
    dest.unlink(missing_ok=True)
    # --no-appstream: the metainfo is validated with the AppStream tools.
    subprocess.run([str(appimagetool), "--no-appstream", f"--runtime-file={runtime}", str(appdir), str(dest)], check=True, env=env)
    dest.chmod(0o755)


def make_pipewire_stand_in(binary: Path, dest: Path) -> None:
    """A libpipewire-0.3.so.0 with the functions kyrr calls, each returning 0.

    kyrr calls pw_init and then pw_main_loop_new, whose NULL the pipewire
    crate reports as an error: the noise filter is off and the window offers
    to install PipeWire. Nothing else in kyrr calls into PipeWire before
    that. Built from the symbols the binary needs and the real library has,
    so a new call in kyrr cannot be missing from it.
    """
    def symbols(path: Path, flag: str) -> dict[str, str]:
        out = subprocess.run(["nm", "-D", flag, str(path)], capture_output=True, text=True, check=True).stdout
        found = {}
        for line in out.splitlines():
            parts = line.split()
            if len(parts) >= 2:
                found[parts[-1].split("@")[0]] = parts[-2]
        return found

    real = symbols(find_library("libpipewire-0.3.so.0", "libpipewire-0.3-0"), "--defined-only")
    used = sorted(set(symbols(binary, "--undefined-only")) & set(real))
    if not used or any(real[name] != "T" for name in used):
        sys.exit(f"libpipewire stand-in: unexpected symbols {[(n, real[n]) for n in used]}")
    source = "".join(f"void *{name}(void) {{ return 0; }}\n" for name in used)
    dest.mkdir(parents=True, exist_ok=True)
    lib = dest / "libpipewire-0.3.so.0"
    subprocess.run(
        ["cc", "-shared", "-fPIC", "-O2", "-Wl,-soname,libpipewire-0.3.so.0", "-o", str(lib), "-x", "c", "-"],
        input=source, text=True, check=True,
    )
    subprocess.run(["strip", "--strip-unneeded", str(lib)], check=True)
    lib.chmod(0o644)


def find_library(soname: str, package: str = "libxkbcommon-x11-0") -> Path:
    ldconfig = shutil.which("ldconfig") or "/sbin/ldconfig"  # /sbin is not on every user's PATH
    out = subprocess.run([ldconfig, "-p"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        if line.split()[:1] == [soname] and "x86-64" in line:
            return Path(line.split(" => ")[1])
    sys.exit(f"{soname} not found (install {package})")


def main() -> None:
    ver = version()
    name = f"kyrr-{ver}-linux-{ARCH}"
    work = DIST / "stage-linux"
    root = work / name
    DIST.mkdir(exist_ok=True)
    stage(root)
    tarball = DIST / f"{name}.tar.gz"
    make_tarball(root, tarball)
    write_checksum(tarball)
    appimage = DIST / f"kyrr-{ver}-{ARCH}.AppImage"
    make_appimage(root, work, appimage)
    write_checksum(appimage)
    print(tarball)
    print(appimage)


if __name__ == "__main__":
    main()
