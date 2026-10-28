# Texts, both languages (Haiku)

Input: `python3 scripts/release-gate.py` output, `git diff vW..HEAD` of
`crates/kyrr-app/ui/app.slint`, `crates/kyrr-app/ui/lang/de/LC_MESSAGES/kyrr.po`,
the plugin's `Texts.js`, NEWS.md, README.md, `site/index.html`, `site/de/index.html`.

1. release-gate.py: no FAIL; every WARN looked at and judged in the report.
2. Every new or changed text, German and English: correct spelling and
   grammar, same meaning in both languages, says what the user gets, short.
3. Flo's rules (see README-crew.md): no paths, commands or protocol names in
   app texts; the German terms; no Flatpak; the word-creation line and the
   sayings stay playful, everything else factual.
4. Same thing, same word: window, bar menu, website, README and NEWS name a
   feature the same way (e.g. "Stimme isolieren" everywhere).
5. Numbers and units: decimal comma in German, the same units as the UI.
6. New texts fit: check the longest German ones against the layout pictures.
