# ksni 0.3.6, patched

The crates.io release of [ksni](https://github.com/iovxw/ksni) 0.3.6
(Unlicense), with one change in `src/service.rs`: inside a Flatpak
(`/.flatpak-info` exists) the tray registers under its unique D-Bus name
instead of owning `org.kde.StatusNotifierItem-<pid>-<n>`. A Flatpak may only
own that name with `--own-name`, which Flathub no longer grants to new apps.
Waybar, KDE and the other StatusNotifier hosts accept the unique name (Qt and
Electron register the same way in a sandbox).

Slint 1.18 starts its tray with `spawn()` and leaves the name on. Slint's
`master` already calls `disable_dbus_name` in a sandbox
(`internal/core/items/system_tray/ksni.rs`). Remove this folder and the
`[patch.crates-io]` entry in the root `Cargo.toml` once kyrr uses a Slint
release with that change.

Also removed: the `[[example]]`, `[[test]]` and dev-dependency sections of the generated
`Cargo.toml` (the folders are not copied).
