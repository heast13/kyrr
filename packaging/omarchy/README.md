# Omarchy

What kyrr brings for [Omarchy](https://omarchy.org) 4.

- **Dictation program.** On Omarchy 4.5 and newer kyrr adds itself to Setup >
  Defaults > Dictation (`crates/kyrr-linux/src/omarchy.rs`). Omarchy's own
  dictation keys then use kyrr. kyrr writes `omarchy-dictation-kyrr` and
  `omarchy-install-dictation-kyrr` to `~/.local/bin` and a block between
  `>>> kyrr` and `<<< kyrr` to `~/.config/omarchy/extensions/omarchy-menu.jsonc`.
- **Look.** kyrr's window and dictation pill take the Omarchy theme in use:
  its colours, square corners and its monospace font
  (`crates/kyrr-app/src/skin.rs`).
- **Bar widget.** `plugin/` is a plugin for Omarchy's bar. Omarchy installs
  plugins from a git repository with `manifest.json` at its root, so it is
  published as its own repository (`heast13/omarchy-kyrr`): copy the folder's
  files there, check them with `omarchy plugin validate`, and list the
  repository at [plugins.omarchy.org](https://plugins.omarchy.org) (an issue
  form: link, category "Audio", tags).
