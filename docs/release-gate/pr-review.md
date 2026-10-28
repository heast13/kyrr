# Before every push (Haiku)

The reviewer that runs once, right before a pull request is pushed for
review or merged (not after every step). Input: `git diff origin/main...HEAD`
of each repository the work touched, and the CI results if a run exists.

1. **Correct.** The change does what its description says; no leftover debug
   code, no dead code, no TODO without a reason; error cases handled.
2. **Every system.** A change to a feature reaches Windows, Linux and the
   Omarchy bar menu, or the PR says why not. A new or changed control in
   `app.slint` has its entry in `packaging/omarchy/contract.json` and, unless
   it is `windowOnly`, the plugin change exists (link the plugin PR).
3. **Tested.** New behaviour has a test; no test removed, ignored or
   loosened; `cargo fmt`, `cargo clippy -D warnings`, `cargo test`,
   `python3 scripts/check-omarchy-contract.py` and
   `python3 scripts/release-gate.py` pass locally or in CI.
4. **Layout.** UI changes: the layout check passes; look at its pictures of
   the changed pages in German at scale 1.0 and 2.0.
5. **Texts.** Both languages, Flo's text rules (README-crew.md).
6. **Safe.** No secrets, no user text in process arguments, downloads
   verified, file modes private for user data, no new network access the
   user did not ask for.
7. **Clean.** CHANGELOG and NEWS untouched (only the release PR writes
   them); no helper files, backups or scratch files committed; commit
   messages explain why.

Verdict per item like the gate (README-crew.md), but reply in the chat
instead of a report file. Any FAIL: fix before pushing.
