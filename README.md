# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
</p>

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

Noise suppression and dictation for Windows. Free, open source, and
entirely on your PC.

- **Noise suppression for every app.** kyrr filters the microphone itself,
  so Teams, Zoom, Discord, OBS and the browser all get a clean voice
  without any setup. To our ears on par with paid tools such as Krisp (a
  listening impression, not a formal benchmark); try the built-in
  before/after test.
- **Dictation in English and German.** Hold F9, speak, release: the text
  appears at the cursor, with punctuation. Accurate in English, and the
  first dictation we found that is reliable in German too.
- **Private.** No cloud, no account, no telemetry. After the one-time
  model download kyrr works offline.
- **Simple.** Three switches, one key. Everything else is optional.

<p align="center">
  <img src="docs/screenshots/main.png" width="380" alt="kyrr's window: noise filter, voice and dictation, each with one switch">
</p>
<p align="center">
  <img src="docs/screenshots/settings.png" width="760" alt="Settings in two columns: noise strength and model, voice sound, volume, S sounds, low cut, language and autostart; dictation key, level check with before/after test and listening, about">
</p>

## Features

| | |
|---|---|
| Noise suppression | Neural ([DPDFNet](https://github.com/ceva-ip/DPDFNet)), 60 ms latency, 5 to 11 % of one CPU core on a current desktop. A large and a small model, three strengths. |
| Voice | Sound presets (Neutral, Warm, Clear, Bright, custom EQ), compressor, de-esser, low cut. |
| Dictation | F9, F8, Pause or right Ctrl; hold or tap. A ten-second sentence is text in well under a second. "New line" and "neue Zeile" for line breaks. |
| Check | Live level, ten-second before/after recording, listening to yourself on headphones. |

## Compared with

| | kyrr | NVIDIA Broadcast | Krisp |
|---|---|---|---|
| Price | free, open source | free | subscription after a free trial |
| Needs | 64-bit Intel or AMD PC, Windows 10 or 11 | NVIDIA RTX graphics card | an account |
| Works in | every app, on the microphone itself | apps set to its virtual microphone | apps set to its virtual microphone |

kyrr also dictates into any app, on your PC. As far as we know in October
2026; corrections welcome.

## Install

Download and run the [installer](https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe).
It needs Windows 10 22H2 or 11 (64-bit) and administrator rights; sound
drops out for a second while the filter is set up.

- Until the installer is [code-signed](#code-signing-policy), SmartScreen
  warns about it ("More info", "Run anyway"). Each release has a `.sha256`
  file to check it against.
- On Windows on ARM, kyrr offers dictation only: the noise filter is built
  for Intel and AMD processors.
- Uninstalling removes the filter, restores the Windows setting below and
  offers to delete the speech models.

## How it works

**Noise suppression** runs as an audio processing object (APO) that the
Windows audio engine applies to the microphone, as Equalizer APO does. The
microphone driver's own effects stay in place, before kyrr. Loading an APO
not signed by Microsoft needs `DisableProtectedAudioDG`; the installer sets
it and uninstalling restores the previous value. If a Windows update removes
kyrr from the microphone, the app offers to repair it. Without AVX2, kyrr
uses ONNX Runtime and the small model.

**Voice** shaping runs in the same filter, after noise suppression, with no
added latency: low cut, three-band EQ, de-esser, compressor and limiter.

**Dictation** records while the key is held and transcribes locally with
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). The text is pasted at
the cursor and the clipboard restored. The model is downloaded on first use
and unloaded after 15 minutes without dictation.

| Language | Model | Download |
|---|---|---|
| German | [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline), a German fine-tune of NVIDIA Parakeet | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 663 MB |

## Privacy

Audio and text never leave your computer. kyrr connects to the internet only
to download a speech model and, when you click "Check for updates", to ask
GitHub for the latest version. Models come from kyrr's own mirrors
([Hugging Face](https://huggingface.co/heast13/kyrr-models), then
[GitHub](https://github.com/heast13/kyrr-assets); the English model last
from the sherpa-onnx project) and are checked against SHA256 hashes built
into kyrr. No telemetry.

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

`SHERPA_ONNX_LIB_DIR` selects sherpa-onnx's libraries without
text-to-speech (the default ones bundle GPL-3.0 code);
`scripts/check-binaries.py` rejects builds that contain it. The cores
(`kyrr-denoise`, `kyrr-voice`, `kyrr-dictate`) also build and test on Linux.

## License

MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)). Third-party components and models keep
their own licenses, see [THIRD_PARTY.md](THIRD_PARTY.md) and
[NOTICE](NOTICE). The UI is made with [Slint](https://slint.dev) under its
royalty-free license. Security issues: [SECURITY.md](SECURITY.md).

## Acknowledgements

kyrr builds on [DPDFNet](https://github.com/ceva-ip/DPDFNet) by Ceva,
[HushMic](https://github.com/Fovty/HushMic) by Fovty, NVIDIA's Parakeet,
[primeLine AI](https://huggingface.co/primeline/parakeet-primeline)'s German
model, [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) by csukuangfj and
the k2-fsa team, [Equalizer APO](https://sourceforge.net/projects/equalizerapo/),
[Slint](https://slint.dev) and many more. Thank you.
