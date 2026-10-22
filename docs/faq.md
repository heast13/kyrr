# Questions

## Does kyrr work in Teams, Zoom, Discord and the like?

Yes, in every app. kyrr cleans the microphone itself, so there is nothing to
choose in the app: it just gets the cleaner sound. On Linux this works for
every app that uses the default microphone.

## Does anything leave my computer?

No. Audio, dictated text and voice memos stay on your PC. kyrr goes online
only for the speech model, the update check and an update you start
yourself; see [Privacy](../README.md#privacy).

## How does kyrr compare with NVIDIA Broadcast and Krisp?

| | kyrr | NVIDIA Broadcast | Krisp |
|---|---|---|---|
| Price | free, open source | free | free for 60 minutes a day, subscription beyond |
| Needs | any 64-bit Intel or AMD PC with Windows 10 or 11, or Linux with PipeWire | an NVIDIA RTX graphics card | an account |
| Where it works | on the microphone itself: every app, nothing to choose | apps set to its virtual microphone | apps set to its virtual microphone |

On top, kyrr dictates into any app, on your computer. As far as we know in
October 2026; corrections welcome.

## Which languages can I dictate in?

German, English and more European languages (French, Spanish, Italian,
Dutch, Polish, Portuguese and others). Pick the language in the first-start
setup or later in Setup. The models are listed in [How kyrr
works](how-it-works.md#dictation).

## Why does Windows warn about the installer?

Because it is not code-signed yet. Click "More info", then "Run anyway". Why
it is not signed and how to check the file: [code signing
policy](../README.md#code-signing-policy).

## Dictation does nothing in one window on Windows

That window probably runs as administrator. Windows keeps normal programs
from seeing the keyboard there, so the dictation key does not reach kyrr.

## Does kyrr run on Windows on ARM?

Only dictation. The noise filter is built for Intel and AMD processors, and
the audio engine on ARM cannot load it.

## The dictation key does not work on my Linux desktop

Each desktop handles keys differently; kyrr sets them up for GNOME, KDE,
Hyprland, Sway, river, niri and X11 desktops. What happens on yours:
[desktop notes](install.md#desktop-notes).

## How do I update?

On Windows, kyrr says when there is a new version and installs it when you
click "Update". On Linux, paste the install commands again; see [Start at
sign-in and updates](install.md#start-at-sign-in-and-updates).

## How do I uninstall?

On Windows, like any other app in the Windows settings. On Linux, see
[Uninstall on Linux](install.md#uninstall-on-linux).
