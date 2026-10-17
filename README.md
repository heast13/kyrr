# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
  <a href="#install-on-linux"><img src="https://img.shields.io/badge/Install%20on%20Linux-beta-005fb8?style=for-the-badge" alt="Install kyrr on Linux (beta)"></a>
</p>

Website: [heast13.github.io/kyrr](https://heast13.github.io/kyrr/) (Deutsch: [/de/](https://heast13.github.io/kyrr/de/))

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

A small, free app for your microphone, for **Windows** and **Linux**:
cleaner sound, a better voice and dictation, all on your own computer.

> **Everything runs on your PC.** Your voice and your dictated text never
> leave your computer: no cloud, no account, no telemetry. Once the speech
> model is downloaded, kyrr works fully offline. [Details](#privacy)

What it does:

- **Remove background noise.** Real-time neural noise suppression
  ([DPDFNet](https://github.com/ceva-ip/DPDFNet)) on your microphone, for
  every app: Teams, Zoom, Discord, OBS, the browser. A large model for the
  cleanest result, a small one for older laptops.
- **Improve voice.** Shaping after the noise is gone: a sound (Neutral, Warm,
  Clear, Bright or your own), a compressor, softer S sounds, pop protection
  and a cut for low rumble.
- **Automatic volume.** Brings your voice to the same level however loud
  your microphone is set.
- **Dictation.** Hold F9 (or F8, Pause, right Ctrl; or tap to start and
  stop), speak, release. The text appears where your cursor is. German and
  English, transcribed on your PC. Say "new line" or "neue
  Zeile" for a line break.
- **Check.** A live level of what the microphone hears and what apps get,
  a ten-second test (record once, then play the take before and after kyrr)
  and listening to yourself live on headphones.
- **Follows your microphone.** Plug in another one, or the same one into
  another USB port, and kyrr moves along by itself. It also tells you what
  is wrong: a muted microphone (with a button to switch it back on), none
  found, or one that just arrived.

<p align="center">
  <img src="docs/screenshots/main.png" width="380" alt="kyrr's window: noise filter, improve voice and dictation, each with one switch">
</p>
<p align="center">
  <img src="docs/screenshots/settings.png" width="760" alt="Settings in two columns: filter strength and model, voice sound with its sliders, compressor, S sounds, low cut, pop protection, automatic volume, language, autostart, updates and microphone; dictation key, level check with before/after test and listening, about">
</p>

Status: young and in active development.

## Compared with

| | kyrr | NVIDIA Broadcast | Krisp |
|---|---|---|---|
| Price | free, open source | free | free for 60 minutes a day, subscription beyond |
| Needs | any 64-bit Intel or AMD PC with Windows 10 or 11, or Linux with PipeWire | an NVIDIA RTX graphics card | an account |
| Where it works | on the microphone itself: every app, nothing to choose | apps set to its virtual microphone | apps set to its virtual microphone |

On top, kyrr dictates into any app, on your computer. As far as we know in
October 2026; corrections welcome.

## Install

### Windows

Windows 10 22H2 or Windows 11, 64-bit.

1. Download `kyrr-setup-<version>.exe` from the [latest
   release](https://github.com/heast13/kyrr/releases/latest) (see the [code
   signing policy](#code-signing-policy)).
2. Run it. The installer needs administrator rights, and sound drops out for
   a second while kyrr sets up its filter.
3. Until the installer is code-signed, Windows SmartScreen warns about it
   ("More info", "Run anyway"); compare the installer with its `.sha256`
   file from the release page first.

On Windows on ARM (Snapdragon and other ARM processors) kyrr only dictates:
its noise filter is built for Intel and AMD processors, and the audio engine
there cannot load it.

<details>
<summary>Uninstall</summary>

Uninstalling takes the filter off the microphone, puts the Windows setting
described under "How it works" back and asks whether to delete the downloaded
speech models.

</details>

<a id="install-on-linux"></a>

### Linux (beta, AppImage)

The Linux version works, but has been tested on fewer systems than the
Windows version. One file, an AppImage, for every distribution.

**With a terminal.** Paste this; it downloads kyrr, makes it executable and
starts it:

```sh
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/kyrr https://github.com/heast13/kyrr/releases/latest/download/kyrr-x86_64.AppImage
chmod +x ~/.local/bin/kyrr
~/.local/bin/kyrr &
```

**Prefer clicking?**

1. Download `kyrr-x86_64.AppImage` from the [latest
   release](https://github.com/heast13/kyrr/releases/latest).
2. Allow executing it as a program in your file manager (right click,
   Properties).
3. Double click it.

Either way kyrr adds itself to your app menu on its first start.

**Start at sign-in** switches on by itself when the first-start setup is
done. Switch it off or on again in Settings, "Start when you sign in".

**Updates** are not installed by themselves: kyrr says when there is a new
version; quit kyrr and paste the install commands again to replace the file.

**Requirements**

- 64-bit Intel or AMD with glibc 2.39 or newer (Ubuntu 24.04, Fedora 40,
  Debian 13, Linux Mint 22, Arch or newer).
- PipeWire for the noise filter, which these use anyway; where it is
  missing, a button in kyrr installs it (your system asks for your password).
- A processor with AVX2 for the noise filter (most from 2015 on); dictation
  works without.
- No administrator rights needed.

**Desktop notes**

- GNOME 48+ and KDE: your desktop asks once for the dictation key and once
  whether kyrr may type; kyrr walks you through both.
- GNOME 46 and 47: kyrr adds the key to GNOME's own keyboard shortcuts, where
  each press starts or ends dictation.
- Hyprland (Omarchy, say), Sway, river and niri: kyrr adds the key to your
  key bindings itself, in a block of its own that it keeps up to date, and
  the start at sign-in too where the session does not start XDG autostart
  entries.
- X11 desktops (Linux Mint, XFCE): the key just works.
- Without a tray icon the window's X minimizes kyrr where the desktop can
  bring it back (GNOME, say) and quits it on tiling ones; "Quit" always ends
  it.

<details>
<summary>Uninstall</summary>

Quit kyrr ("Quit" at the bottom left), then:

```sh
rm -f ~/.local/bin/kyrr \
  ~/.config/autostart/io.github.heast13.kyrr.desktop \
  ~/.local/share/applications/io.github.heast13.kyrr.desktop \
  ~/.local/share/icons/hicolor/256x256/apps/io.github.heast13.kyrr.png \
  ~/.local/share/icons/hicolor/scalable/apps/io.github.heast13.kyrr.svg
rm -rf ~/.config/kyrr ~/.local/share/kyrr
```

The last line also deletes your settings and the downloaded speech models;
leave it out to keep them. Used a different place for the file? Delete that
instead of `~/.local/bin/kyrr`.

On Hyprland, Sway, river and niri, delete the blocks between the `>>> kyrr`
and `<<< kyrr` lines (key and start at sign-in) from your config; on GNOME,
delete the "kyrr" entry under Custom Shortcuts in the keyboard settings.

</details>

## How it works

**Remove background noise.** kyrr installs an audio processing object (APO), a
filter the Windows audio engine runs on the microphone itself, the way
Equalizer APO does. Every app gets the cleaned signal without choosing a
virtual device. If the microphone's driver has its own effect (for example
beamforming), kyrr keeps it and runs after it. Latency is 60 ms at 48 kHz.
On processors without AVX2 the filter falls back to ONNX Runtime and kyrr
switches to the small model once.

Windows only loads audio filters signed by Microsoft unless the setting
`DisableProtectedAudioDG` is on. kyrr's installer switches it on, as
Equalizer APO's does, says so before it starts, and restores the previous
value on uninstall (unless Equalizer APO is installed and still needs it).
If a Windows or driver update takes kyrr off the microphone, the app notices
and offers to repair it.

On Linux kyrr does the same as a PipeWire microphone of its own, called
"kyrr": it reads your real microphone, removes the noise and becomes the
system's default microphone, so every app that uses the default gets the
cleaned signal. kyrr only reads your real microphone while an app listens to
"kyrr". When kyrr quits, its microphone goes away and the system falls back
to your real one.

**Improve voice.** The same filter can shape the voice: a low cut (80 Hz,
100 Hz for the bright sound) before the noise filter, then three EQ bands
(body at 250 Hz, presence at 3 kHz, air above 8 kHz), a de-esser on the band
above 4 kHz, a compressor (light, medium or strong) and a limiter at
−1 dBFS. It works with the noise filter switched off too, and adds no
latency. On by default with a neutral sound, medium compression, light
S-sound softening and the low cut; pop protection is off by default. Reset
brings back the defaults.

*Pop protection* listens to the raw microphone for the thump of a P or B and
turns everything below 120 Hz down while it lasts, starting 10 ms before the
thump arrives. *Automatic volume* (on by default, works with the voice
shaping switched off too) measures the voice only while someone speaks and
moves it slowly (5 dB/s up, 10 dB/s down, at most +24/−12 dB) towards about
−23 dBFS; pauses, clicks and steady noise leave it alone. Apps that
adjust the volume themselves (call apps, say) can work against it, so use
one of the two.

**Your microphone.** kyrr checks every few seconds which microphone the system
uses. When it changes, the filter moves along: Windows asks for permission
once and sound pauses for a moment; on Linux it simply moves. If that does not
happen (you declined, say), a button on the main page offers it. A microphone
you pick by hand stays until the system's default changes.

**Check.** On Windows the filter shares a small block of memory with the app
(`Global\kyrr-monitor-3`): the level before and after kyrr, what the
automatic volume adds, and on request one ten-second take, unprocessed and
processed. The audio service writes it; users signed in at this computer can
read it and ask for a take, which only records while an app already uses the
microphone. The app plays the take, keeps it only in memory and clears it
from the shared block.

On Linux the filter runs inside the app, which reads the levels and the
take directly.

**Dictation.** kyrr records while the key is held (up to 15 minutes) and
transcribes locally with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (its build without
text-to-speech). The speech model is downloaded once on first use and let go
of after a quarter of an hour without dictation:

| Language | Model | Size |
|---|---|---|
| German | [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline), a German fine-tune of NVIDIA Parakeet; also handles English | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 663 MB |

## Privacy

Audio and text never leave your computer. No telemetry. kyrr connects to
the internet for exactly these things:

- **The speech model**, once, when you first use dictation: from kyrr's own
  mirrors ([Hugging Face](https://huggingface.co/heast13/kyrr-models), else
  [GitHub](https://github.com/heast13/kyrr-assets); the English model last
  from the sherpa-onnx project's Hugging Face page), every file checked
  against a SHA256 hash built into kyrr.
- **The update check**: once a day, one request to the GitHub API for the
  newest version number of kyrr (`api.github.com/repos/heast13/kyrr/releases/latest`),
  whose answer also carries the short "What's new" text of that version.
  It sends nothing about you beyond what any web request carries: your IP
  address and "kyrr/<version>" as the program's name. It can be switched
  off under Settings, General ("Check for updates automatically"); then
  kyrr only asks when you click "Check for updates".
- **An update**, only when you click "Update": the installer of the new
  version from this repository's releases, checked against its SHA256 file.
  On Linux kyrr opens the release page instead.

Nothing is installed without your click. (Building kyrr also downloads its
assets and libraries, from kyrr's mirror on GitHub first.)

## Code signing policy

kyrr's releases are not code-signed. A free certificate from the SignPath
Foundation was applied for and declined for now, because kyrr is not known
widely enough yet. That is why Windows SmartScreen warns about the installer
("More info", "Run anyway").

To check that a download is the one built from this repository, compare it
with the `.sha256` file on the release page. Every release is built on
GitHub by the [release workflow](.github/workflows/release.yml) from a
version tag of this repository. Third-party files keep the signatures of
their authors (ONNX Runtime and the Visual C++ runtime by Microsoft); models
are data.

kyrr sends no data about you or your computer anywhere, see
[Privacy](#privacy).

## Building

On Windows, with Rust (MSVC), the Visual Studio Build Tools and Python 3; for
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

On Linux (the packages libpipewire-0.3-dev, libclang-dev, libasound2-dev,
libfontconfig1-dev, libxkbcommon-dev, libxkbcommon-x11-0 and libssl-dev, or
their names elsewhere):

```sh
python3 scripts/fetch-assets.py --linux
SHERPA_ONNX_LIB_DIR=$PWD/assets/sherpa-onnx/lib cargo build --release -p kyrr
python3 scripts/package-linux.py   # dist/: AppImage and tarball, each with a .sha256
```

`package-linux.py` downloads linuxdeploy and appimagetool (pinned by SHA256)
and needs `libxkbcommon-x11` installed to bundle it. The AppImage runs on the
glibc of the machine that built it or newer (the release is built on
Ubuntu 24.04: glibc 2.39).

The cores (`kyrr-denoise`, `kyrr-voice`, `kyrr-dictate`) build and test on Linux too;
`cargo run -p kyrr --example preview -- <dir>` renders the window headless.

## License

kyrr is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party components and models keep their own licenses,
see [THIRD_PARTY.md](THIRD_PARTY.md); the app shows the model credits under
Settings, About. kyrr's window is made with [Slint](https://slint.dev) under
its royalty-free license, which asks for the badge at the top of this page.
Security issues: see [SECURITY.md](SECURITY.md).

If you build on kyrr, please keep the copyright notice and
[NOTICE](NOTICE) (the licenses ask for it) and, if you like, mention
heast13 with a link to this repository.

## Thanks

kyrr stands on the work of others. Thank you to

- [Fovty](https://github.com/Fovty) for HushMic, whose DPDFNet runtime
  kyrr's filter is built on,
- [Ceva](https://github.com/ceva-ip/DPDFNet) for DPDFNet and for releasing
  its models openly,
- [primeLine AI](https://huggingface.co/primeline/parakeet-primeline) for
  the German speech model,
- NVIDIA for Parakeet, the model family behind both languages,
- csukuangfj and the [k2-fsa](https://github.com/k2-fsa/sherpa-onnx) team
  for sherpa-onnx and the English model in ONNX,
- [Equalizer APO](https://sourceforge.net/projects/equalizerapo/), which
  showed how a filter can live inside Windows audio,
- [Slint](https://slint.dev) for the UI toolkit.
