# Bar menu parity, Omarchy (Haiku)

Input: the pinned run: `<out>/src` (kyrr at the pinned commit, with
`packaging/omarchy/contract.json`) and `<out>/plugin` (omarchy-kyrr at the
pinned plugin commit).

How the plugin's tests run (`tests/README.md` and `.github/workflows/ci.yml` in
omarchy-kyrr):

- Here, in `<out>/plugin`, against the pinned contract (`KYRR_CONTRACT`; without
  it the tests read kyrr's `main` from GitHub, which is not the pinned commit):
  `KYRR_CONTRACT=<out>/src/packaging/omarchy/contract.json node --test tests/*.test.mjs`
  (pages and texts; plain node, no npm), the same variable with
  `node tests/warnings.mjs` (warnings only, never fail), and
  `bash tests/update-flow.test.sh` (the update flow with stub curl, omarchy,
  git, setsid and fusermount3; it needs no contract).
- CI also runs `shellcheck --severity=warning update.sh tests/*.sh tests/load/*.sh`
  and Omarchy's own plugin validation and text-format scan, which need an
  Omarchy checkout at the tag. Run them only when one is at hand; otherwise WARN.
- The load test (`tests/load/run.sh`) needs Arch with quickshell, sway, grim,
  socat, jq, nodejs and a Nerd font. It does not run in the cloud.

1. `python3 scripts/check-omarchy-contract.py --release` in `<out>/src` passes
   (no gap left).
2. The plugin's CI is green for the pinned plugin commit (GitHub API or MCP),
   with the contract of this kyrr version; the local test commands above pass
   in `<out>/plugin`.
3. Every feature of this version (CHANGELOG) can be reached in the menu with
   the keyboard alone and with the mouse alone: name the page and row.
4. The load test's pictures (artifact `load-<tag>` of the plugin CI) are not
   reachable from here, and the load test does not run in the cloud: WARN, and
   Flo checks the menu on his laptop (docs/release-gate.md section 4: every page
   with keyboard and mouse). Wherever the pictures are readable: every page,
   every screen size; nothing cut off, nothing overlapping, no page that scrolls
   on 1080 px or more, text readable on the theme.
5. Texts of the menu (Texts.js) say the same as the window's (kyrr.po and
   app.slint) for the same thing, in both languages.
6. The plugin's README "State" and commands sections match
   `packaging/omarchy/contract.json`.
7. Versions: if the plugin changed since the last kyrr release, its
   manifest.json version is higher; the plugin hides pages a too old kyrr
   cannot serve (reads `ai` etc. only when present).
8. Update path: the update row runs `update.sh update` (kyrr, plugin, shell
   restart in one go); `bash tests/update-flow.test.sh` passes in `<out>/plugin`.
9. Store: if the plugin changed, the report says so and names the commit SHA
   that has to go into the store's "Plugin verification" form.
