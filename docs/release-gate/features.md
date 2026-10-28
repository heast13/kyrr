# New features on every system (Haiku)

Input: CHANGELOG.md section of this version, NEWS.md section, `git log vW..HEAD`,
the plugin's `git log` since kyrr's previous release.

1. **Feature list.** Every user-visible change of this version, from CHANGELOG,
   NEWS and the commits (a commit that changes behaviour but is in neither is
   a FAIL: the notes miss it).
2. **Matrix.** For each feature one row: Windows / Linux / Omarchy menu, each
   `done` (file:line where it is wired), `n/a` (with the written reason) or
   `missing` (FAIL). Use the window's UI in `crates/kyrr-app/ui/app.slint`,
   platform code in `crates/kyrr-windows`, `crates/kyrr-linux`,
   `crates/kyrr-app/src/platform.rs`, and the menu in omarchy-kyrr
   (`Views.js`, `Panel.qml`).
3. **Same behaviour.** Defaults, texts and limits are the same on every
   system (e.g. a default that is on for Windows and off for Linux is a FAIL
   unless the code says why).
4. **Tested.** Each feature has an automated test (name it) or is on the
   real-device list. A feature with neither is a FAIL.
5. **Real-device list for Flo.** Write it in German at the end of your report:
   per system (Windows 11, Linux GNOME/KDE, Omarchy), short steps a person
   can follow without knowing the code, each with what should happen. Always
   include the fixed items of `docs/release-gate.md` section 4, then one block
   per new feature. Mark items that only this release needs.
