# Install, update, uninstall (Sonnet)

Input: `crates/kyrr-app/src/update.rs`, `installer/`, `scripts/package*.py`,
`.github/workflows/release.yml`, omarchy-kyrr `update.sh` and `Panel.qml`,
docs/install.md, `git diff vW..HEAD`.

Think through each path from the user's side, with the previous version W
installed, and say for each whether it works, citing the code:

1. Windows: the update button downloads, checks the SHA256, installs and
   restarts kyrr; settings, voice memos and models survive; the filter (APO)
   keeps working; failure midway leaves the old kyrr working.
2. Windows fresh install and uninstall (nothing left that the installer put
   there, user data kept or asked).
3. Linux AppImage: how the user learns of the update and gets it (kyrr cannot
   replace itself on Linux: which path does the UI offer?); autostart entry
   and desktop file still point at the right file after an update.
4. Omarchy: the update row updates kyrr AND the plugin AND restarts the bar
   in one click, with no terminal command; a plugin-only update reaches the
   user; a failed step says so and leaves a working state; a kyrr newer than
   the plugin (or older) is handled (pages hidden, update offered).
5. Config migration: a config file of W loads in this version (new fields
   default sensibly, renamed fields carried over); downgrade does not crash.
6. Checksums and URLs: every download is verified before it runs; names in
   the code match what `release.yml` uploads.
7. First run after update: no setup again, What's new shows this version's
   NEWS once, on every system.
