# Changelog

## 0.5.2 (2026-10-14)

- A microphone Windows has muted is said on the main page ("Your mic is muted
  in Windows.") with a "Switch it on" link that lifts the mute, and dictation
  tells why nothing was written.
- No microphone at all: "No microphone found. kyrr keeps looking."; a newly
  plugged one is named in green for ten seconds ("Microphone connected: ...").
- The main page has a "Quit" button, bottom left; "Off to the corner" moved to
  the right.
- New defaults: S sounds softened "light" and automatic volume on. The main
  page's voice card is called "Improve voice".
- "What's new" opens by itself after an update and, under Settings, About,
  at any time, listing every version from the newest down. The text is built
  into the app (NEWS.md), so it also works when you install by hand.

## 0.5.1 (2026-10-14)

- Plugging another microphone in or out is followed by itself: kyrr checks
  every few seconds which microphone Windows uses and moves the noise filter
  there (Windows asks for permission once, then sound pauses for a moment).
  A microphone on another USB port counts as a new one. If you decline, a
  button on the main page offers it. A microphone you pick by hand with
  "Change microphone" stays while Windows' default stays as it was, and so
  does one kyrr was already set up on.
- If Windows takes kyrr off the microphone (an update, most likely), kyrr
  puts itself back once by itself; the "Repair" button stays as the fallback.
- The microphone list in the window stays current while it is open.
- Microphones Windows had not given a "supported modes" entry (seen on a
  RØDE PodMic USB) now get the noise filter too: kyrr adds the entry when it
  attaches. A microphone attached earlier without it is repaired by itself.

## 0.5.0 (2026-10-13)

- The main window keeps its height; it cannot be pulled taller or shorter
  any more. Settings stay resizable.
- A small red note at the bottom of the main page: when your microphone is
  too quiet or too loud for the automatic volume to fix (turn it up or down
  in Windows), or when "Listen to yourself" is still running. Several notes
  take turns, one line at a time. The listen button is red while it runs.
- Automatic volume, advanced view: "Right now: +9 dB" in green when it
  raises your voice, "−9 dB" in red when it lowers it.
- The advanced settings use the room better: the automatic volume sits in
  the right column, and the window grows up to 1250 pixels where the screen
  has room, so everything shows without scrolling.
- "What's new": an update comes with a short summary in plain words, in the
  update note's new button. kyrr reads it from the answer of its daily
  update check, so there is no extra connection. It shows from the update
  after this one on.

## 0.4.0 (2026-10-11)

- Automatic volume, in Settings below Voice: brings your voice to the same
  level however your microphone is set, with or without the voice shaping.
  It listens only while you speak and moves slowly, so pauses, clicks and a
  fan leave it alone. Off by default; the advanced view shows what it evens
  out right now.
- Pop protection: softens the thump of P and B when you are close to the
  mic, without touching the voice. Off by default, under Settings, Advanced.
- Settings come in Simple and Advanced. Simple shows the basics: filter
  strength, sound, automatic volume, dictation, the check, language,
  autostart and the microphone. Advanced adds the model, the custom sound,
  the compressor (formerly "Even out volume"), S sounds, low cut, pop
  protection and checking for updates; a new version still shows in the
  main window. The credits fold away behind a button in Simple.
- A long microphone name wraps instead of being cut off; shorter texts.
- From 0.3.1, the update installs with one click from kyrr itself.

## 0.3.1 (2026-10-11)

- Updates in one click. Once a day kyrr asks GitHub for the newest version
  number (Settings, General, "Look for updates"; switched off, kyrr only
  asks when you click "Check for updates"). A new version shows as a small
  green note at the bottom of the main window and in the tray menu.
  "Update" downloads it, checks it against the release's SHA256 and
  installs it with only a progress window: Windows asks for permission
  once, and kyrr restarts by itself. The note's × hides it until the next
  version.
- The settings page shows whole where the screen has room (up to 1100
  pixels tall instead of 900), without scrolling.
- This works from 0.3.1 on: coming from 0.3.0, install this version by hand
  once more.

## 0.3.0 (2026-10-11)

- Listen to yourself: a new button under Check plays your microphone live
  on your headphones, the way your apps hear it.
- The before/after test only records now; play the take with Before and
  After whenever you like, and delete it with Delete recording.
- The settings show in two columns side by side, in a wider window that
  grows up to 900 pixels tall where the screen has room and moves instead
  of hiding behind the taskbar. The main view stays narrow.
- Even out volume starts at Medium. Who never changed it gets Medium too.
- With noise removal on, the main view says "Just your voice." instead of
  the microphone's name.
- The button at the bottom of the window only puts it away, like the close
  button. Quit kyrr from the tray icon's menu (right-click).
- Opening kyrr from the tray always brings its window to the front, at the
  size its content needs. It sometimes opened behind the current window,
  or too small after starting with Windows.
- Windows on ARM (Snapdragon and other ARM processors): kyrr no longer sets
  up its noise filter there, which the audio engine cannot load and which
  could leave the microphone silent. Dictation works; installing this
  version also removes the filter an earlier one put there.
- The speech models download from kyrr's own mirrors, on Hugging Face and,
  if that fails, on GitHub; the English model no longer depends on someone
  else's repository (that stays the last resort). Each file is still
  checked against its built-in SHA256.
- The level meter only runs while the window is open.

## 0.2.1 (2026-10-10)

- Dictation in German works on older processors too (Intel before
  Ice Lake/Alder Lake, AMD before Zen 4). On these it typed nothing or a
  stray word. kyrr fetches the corrected speech model by itself (about
  900 MB) and deletes the old one.
- The before/after test records ten seconds instead of five.
- Dictated text is pasted instead of typed key by key, so it arrives whole
  in every app (Windows 11 Notepad dropped words), and "new line" makes a
  new line in chat apps instead of sending the message. The clipboard is put
  back afterwards, and the dictated text stays out of the clipboard history.
- Line breaks also with "nächste Zeile", "nächster Absatz", "Zeilenumbruch",
  "next line", "next paragraph", "newline" and "line break".
- The installer thanks the projects kyrr builds on, on its last page and in
  the NOTICE file next to the app.

## 0.2.0 (2026-10-10)

First public release.

- Remove background noise: DPDFNet inside the Windows audio engine, for every
  app; strength gentle/medium/strong, a large and a small model.
- Voice: neutral, warm, clear, bright or your own sound, even volume, softer
  S sounds, low cut.
- Dictation: hold F9, speak, release; German and English, on your computer.
- Check: live level and a five-second before/after test.
- Check for updates, only when you click it.

Thanks to Fovty (HushMic), Ceva (DPDFNet), primeLine AI (German model),
NVIDIA (Parakeet) and the sherpa-onnx project; see the README.
