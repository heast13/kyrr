# Tests and CI (Haiku)

Input: the release PR's CI runs, `git diff vW..HEAD`.

1. CI green on Windows and Linux for the release PR's last commit, every job
   (name each with its result); plugin CI green on its main.
2. No test was removed, renamed away, marked `#[ignore]`, skipped by a
   condition or loosened (a tolerance raised, an assert dropped) since vW
   without a stated reason in the commit (list every case).
3. Every new feature (CHANGELOG) has at least one automated test; name it,
   or FAIL with the feature's name.
4. Clippy and fmt ran without `allow` attributes added to silence new
   warnings (list new `#[allow(...)]`).
5. If dictation or audio code changed (`crates/kyrr-dictate`,
   `crates/kyrr-denoise`, `crates/kyrr-voice`): the ASR or filter benchmark
   was run for this release (notes in /mnt/project-files/research/ or the PR);
   no worse than the previous release. Missing run = FAIL.
6. Build artifacts of the release PR (or the draft release): Windows setup,
   AppImage, tarball, each with its .sha256, sizes plausible against vW.
