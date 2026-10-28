# Versions and release notes (Haiku)

Input: the release PR.

1. `python3 scripts/release-gate.py --release --online --plugin <omarchy-kyrr>`
   has no FAIL (paste its output).
2. NEWS.md of this version: only what users notice, plain sentences, the
   `(Windows)`/`(Linux)` marks right (an item marked for one system really is
   only there), German matches English, each block under about 2000
   characters, never a line saying nothing changes on Windows.
3. CHANGELOG.md of this version lists every change of `git log vW..HEAD`
   that a user or packager notices; dated today or the planned day.
4. `python3 scripts/release-notes.py` (read it for its arguments) renders
   the release description without error; the hidden What's-new blocks are
   in it.
5. metainfo: the new `<release>` with description and details URL; AppStream
   rules (`appstreamcli validate` if available, else by reading).
6. Installer (`installer/`), winget manifests (`packaging/winget`) and AUR
   (`packaging/aur`) take the version from the build or are bumped by their
   workflows; nothing hard-coded to the old version.
