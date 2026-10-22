# Installing kyrr

The short version is in the [README](../README.md#install). This page has
the requirements, notes for each Linux desktop, updates and uninstalling.

- [Windows](#windows)
- [Linux](#linux)
  - [Requirements](#requirements)
  - [Desktop notes](#desktop-notes)
  - [Start at sign-in and updates](#start-at-sign-in-and-updates)
  - [Uninstall on Linux](#uninstall-on-linux)

## Windows

Windows 10 22H2 or Windows 11, 64-bit, on an Intel or AMD processor.

1. Download `kyrr-setup-<version>.exe` from the [latest
   release](https://github.com/heast13/kyrr/releases/latest).
2. Run it. The installer needs administrator rights, and sound drops out for
   a second while kyrr sets up its filter (what it changes is described in
   [How kyrr works](how-it-works.md#noise-filter)).
3. Until the installer is code-signed, Windows SmartScreen warns about it
   ("More info", "Run anyway"). To check the file first, compare it with its
   `.sha256` file from the release page (see the [code signing
   policy](../README.md#code-signing-policy)).

On Windows on ARM (Snapdragon and other ARM processors) kyrr only dictates:
its noise filter is built for Intel and AMD processors, and the audio engine
there cannot load it.

**Uninstall** kyrr like any other app in the Windows settings. Uninstalling
takes the filter off the microphone, puts the Windows setting
`DisableProtectedAudioDG` back and asks whether to delete the downloaded
speech models.

## Linux

The Linux version works, but has been tested on fewer systems than the
Windows version. One file, an AppImage, for every distribution.

**With a terminal.** Paste this; it downloads kyrr, makes it executable and
starts it:

```sh
mkdir -p ~/.local/bin
curl -fL -o ~/.local/bin/kyrr.new https://github.com/heast13/kyrr/releases/latest/download/kyrr-x86_64.AppImage && chmod +x ~/.local/bin/kyrr.new && mv -f ~/.local/bin/kyrr.new ~/.local/bin/kyrr
~/.local/bin/kyrr --replace &
```

**Prefer clicking?**

1. Download `kyrr-x86_64.AppImage` from the [latest
   release](https://github.com/heast13/kyrr/releases/latest).
2. Allow executing it as a program in your file manager (right click,
   Properties).
3. Double click it.

Either way kyrr adds itself to your app menu on its first start.

### Requirements

- 64-bit Intel or AMD with glibc 2.39 or newer (Ubuntu 24.04, Fedora 40,
  Debian 13, Linux Mint 22, Arch or newer).
- PipeWire for the noise filter, which these use anyway; where it is
  missing, a button in kyrr installs it (your system asks for your password).
- A processor with AVX2 for the noise filter (most from 2015 on); dictation
  works without.
- FUSE 3 (the `fuse3` package), which AppImages need to start. Every
  desktop system has it; a bare server image may not.
- No administrator rights needed.

### Desktop notes

- **GNOME 48+ and KDE:** your desktop asks once for the dictation and voice
  memo keys and once whether kyrr may type; kyrr walks you through both.
- **GNOME 46 and 47:** kyrr adds the keys to GNOME's own keyboard shortcuts,
  where each press starts or ends dictation (or a voice memo).
- **Hyprland (Omarchy, say), Sway, river and niri:** kyrr adds the keys to
  your key bindings itself, in a block of its own that it keeps up to date,
  and the start at sign-in too where the session does not start XDG
  autostart entries.
- **X11 desktops (Linux Mint, XFCE):** the keys just work.
- **Without a tray icon** the window's X minimizes kyrr where the desktop can
  bring it back (GNOME, say) and quits it on tiling ones; "Quit" always ends
  it.

### Start at sign-in and updates

**Start at sign-in** switches on by itself when the first-start setup is
done. Switch it off or on again in Setup, "Start when you sign in".

**Updates** are not installed by themselves: kyrr says when there is a new
version. Paste the install commands again: a running kyrr closes, the file is
replaced and the new version starts.

### Uninstall on Linux

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
and `<<< kyrr` lines (keys and start at sign-in) from your config; on GNOME,
delete the "kyrr" and "kyrr: voice memo" entries under Custom Shortcuts in
the keyboard settings. Your voice memos stay in Documents/kyrr.
