# Texts, both languages (Haiku)

Input, all from the pinned run (`<out>`): `release-gate.txt` (the output of
`release-gate.py --release` for that commit); `git diff W..<sha>` in `<out>/src`
of `crates/kyrr-app/ui/app.slint` and `crates/kyrr-app/ui/lang/de/LC_MESSAGES/kyrr.po`;
the plugin's `Texts.js` in `<out>/plugin`; NEWS.md, README.md, `site/index.html`,
`site/de/index.html`.

1. release-gate.txt: no FAIL; every WARN looked at and judged in the report.
   Its checks on names, numbers and words:
   - `window-names` (FAIL): a window title quoted in the other language.
   - `news-linux-view` (FAIL): a line that Linux shows says "Windows" (the
     plural of Fenster is Fenster; mark the item (Windows) or reword it).
   - `quoted-names` (WARN): a quoted control name that is no UI text.
   - `number-format` (WARN): 125% in English texts, 125 % in German ones.
   - `words-diktat` (WARN): a Diktat compound such as Diktatfunktion. The bare
     noun "Diktat" is no longer reported (a dictated text is called that too),
     so read the feature names yourself: Diktierfunktion, not Diktat.
   - Hosts listed as "not reachable through the cloud proxy" are not a FAIL:
     write them as WARN, see README-crew.md.
2. Every new or changed text, German and English: correct spelling and
   grammar, same meaning in both languages, says what the user gets, short.
3. Flo's rules (see README-crew.md): no paths, commands or protocol names in
   app texts; the German terms; no Flatpak; the word-creation line and the
   sayings stay playful, everything else factual.
4. Same thing, same word: window, bar menu, website, README and NEWS name a
   feature the same way (e.g. "Stimme isolieren" everywhere).
5. Numbers and units: decimal comma in German, the same units as the UI.
6. New texts fit: check the longest German ones against the layout pictures
   in `<out>/layout/`.
