# Changelog

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
