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
  listed at [plugins.omarchy.org](https://plugins.omarchy.org). Its menu is
  all of kyrr on Omarchy: switches, settings, recordings and the first-run
  setup, walked with the keyboard; it installs kyrr when it is missing.
  While the plugin is installed and turned on (`~/.config/omarchy/shell.json`
  names `io.github.heast13.kyrr`), kyrr runs without a window and without a tray icon:
  a start from the app menu opens the widget's menu instead, and kyrr quits
  once the widget is gone (the plugin turned off). kyrr's side: the state
  file `$XDG_RUNTIME_DIR/kyrr/state` (one line of JSON, `tell_widget` in
  `crates/kyrr-app/src/main.rs`), command lines on the socket
  (`crates/kyrr-linux/src/instance.rs`, carried out by `omarchy_command`),
  and `$XDG_RUNTIME_DIR/kyrr-widget`, which the widget renews while it is in
  the bar. Both formats are in the widget's README.
