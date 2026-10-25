# Omarchy

What kyrr brings for [Omarchy](https://omarchy.org) 4.

- **Dictation program.** On Omarchy 4.5 and newer kyrr adds itself to Setup >
  Defaults > Dictation (`crates/kyrr-linux/src/omarchy.rs`). Omarchy's own
  dictation keys then use kyrr. kyrr writes `omarchy-dictation-kyrr` and
  `omarchy-install-dictation-kyrr` to `~/.local/bin` and a block between
  `>>> kyrr` and `<<< kyrr` to `~/.config/omarchy/extensions/omarchy-menu.jsonc`.
  The first script hands each key press to the running kyrr over its socket
  with `socat`, which Omarchy installs; without it each press starts kyrr,
  which passes it on (slower).
- **Look.** kyrr's window and dictation pill take the Omarchy theme in use:
  its colours, square corners and its monospace font
  (`crates/kyrr-app/src/skin.rs`).
- **Bar widget.** Its own repository,
  [heast13/omarchy-kyrr](https://github.com/heast13/omarchy-kyrr) (Omarchy
  installs plugins from a git repository with `manifest.json` at its root),
  listed at [plugins.omarchy.org](https://plugins.omarchy.org). It shows what
  kyrr does and has kyrr's tray menu; it installs kyrr when it is missing.
  kyrr's side: the state file `$XDG_RUNTIME_DIR/kyrr/state` (one line of
  JSON), the socket words `toggle-filter`, `toggle-voice`,
  `toggle-dictation`, `toggle-memo`, `open-memos`, `install-update` and `quit`
  (`crates/kyrr-linux/src/instance.rs`), and `$XDG_RUNTIME_DIR/kyrr-widget`,
  which the widget renews while it is in the bar: then kyrr hides its tray
  icon and the window's buttons to hide or quit.
