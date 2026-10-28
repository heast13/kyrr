# Tests and CI (Haiku)

Input: the CI results of the pinned commit (GitHub API or MCP tools), the run
of the test builds (docs/release-gate.md section 4), `git diff W..<sha>` in
`<out>/src`.

1. CI green on Windows and Linux for the pinned commit (see `PINNED.md`), every
   job (name each with its result), and the plugin's CI green for the plugin
   commit in `PINNED.md`. Read the results through the GitHub API or MCP; if
   they are not readable from here, WARN.
2. No test was removed, renamed away, marked `#[ignore]`, skipped by a
   condition or loosened (a tolerance raised, an assert dropped) since vW
   without a stated reason in the commit (list every case).
3. Every new feature (CHANGELOG) has at least one automated test; name it,
   or FAIL with the feature's name.
4. Clippy and fmt ran. Every new `#[allow(...)]` (list it with file:line) is a
   FAIL, except in developer examples (`crates/*/examples/`) with the reason
   given in the code.
5. If dictation or audio code changed (`crates/kyrr-dictate`,
   `crates/kyrr-denoise`, `crates/kyrr-voice`): the ASR or filter benchmark
   was run for this release (notes in /mnt/project-files/research/ or the PR);
   no worse than the previous release. Missing run = FAIL.
6. Artifacts of the test-build run (`kyrr-installer`: Windows setup with its
   .sha256; `kyrr-linux`: AppImage, tarball, each with its .sha256) cannot be
   downloaded from the cloud. Check their names and sizes from the run's
   metadata (GitHub API) against vW. Contents and checksums: WARN, Flo's
   installs (docs/release-gate.md section 4) cover them.
