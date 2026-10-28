# For every reviewer of the release gate

You check one area of a kyrr release before it is tagged (see
`docs/release-gate.md`). Your brief is the other file you were given. Rules
for all areas:

- **Every item of your brief gets a verdict.** PASS, FAIL or WARN, each with
  its evidence: file:line, the command and its output, a URL, a picture's
  name. "Looks fine" is no evidence. An item you could not check is a FAIL
  with the reason ("could not run X because Y"), never a quiet skip. Only what
  the cloud cannot reach is a WARN instead (see below).
- **FAIL** = a user would hit it, or a rule below is broken. **WARN** = worth
  a look, not a blocker. When unsure, FAIL and say why.
- **Look at what changed, and around it.** The diff is the one named in
  `PINNED.md` (`git -C <out>/src diff <W>..<sha>`, W = previous version); it
  shows what is new. Check those parts closely, then the rest of your area
  briefly, because old things break too.
- **Both systems, three surfaces.** Windows, Linux (GNOME, KDE, …) and the
  Omarchy bar menu (repo heast13/omarchy-kyrr) are meant to look, read and
  behave the same; a difference is a FAIL unless the system forces it and it
  is written down.
- **Don't fix, report.** Change no files in the repositories. Scratch files go
  in your scratchpad and are deleted when you are done.
- **Report** to `/mnt/project-files/notes/release-gate/<version>/<area>.md`:
  first line `RESULT: PASS|FAIL (n FAIL, m WARN)`, then one line per item
  (`FAIL 3. … — evidence`), FAILs first. Your reply is that file's first
  20 lines.

## Where you read, what you can reach

- **Read only from the pinned folder** that your message names (`<out>`, see
  `PINNED.md`): the sources in `<out>/src` and `<out>/plugin`, the briefs in
  `<out>/src/docs/release-gate/`, the pictures in `<out>/layout/`, and
  `release-gate.txt`. Not from the live kyrr or omarchy-kyrr checkout, which
  moves while fixes are pushed.
- **The commit goes into the report**: the first line after `RESULT:` is
  `Commit: <sha>` (plugin: `<sha>`), copied from `PINNED.md`.
- **Not reachable from the cloud**: CI artifacts (`layout-windows`,
  `layout-linux`, `load-<tag>`, the artifacts of a release run) and some
  external hosts that the cloud proxy blocks with 403: shields.io, slint.dev
  and the release asset hosts. That is no FAIL of the release: write
  `WARN: not reachable from here, please check: <what>` and name it in the
  gate report, so that someone on another machine checks it.
- **What runs here**: the layout pictures are in `<out>/layout/` (built on
  this machine: all four looks on Linux; the Windows looks are not built here).
  The Linux build and `cargo test -p kyrr-linux omarchy` run in the cloud, in
  `<out>/src`. Setup: apt packages `libpipewire-0.3-dev libclang-dev
  libasound2-dev libfontconfig1-dev libxkbcommon-dev libssl-dev pkg-config`
  (`gate-prepare.py --apt` installs them), then `python3 scripts/fetch-assets.py
  --linux`, then `SHERPA_ONNX_LIB_DIR=assets/sherpa-onnx/lib` for cargo.

Project rules every area knows (from Flo):

- App texts are for normal users: simple, helpful, no file paths, commands
  or protocol names. German: "Geräuschunterdrückung" (not
  Rauschunterdrückung), "Diktierfunktion" (not Diktat); "diktieren" is fine.
- Flatpak is gone for good: never in texts, site, notes or releases.
- Website: factual, short, for non-technical people, no slogans, no AI
  filler; playful only in the word-creation line and the dictation sayings.
- Newest systems are the reference (current Windows 11, newest Ubuntu,
  Fedora, GNOME, KDE, Arch, Omarchy 4.0.6 and newest).
- The Omarchy update is one click: kyrr, the plugin and the bar restart,
  never a terminal command for the user.
