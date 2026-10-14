# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
</p>

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

A small, free app for your microphone, for **Windows** and **Linux**:
cleaner sound, a better voice and dictation, all on your own computer.
Windows is out; the Linux version (an AppImage) follows shortly.

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
| Needs | any 64-bit Intel or AMD PC with Windows 10 or 11 (Linux soon) | an NVIDIA RTX graphics card | an account |
| Where it works | on the microphone itself: every app, nothing to choose | apps set to its virtual microphone | apps set to its virtual microphone |

On top, kyrr dictates into any app, on your computer. As far as we know in
October 2026; corrections welcome.

## Install

**Windows.** Download `kyrr-setup-<version>.exe` from the [latest
release](https://github.com/heast13/kyrr/releases/latest) and run it (see the
[code signing policy](#code-signing-policy)).

**Linux.** An AppImage is coming; this page gets its download then.

Windows 10 22H2 or Windows 11, 64-bit. The installer needs administrator
rights, and sound drops out for a second while kyrr sets up its filter.
On Windows on ARM (Snapdragon and other ARM processors) kyrr only
dictates: its noise filter is built for Intel and AMD processors, and the
audio engine there cannot load it.
Until the installer is code-signed, Windows SmartScreen warns about it
("More info", "Run anyway"); compare the installer with its `.sha256` file
from the release page first.

Uninstalling takes the filter off the microphone, puts the Windows setting
described under "How it works" back and asks whether to delete the downloaded speech models.

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

**Your microphone.** kyrr checks every few seconds which microphone the
system uses. When it changes, the filter moves along: Windows asks for
permission once and sound pauses for a moment. If you decline, a button on
the main page offers it. A microphone you pick by hand stays until the
system's default changes.

**Check.** The filter shares a small block of memory with the app
(`Global\kyrr-monitor-3`): the level before and after kyrr, what the
automatic volume adds, and on request one ten-second take, unprocessed and
processed. The audio service writes it; users signed in at this computer can
read it and ask for a take, which only records while an app already uses the
microphone. The app plays the take, keeps it only in memory and clears it
from the shared block.

**Dictation.** kyrr records while the key is held and transcribes locally
with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (its build without
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
