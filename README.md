# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

A small, free Windows app that does two things well:

- **Clean microphone.** Real-time neural noise suppression
  ([DPDFNet](https://github.com/ceva-ip/DPDFNet)) on your microphone, for
  every app: Teams, Zoom, Discord, OBS, the browser.
- **Dictation.** Hold F9, speak, release. The text appears where your cursor
  is. German and English, on your computer, no account, no cloud.

Status: early development, not released yet.

## How it works

**Clean microphone.** kyrr installs an audio processing object (APO), a
filter the Windows audio engine runs on the microphone itself, the way
Equalizer APO does. Every app gets the cleaned signal without choosing a
virtual device. If the microphone's driver has its own effect (for example
beamforming), kyrr keeps it and runs after it. Latency is 60 ms at 48 kHz.

Windows only loads audio filters signed by Microsoft unless the setting
`DisableProtectedAudioDG` is on. kyrr's installer switches it on, as
Equalizer APO's does, says so before it starts, and restores the previous
value on uninstall.

**Dictation.** kyrr records while F9 is held and transcribes locally with
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). The speech model is
downloaded once on first use:

| Language | Model | Size |
|---|---|---|
| German | [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline), a German fine-tune of NVIDIA Parakeet; also handles English | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 660 MB |

## Privacy

Audio never leaves your computer. kyrr makes network connections only to
download the speech model, from Hugging Face, with every file checked
against a SHA256 hash built into kyrr.

## Building

Windows with Rust (MSVC) and Python 3; for the installer Inno Setup 6.

```powershell
python scripts/fetch-assets.py --windows
cargo build --release -p kyrr -p kyrr-apo
python scripts/package.py
```

The cores (`kyrr-denoise`, `kyrr-dictate`) build and test on Linux too;
`cargo run -p kyrr --example preview -- <dir>` renders the window headless.

## License

kyrr is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Third-party components and models keep their own licenses,
see [THIRD_PARTY.md](THIRD_PARTY.md). Made with [Slint](https://slint.dev).
