# How kyrr works

The details behind each part of the app. For installing, see
[install.md](install.md); for short answers, the [FAQ](faq.md).

- [Noise filter](#noise-filter)
- [Improve voice](#improve-voice)
- [Your microphone](#your-microphone)
- [Check](#check)
- [Dictation](#dictation)
- [Voice memos](#voice-memos)
- [Keys](#keys)

<p align="center">
  <img src="screenshots/settings.png" width="760" alt="The pages the gears open: isolate your voice (filter strength), dictation (key and how it works) and improve voice (sound with sliders, compressor, S sounds, low cut, pop protection, distance compensation, automatic volume)">
</p>

## Noise filter

kyrr removes noise with [DPDFNet](https://github.com/ceva-ip/DPDFNet), a
neural network that runs in real time on your processor. A large model gives
the cleanest result, a small one suits older laptops. Latency is 60 ms at
48 kHz. On processors without AVX2 the filter falls back to ONNX Runtime and
kyrr switches to the small model once.

**Windows.** kyrr installs an audio processing object (APO), a filter the
Windows audio engine runs on the microphone itself, the way Equalizer APO
does. Every app gets the cleaned signal without choosing a virtual device. If
the microphone's driver has its own effect (for example beamforming), kyrr
keeps it and runs after it.

Windows only loads audio filters signed by Microsoft unless the setting
`DisableProtectedAudioDG` is on. kyrr's installer switches it on, as
Equalizer APO's does, says so before it starts, and restores the previous
value on uninstall (unless Equalizer APO is installed and still needs it).
If a Windows or driver update takes kyrr off the microphone, the app notices
and offers to repair it.

**Linux.** kyrr adds a PipeWire microphone of its own, called "kyrr": it
reads your real microphone, removes the noise and becomes the system's
default microphone, so every app that uses the default gets the cleaned
signal. kyrr only reads your real microphone while an app listens to "kyrr".
When kyrr quits, its microphone goes away and the system falls back to your
real one.

## Improve voice

The same filter can shape the voice. Pick a sound (Neutral, Warm, Clear,
Bright or your own); behind it are a low cut (80 Hz, 100 Hz for the bright
sound) before the noise filter, then three EQ bands (body at 250 Hz, presence
at 3 kHz, air above 8 kHz), a de-esser on the band above 4 kHz, a compressor
(light, medium or strong) and a limiter at −1 dBFS. It works with the noise
filter switched off too, and adds no latency. On by default with a neutral
sound, medium compression, light S-sound softening and the low cut; pop
protection is off by default. Reset brings back the defaults.

**Pop protection** listens to the raw microphone for the thump of a P or B
and turns everything below 120 Hz down while it lasts, starting 10 ms before
the thump arrives.

**Automatic volume** (on by default, works with the voice shaping switched
off too) measures the voice only while someone speaks and moves it slowly
(5 dB/s up, 10 dB/s down, at most +24/−12 dB) towards about −23 dBFS;
pauses, clicks and steady noise leave it alone. Apps that adjust the volume
themselves (call apps, say) can work against it, so use one of the two.

**Distance compensation** (on by default, switch it off under the voice gear)
helps when you sit far from the microphone and the room echoes on every
word. It listens to the raw microphone: after a syllable a close voice falls
20 dB and more within 40 ms, while far away no level falls faster than the
room's echo dies down. After 5 to 15 seconds of speech that tells near from
far. Only when far, it fades in over two seconds: the echo tail after each
syllable goes down by up to 8 dB in four bands, a little bass comes back
(+2.5 dB below 150 Hz), some box goes (−2.5 dB at 450 Hz) and a touch of
presence is added (+1.5 dB at 3.5 kHz), all before the automatic volume.
Close to the microphone the audio stays the same bit for bit. While it is on
and the microphone sounds far, the main page also suggests moving a little
closer.

## Your microphone

kyrr checks every few seconds which microphone the system uses. When it
changes, the filter moves along: Windows asks for permission once and sound
pauses for a moment; on Linux it simply moves. If that does not happen (you
declined, say), a button on the main page offers it. A microphone you pick by
hand stays until the system's default changes.

kyrr also tells you what is wrong: a muted microphone (with a button to
switch it back on), none found, or one that just arrived.

## Check

The check page shows a live level of what the microphone hears and what apps
get, records a ten-second test (played back before and after kyrr) and lets
you listen to yourself live on headphones.

On Windows the filter shares a small block of memory with the app
(`Global\kyrr-monitor-3`): the level before and after kyrr, what the
automatic volume adds, whether the microphone sounds far, and on request one
ten-second take, unprocessed and processed. The audio service writes it;
users signed in at this computer can read it and ask for a take, which only
records while an app already uses the microphone. The app plays the take,
keeps it only in memory and clears it from the shared block.

On Linux the filter runs inside the app, which reads the levels and the take
directly.

## Dictation

Hold F9 (or a key of your own, or tap to start and stop), speak, release: the
text appears where your cursor is. Say "new line" or "neue Zeile" for a line
break. While you dictate, a small wave at the bottom of the screen shows that
kyrr is listening, with the words heard so far (switch "Live text" off in
Setup), and then that it is writing.

kyrr records while the key is held (up to 15 minutes) and transcribes locally
with [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (its build without
text-to-speech). The speech model is downloaded once on first use and let go
of after a quarter of an hour without dictation:

| Language | Model | Size |
|---|---|---|
| German and Multilingual (Language choice in the first-start setup and in Setup) | [moondream/parakeet-ultra](https://huggingface.co/moondream/parakeet-ultra), a further training of NVIDIA Parakeet v3 for 25 European languages. Both choices use the same files (one download); "Multilingual" picks the language itself. Works in French, Spanish, Polish, Portuguese, Italian and Dutch as well as German and English | 900 MB |
| English | [NVIDIA parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b) | 663 MB |

On Windows dictation does nothing while a window running as administrator is
in front: Windows keeps normal programs from seeing the keyboard there.

## Voice memos

The memo key (F8, or a key of your own; hold it while you talk, or tap it to
start and again to end, as you like) starts and ends a recording, as does the
tray menu. Where the desktop does not report a key being let go (GNOME 46 and
47, niri), it is tap only. Voice memos can be switched off on the main page.

kyrr records the microphone as dictation hears it (with the noise filter when
it is on) as Opus (mono, 24 kbit/s, about 11 MB an hour) for up to four
hours, into `Documents/kyrr` (the folder Windows or your desktop calls
Documents, wherever it is). The file is called
`2026-10-21 14-05 Title.opus`: when the recording ends, the dictation model
listens to its first seconds and the first real sentence becomes the title.

The list plays, renames, shows in the folder or deletes memos. A dot beside
"Recordings" on the main page shows a recording you have not looked at yet.

"Write text" transcribes the whole memo in pieces of half a minute, cut at a
pause, and saves the text beside it as `… Title.txt`, one paragraph per piece
with where it starts and when it was said (`[1:30 · 14:06] …`). The text
opens in a field: select and copy as you like (the times never go along),
correct it (it saves itself, in the file too), show the times as position,
time of day or not at all, and "Play from here" plays from the paragraph the
cursor is in. You can edit the `.txt` by hand as well; kyrr reads it again
when you open the text.

Dictation comes first: while you dictate, the text waits. The memo key does
nothing while you dictate, and the dictation key nothing while a memo
records.

On Linux the memo key is set up like the dictation key: GNOME 48 and KDE ask
for both keys in one dialog, GNOME 46 and 47 get a second custom shortcut,
and on Hyprland, Sway, river and niri the key goes into kyrr's block in the
config.

## Keys

Dictation (F9) and voice memos (F8) each have a field "Set key" next to the
standard key: click in, press a key, done. One key each, never the same for
both. Offered are F1 to F24, Pause, Scroll Lock, the menu key and right Ctrl;
keys programs need for typing (letters, Space, Enter, arrows, Shift, Alt,
left Ctrl, Windows) are turned down.

kyrr takes the key from every program while it runs, except with Ctrl, Alt,
Shift or Windows held; that is why F5, F11 and the like come with a warning,
and why right Ctrl costs the shortcuts that use it. Where the desktop owns
the key (GNOME 48 and KDE) the field is replaced by a button to the desktop's
own shortcut settings. Holding works wherever the desktop reports the
release: not on GNOME 46 and 47 (a custom shortcut only reports the press)
and not on niri, where each press toggles instead.
