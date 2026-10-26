# kyrr

*kyrr* (Old Norse): still, quiet, at rest.

A free app for your microphone on **Windows** and **Linux**. It removes
background noise, makes your voice sound better and turns speech into text.
Everything runs on your own computer: your voice and text never leave it,
and there is no account.

<p>
  <a href="https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe"><img src="https://img.shields.io/github/v/release/heast13/kyrr?label=Download%20for%20Windows&style=for-the-badge&color=005fb8" alt="Download kyrr for Windows"></a>
  <a href="#linux"><img src="https://img.shields.io/badge/Install%20on%20Linux-beta-005fb8?style=for-the-badge" alt="Install kyrr on Linux (beta)"></a>
</p>

[Website](https://heast13.github.io/kyrr/) (auf [Deutsch](https://heast13.github.io/kyrr/de/))
· [Questions](docs/faq.md)
· [How it works](docs/how-it-works.md)
· [What's new](https://github.com/heast13/kyrr/releases/latest)

<a href="https://slint.dev"><img src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png" height="36" alt="Made with Slint"></a>

<p align="center">
  <img src="docs/screenshots/main.png" width="380" alt="kyrr's window: noise filter, improve voice and dictation, each with one switch and a gear for its settings">
</p>

## What it does

- **Removes noise** from your microphone in real time: keyboard, fan, traffic,
  the dog. Works in every app (Teams, Zoom, Discord, OBS, the browser) with
  nothing to set up there.
- **Improves your voice**: a sound of your choice, even volume and softer
  S sounds. It also helps when you sit far from the microphone.
- **Dictation**: hold F9, speak, let go. The text appears where your cursor
  is, in German, English and other European languages.
- **Voice memos**: hold F8 and talk. kyrr saves the recording in
  Documents/kyrr, names it after its first words and writes it out as text on
  request.
- **Follows your microphone**: plug in another one and kyrr moves along. A
  ten-second test lets you hear the difference.

More about each part in [How kyrr works](docs/how-it-works.md).

## Three versions

kyrr is the same app in three versions:

- **Windows:** the installer, see [Windows](#windows) below.
- **Linux:** one AppImage for most desktops (beta), see [Linux](#install-on-linux).
- **Omarchy:** a plugin for Omarchy's bar, see [Omarchy](#omarchy).

## Install

### Windows

Windows 10 22H2 or Windows 11, 64-bit.

1. [Download the installer](https://github.com/heast13/kyrr/releases/latest/download/kyrr-setup.exe)
   and run it. It needs administrator rights, and sound drops out for a
   second while kyrr sets up its filter ([what the installer
   changes](docs/how-it-works.md#noise-filter)).
2. Windows SmartScreen warns because the installer is not signed yet: click
   "More info", then "Run anyway" (see [code signing
   policy](#code-signing-policy)).

<a id="install-on-linux"></a>

### Linux

Beta: works, but tested on fewer systems than Windows. Paste this into a
terminal; it downloads kyrr and starts it:

```sh
mkdir -p ~/.local/bin
curl -fL -o ~/.local/bin/kyrr.new https://github.com/heast13/kyrr/releases/latest/download/kyrr-x86_64.AppImage && chmod +x ~/.local/bin/kyrr.new && mv -f ~/.local/bin/kyrr.new ~/.local/bin/kyrr
~/.local/bin/kyrr --replace &
```

Prefer clicking? [Download the
AppImage](https://github.com/heast13/kyrr/releases/latest/download/kyrr-x86_64.AppImage),
allow it to run as a program (right click, Properties) and double click it.

Needs a 64-bit Intel or AMD PC with Ubuntu 24.04, Fedora 40, Debian 13, Linux
Mint 22, Arch or newer, and PipeWire. Notes for GNOME, KDE, Hyprland and other
desktops, updates and uninstalling: [installation
details](docs/install.md).

### Omarchy

For Omarchy 4.0 or newer, kyrr is a plugin for the bar, with its menu in
Omarchy's own style and no window of its own. One command in a terminal:

```sh
omarchy plugin add https://github.com/heast13/omarchy-kyrr.git --enable
```

Then click the kyrr icon in the bar. The menu installs kyrr for you. Details
in [kyrr for Omarchy](https://github.com/heast13/omarchy-kyrr).

## Getting started

1. On first start, a short setup in three steps: set up the microphone, pick
   your language (the speech model, about 660 to 900 MB, downloads once; after that
   dictation works offline) and choose "Best quality" or "Lean" for older
   laptops.
2. In the main window each part has a switch, and a gear for its settings.
3. kyrr starts with your computer and waits in the tray (both can be
   switched off in Setup). Hold F9 in any app to dictate, F8 for a voice
   memo.

## Privacy

Your voice, your dictated text and your voice memos never leave your
computer. No telemetry. kyrr goes online for exactly three things:

- **The speech model**, once, from kyrr's own mirrors
  ([Hugging Face](https://huggingface.co/heast13/kyrr-models), else
  [GitHub](https://github.com/heast13/kyrr-assets); the English model last
  from the sherpa-onnx project's Hugging Face page). Every file is checked
  against a SHA256 hash built into kyrr.
- **The update check**, once a day: one request to the GitHub API for the
  newest version number and its short "What's new" text
  (`api.github.com/repos/heast13/kyrr/releases/latest`).
  It sends nothing about you beyond what any web request carries (your IP
  address and "kyrr/<version>"). Switch it off in Setup ("Check for updates
  automatically"); then kyrr only asks when you click "Check for updates".
- **An update**, only when you click "Update": the new installer from this
  repository's releases, checked against its SHA256 file. Where kyrr cannot
  install it itself (Linux, or a failed download), it opens the download page
  instead.

Nothing is installed without your click. Record other people only when they
agree to it.

## Code signing policy

kyrr's releases are not code-signed yet. A free certificate from the SignPath
Foundation was applied for and declined for now, because kyrr is not known
widely enough. That is why Windows SmartScreen warns about the installer.

To check a download, compare it with the `.sha256` file on the release page.
Every release is built on GitHub by the [release
workflow](.github/workflows/release.yml) from a version tag of this
repository. Third-party files keep their authors' signatures (ONNX Runtime
and the Visual C++ runtime by Microsoft); models are data.

## Feedback

In kyrr, Setup, About, "Feedback" opens an e-mail to
stueckig.habicht_2o@icloud.com with the kyrr version and your system filled
in (kyrr sends nothing by itself). Issues on GitHub work too. Security issues: see [SECURITY.md](SECURITY.md).

To build kyrr yourself, see [Building kyrr](docs/building.md).

## License

kyrr is licensed under either of [Apache 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. Third-party components and models keep
their own licenses, see [THIRD_PARTY.md](THIRD_PARTY.md); the app shows the
model credits under Setup, About. If you build on
kyrr, please keep the copyright notice and [NOTICE](NOTICE) and, if you
like, mention heast13 with a link to this repository.

kyrr's window is made with [Slint](https://slint.dev) under its royalty-free
license, which asks for the badge at the top of this page.

## Thanks

kyrr stands on the work of others. Thank you to

- [Fovty](https://github.com/Fovty) for HushMic, whose DPDFNet runtime
  kyrr's filter is built on,
- [Ceva](https://github.com/ceva-ip/DPDFNet) for DPDFNet and for releasing
  its models openly,
- [moondream](https://huggingface.co/moondream/parakeet-ultra) for the
  German and multilingual speech model,
- NVIDIA for Parakeet, the model family behind all dictation languages,
- csukuangfj and the [k2-fsa](https://github.com/k2-fsa/sherpa-onnx) team
  for sherpa-onnx and the English model in ONNX,
- [Equalizer APO](https://sourceforge.net/projects/equalizerapo/), which
  showed how a filter can live inside Windows audio,
- [Slint](https://slint.dev) for the UI toolkit.
