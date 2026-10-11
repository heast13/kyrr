# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
</p>

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

kyrr takes the background noise out of your microphone as well as paid
tools do, and turns what you say into text, in English and German. A small
Windows app, free and open source, set up in a few clicks.

> **Everything runs on your PC.** Your voice and your dictated text never
> leave your computer: no cloud, no account, no telemetry. Once the speech
> model is downloaded, kyrr works fully offline. [Details](#privacy)

## Why kyrr

**Background noise gone, like with Krisp, without the subscription.** That
was the goal from day one: keyboard, fan, traffic or the dog stay out,
your voice comes through. kyrr uses
[DPDFNet](https://github.com/ceva-ip/DPDFNet), a current neural model,
right inside Windows audio. In my own side-by-side listening it keeps up
with the paid tools I know, Krisp included. That is my ear, not a lab
benchmark, so judge for yourself: the built-in before/after test takes ten
seconds. It is fast, too: 60 ms latency, and on a current desktop
processor about 5 to 11 % of one core.

**Dictation you can rely on, in English and German.** Hold a key, talk,
let go, and the text appears where your cursor is, with punctuation and
capitals. English runs on NVIDIA's Parakeet, one of the most accurate
open speech models; in a small test on LibriSpeech it got about one
word in a hundred wrong. German was the hard part: I never found dictation
software that got it right without fuss. kyrr uses a German fine-tune of
Parakeet by primeLine AI, and for me it is the first that just works. Both
are quick: on a current desktop, a ten-second sentence is text in well
under a second.

**Simple on purpose.** Install, pick your microphone, done: every app
gets the clean voice, with nothing to change in Teams, Zoom or anywhere
else. The window has three switches, dictation is one key, and kyrr waits
quietly in the tray. The settings are there if you want them; you don't
have to touch them.

**Free, local, open source.** No subscription, no account, no cloud. Your
voice and your text stay on your computer, and after the one-time model
download kyrr needs no internet at all. The code is here for anyone to
read.

## What it does

- **Remove background noise** on the microphone itself, for every app:
  Teams, Zoom, Discord, OBS, the browser. A large model for the cleanest
  result, a small one for older laptops.
- **Voice.** A bit of polish after the noise is gone: a sound (Neutral,
  Warm, Clear, Bright or your own), even volume, softer S sounds and a cut
  for low rumble.
- **Dictation.** Hold F9 (or F8, Pause, right Ctrl; or tap to start and
  stop), speak, release. Say "new line" or "neue Zeile" for a line break.
- **Check.** A live level, a ten-second before/after test and listening to
  yourself on headphones.

<p align="center">
  <img src="docs/screenshots/main.png" width="380" alt="kyrr's window: noise filter, voice and dictation, each with one switch">
</p>
<p align="center">
  <img src="docs/screenshots/settings.png" width="760" alt="Settings in two columns: noise strength and model, voice sound, volume, S sounds, low cut, language and autostart; dictation key, level check with before/after test and listening, about">
</p>

A young project that its author uses every day. Feedback and bug reports
are welcome in the [issues](https://github.com/heast13/kyrr/issues).

## Compared with

| | kyrr | NVIDIA Broadcast | Krisp |
|---|---|---|---|
| Price | free, open source | free | subscription after a free trial |
| Needs | any 64-bit Intel or AMD PC with Windows 10 or 11 | an NVIDIA RTX graphics card | an account |
| Where it works | on the microphone itself: every app, nothing to choose | apps set to its virtual microphone | apps set to its virtual microphone |

On top, kyrr dictates into any app, on your computer. As far as I know in
October 2026; corrections welcome.

## Install

Download and run the [installer](https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe)
(all versions on the [releases page](https://github.com/heast13/kyrr/releases)).
Windows 10 22H2 or Windows 11, 64-bit. Setup asks for administrator rights,
and sound drops out for a second while kyrr sets up its filter.

- Until the installer is [code-signed](#code-signing-policy), Windows
  SmartScreen warns about it ("More info", "Run anyway"). Compare it with
  the `.sha256` file from the release page first if you like.
- On Windows on ARM (Snapdragon and other ARM processors) kyrr only
  dictates: its noise filter is built for Intel and AMD processors, and the
  audio engine there cannot load it.
- Uninstalling takes the filter off the microphone, puts the Windows
  setting below back and asks whether to delete the speech models.

## How it works

**Remove background noise.** kyrr installs an audio processing object (APO),
a filter the Windows audio engine runs on the microphone itself, the way
Equalizer APO does. Every app gets the cleaned signal without choosing a
virtual device. If the microphone's driver has its own effect (for example
beamforming), kyrr keeps it and runs after it. On processors without AVX2
the filter falls back to ONNX Runtime and kyrr switches to the small model
once.

Windows only loads audio filters signed by Microsoft unless the setting
`DisableProtectedAudioDG` is on. kyrr's installer switches it on, as
Equalizer APO's does, says so before it starts, and restores the previous
value on uninstall (unless Equalizer APO is installed and still needs it).
If a Windows or driver update takes kyrr off the microphone, the app notices
and offers to repair it.

**Voice.** The same filter shapes the voice, with the chain of
[Omavoice DPDFNet](https://github.com/heast13/omarchy-omavoice-dpdfnet): a low
cut (80 Hz) before the noise filter, then three EQ bands (body at 250 Hz,
presence at 3 kHz, air above 8 kHz), a de-esser above 4 kHz, a compressor
and a limiter at −1 dBFS. No added latency, and it works with the noise
filter off too. Default: neutral sound, medium compression, low cut on.

**Check.** The filter shares a small block of memory with the app
(`Global\kyrr-monitor-2`): the level before and after kyrr, and on request
one ten-second take, unprocessed and processed. The audio service writes
it; users signed in at this computer can read it and ask for a take, which
only records while an app already uses the microphone. The app keeps the
take only in memory and clears it from the shared block.

**Dictation.** kyrr records while the key is held and transcribes locally
with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (its build without
text-to-speech). The text is pasted at the cursor; the clipboard is put back
afterwards. The speech model is downloaded once on first use and let go of
after a quarter of an hour without dictation:

| Language | Model | Size |
|---|---|---|
| German | [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline), a German fine-tune of NVIDIA Parakeet; also handles English | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 663 MB |

## Privacy

Audio never leaves your computer. The app connects to the internet only to
download the speech model and, when you click "Check for updates", to ask
GitHub for the newest version number. Nothing is downloaded or installed by
itself. No telemetry.

The models come from kyrr's own mirrors
([Hugging Face](https://huggingface.co/heast13/kyrr-models), else
[GitHub](https://github.com/heast13/kyrr-assets); the English model last
from the sherpa-onnx project's Hugging Face page), and every file is checked
against a SHA256 hash built into kyrr. Building kyrr downloads its assets
and libraries too, from kyrr's mirror on GitHub first.

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
  Face (or, if that fails, GitHub) and, when you click "Check for updates",
  asks GitHub for the newest version number. Their privacy policies apply
  to those requests: [Hugging Face](https://huggingface.co/privacy),
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

The cores (`kyrr-denoise`, `kyrr-voice`, `kyrr-dictate`) build and test on
Linux too; `cargo run -p kyrr --example preview -- <dir>` renders the window
headless.

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
- [Slint](https://slint.dev) for the UI toolkit,

and to the many more whose work kyrr builds on.
