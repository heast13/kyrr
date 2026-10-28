# Release gate

Everything that has to hold before a version is tagged, checked the same way
every time. Reviews by feel kept missing things (a bar menu without the new
features, an update that left the plugin behind, windows that did not scale,
pages that scrolled for nothing), so the gate is a fixed list: machines check
what machines can, a crew of reviewers checks the rest area by area, and Flo
tests on real devices what nothing else reaches. No tag without a green gate.

A feature counts as done only when it works, the same way, on Windows, on
Linux (GNOME, KDE and the like) and in the Omarchy bar menu, unless the
system forbids it; such exceptions are written down in
`packaging/omarchy/contract.json` (`windowOnly`) or in the feature's notes.

## 1. Automatic, on every pull request

CI (`.github/workflows/ci.yml`) on Windows and Linux, all required (one
exception, see Layout):

- `cargo fmt`, `cargo clippy -D warnings`, `cargo test` on the whole
  workspace, `scripts/check-binaries.py`.
- **Omarchy contract** (`scripts/check-omarchy-contract.py`): every command
  the socket takes, every control of the window and every key of the state
  file is listed in `packaging/omarchy/contract.json`, with the way the bar
  menu does the same. A new setting without its entry fails here.
- **Layout** (`crates/kyrr-app/examples/layout.rs`): every page in German and English at
  scale 1.0 to 2.0 on screens from 720 to 2160 px high. Fails on a window
  taller than the screen, a page that scrolls although the screen has room,
  a width that does not follow the scale, and content wider than the window.
  Its pictures are the CI artifact the layout review looks at. Until the
  bugs it found when it came are fixed (scrolling by rounding at 125 % on
  Windows, the What's new window, panels on Linux) it only warns on normal
  pull requests; a release PR fails on it.
- **Release rules** (`scripts/release-gate.py`, Linux job): translations complete,
  word rules, no Flatpak, relative links resolve, the contract.

In [omarchy-kyrr](https://github.com/heast13/omarchy-kyrr) (its `tests/`):
Omarchy's own plugin validation and text-format scan against 4.0.6 and the
newest stable Omarchy, every page of the menu built from the contract's
state in both languages, the update flow with stand-ins for kyrr, the plugin
and the shell restart, and the menu loaded for real in Quickshell with
Omarchy's components, with pictures per page and screen size.

## 2. Automatic, on the release pull request

`python3 scripts/release-gate.py --release --online --plugin <omarchy-kyrr>`
adds: one version everywhere (Cargo.toml, Cargo.lock, metainfo, CHANGELOG,
NEWS), the NEWS rules for this version (both languages, an item for each
system, length), no open gap in the contract (the bar menu does everything
the window does), external links alive, and a plugin that changed got a new
version.

## 3. The review crew

Each area is one reviewer with its own brief in `docs/release-gate/`, all
started together. Haiku reviews; Sonnet takes the areas where judgement on
code is needed (marked). Each writes its report to
`/mnt/project-files/notes/release-gate/<version>/<area>.md`: every item of
its brief as PASS, FAIL or WARN with the evidence (file:line, command output,
URL, picture). A FAIL blocks the release until it is fixed and that area has
run again.

| Area | Brief | Model |
| --- | --- | --- |
| New features on every system | [features.md](release-gate/features.md) | Haiku |
| Bar menu parity (Omarchy) | [omarchy.md](release-gate/omarchy.md) | Haiku |
| Layout, scaling, light and dark | [layout.md](release-gate/layout.md) | Haiku |
| Texts, both languages | [texts.md](release-gate/texts.md) | Haiku |
| Website, README, GitHub | [web.md](release-gate/web.md) | Haiku |
| Versions and release notes | [versions.md](release-gate/versions.md) | Haiku |
| Tests and CI | [tests.md](release-gate/tests.md) | Haiku |
| Install, update, uninstall | [update.md](release-gate/update.md) | Sonnet |
| Security | [security.md](release-gate/security.md) | Sonnet |

Every pull request gets the same kind of review once before it is pushed:
[pr-review.md](release-gate/pr-review.md) (Haiku).

How to run the crew (the release thread does this):

1. Release PR is open, its CI and the plugin's CI are green.
2. One message starting all reviewers in the background, each with
   "Read docs/release-gate/README-crew.md and docs/release-gate/<area>.md in
   the kyrr checkout, then do it for version X.Y.Z, previous version W"
   and the model from the table.
3. Collect the reports into one gate report for Flo: FAILs first, each with
   its fix (a PR), then WARNs, then the real-device list from the features
   review.
4. Fix, then run the failed areas again. Repeat until no FAIL.

## 4. Flo, on real devices

What only a person at the device can see. The features review writes the
list for this release; these always belong on it:

- **Windows 11:** install over the previous version (the update button in
  kyrr), first start, every switch on the main page, filter heard in a call
  app, dictation German and English in two apps, a voice memo with its text,
  the window at 100 % and 150 % scaling, uninstall leaves nothing behind.
- **Linux (GNOME or KDE, newest release):** the same with the AppImage,
  incl. the dictation key and typing permission of the desktop.
- **Omarchy (Flo's laptop):** one click on the update row updates kyrr and
  the plugin and restarts the bar, with no terminal command; every page of
  the bar menu with keyboard and mouse; the new features in the menu;
  dictation with Omarchy's keys.

Flo ticks the list in the gate report. Anything that fails goes back to
step 1 for what it touches.

## 5. Tag, publish, check

Only after Flo's go: the tag `vX.Y.Z` (it starts `release.yml`, which builds a
draft release), Flo publishes the draft, then the post-release review
([published.md](release-gate/published.md), Haiku) checks what is live:
release assets and checksums, release notes, README, website, versions,
links, the repository's About, AUR and winget, and the plugin store entry
(new commit SHA for a changed plugin).
