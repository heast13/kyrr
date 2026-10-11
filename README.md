# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
</p>

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

A small, free Windows app for your microphone.

> **Everything runs on your PC.** Your voice and your dictated text never
> leave your computer: no cloud, no account, no telemetry. Once the speech
> model is downloaded, kyrr works fully offline. [Details](#privacy)

What it does:

- **Remove background noise.** Real-time neural noise suppression
  ([DPDFNet](https://github.com/ceva-ip/DPDFNet)) on your microphone, for
  every app: Teams, Zoom, Discord, OBS, the browser. A large model for the
  cleanest result, a small one for older laptops.
- **Voice.** Shaping after the noise is gone: a sound (Neutral, Warm,
  Clear, Bright or your own), even volume, softer S sounds and a cut for
  low rumble.
- **Dictation.** Hold F9 (or F8, Pause, right Ctrl; or tap to start and
  stop), speak, release. The text appears where your cursor is. German and
  English, transcribed on your PC. Say "new line" or "neue
  Zeile" for a line break.
- **Check.** A live level of what the microphone hears and what apps get,
  a ten-second test (record once, then play the take before and after kyrr)
  and listening to yourself live on headphones.

<p align="center">
  <img src="docs/screenshots/main.png" width="380" alt="kyrr's window: noise filter, voice and dictation, each with one switch">
</p>
<p align="center">
  <img src="docs/screenshots/settings.png" width="760" alt="Settings in two columns: noise strength and model, voice sound, volume, S sounds, low cut, language and autostart; dictation key, level check with before/after test and listening, about">
</p>

Status: early, first public release (0.2.0).

## Compared with

| | kyrr | NVIDIA Broadcast | Krisp |
|---|---|---|---|
| Price | free, open source | free | subscription after a free trial |
| Needs | any 64-bit Intel or AMD PC with Windows 10 or 11 | an NVIDIA RTX graphics card | an account |
| Where it works | on the microphone itself: every app, nothing to choose | apps set to its virtual microphone | apps set to its virtual microphone |

On top, kyrr dictates into any app, on your computer. As far as we know in
October 2026; corrections welcome.

## Install

Download `kyrr-setup-<version>.exe` from the [latest
release](https://github.com/heast13/kyrr/releases/latest) and run it (see the
[code signing policy](#code-signing-policy)).

Windows 10 22H2 or Windows 11, 64-bit. The installer needs administrator
rights, and sound drops out for a second while kyrr sets up its filter.
On Windows on ARM (Snapdragon and other ARM processors) kyrr only
dictates: its noise filter is built for Intel and AMD processors, and the
audio engine there cannot load it.
Until the installer is code-signed, Windows SmartScreen warns about it
("More info", "Run anyway"); compare the installer with its `.sha256` file
from the release page first.

Uninstalling takes the filter off the microphone, puts the Windows setting
below back and asks whether to delete the downloaded speech models.

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

**Voice.** The same filter can shape the voice, with the chain of
[Omavoice DPDFNet](https://github.com/heast13/omarchy-omavoice-dpdfnet) and
its sounds and de-esser levels: a low cut (80 Hz) before the noise filter,
then three EQ bands (body at 250 Hz, presence at 3 kHz, air above 8 kHz), a
de-esser on the band above 4 kHz, a compressor (light, medium or strong) and
a limiter at −1 dBFS. It works with the noise filter switched off too, and
adds no latency. On by default with a neutral sound, light compression and
the low cut; Reset brings back the defaults.

**Check.** The filter shares a small block of memory with the app
(`Global\kyrr-monitor-2`): the level before and after kyrr, and on request
one ten-second take, unprocessed and processed. The audio service writes
it; users signed in at this computer can read it and ask for a take, which
only records while an app already uses the microphone. The app plays the
take, keeps it only in memory and clears it from the shared block.

**Dictation.** kyrr records while the key is held and transcribes locally
with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (its build without
text-to-speech). The speech model is downloaded once on first use and let go
of after a quarter of an hour without dictation:

| Language | Model | Size |
|---|---|---|
| German | [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline), a German fine-tune of NVIDIA Parakeet; also handles English | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 663 MB |

## Privacy

Audio never leaves your computer. The app makes network connections only to
download the speech model, from Hugging Face, with every file checked
against a SHA256 hash built into kyrr, and when you click "Check for
updates", one request to GitHub for the newest version number. Nothing is
downloaded or installed by itself. No telemetry.
(Building kyrr also downloads its assets and libraries from GitHub.)

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org).

- Signed are kyrr's own files: `kyrr.exe`, `kyrr_apo.dll` and the installer
  `kyrr-setup-<version>.exe`, each built by the [release
  workflow](.github/workflows/release.yml) on GitHub from a version tag of
  this repository. Third-party files keep the signatures of their authors
  (ONNX Runtime and the Visual C++ runtime by Microsoft); models are data.
- Committers and reviewers: [heast13](https://github.com/heast13).
  Approvers, who approve every signing request: [heast13](https://github.com/heast13).
- Privacy policy: see [Privacy](#privacy). kyrr sends no data about you or
  your computer anywhere; it only downloads the speech model from Hugging
  Face and, when you click "Check for updates", asks GitHub for the newest
  version number. Their privacy policies apply to those requests:
  [Hugging Face](https://huggingface.co/privacy),
  [GitHub](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

## Building

Windows with Rust (MSVC), the Visual Studio Build Tools and Python 3; for
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
- [Slint](https://slint.dev) for the UI toolkit
