# Changelog

## Unreleased

- Listen to yourself: a new button under Check plays your microphone live
  on your headphones, the way your apps hear it.
- The before/after test only records now; play the take with Before and
  After whenever you like, and delete it with Delete recording.
- Even out volume starts at Medium. Who never changed it gets Medium too.
- The window opened from the tray always fits its content. Started with
  Windows, it used to come up too small.
- The button at the bottom of the window only puts it away, like the close
  button. Quit kyrr from the tray icon's menu (right-click).
- The settings get more room: the window grows up to 900 pixels where the
  screen has space, and moves up instead of hiding behind the taskbar.
- The level meter only runs while the window is open.
- Windows on ARM (Snapdragon and other ARM processors): kyrr no longer sets
  up its noise filter there, which the audio engine cannot load and which
  could leave the microphone silent. Dictation works; installing this
  version also removes the filter an earlier one put there.

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
