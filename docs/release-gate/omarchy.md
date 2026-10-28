# Bar menu parity, Omarchy (Haiku)

Input: kyrr checkout, omarchy-kyrr checkout (clone github.com/heast13/omarchy-kyrr).

1. `python3 scripts/check-omarchy-contract.py --release` passes (no gap left).
2. The plugin's CI is green on its main branch (GitHub Actions of
   heast13/omarchy-kyrr), with the contract of this kyrr version.
3. Every feature of this version (CHANGELOG) can be reached in the menu with
   the keyboard alone and with the mouse alone: name the page and row.
4. Look at the load test's pictures (artifact of the plugin CI): every page,
   every screen size; nothing cut off, nothing overlapping, no page that
   scrolls on 1080 px or more, text readable on the theme.
5. Texts of the menu (Texts.js) say the same as the window's (kyrr.po and
   app.slint) for the same thing, in both languages.
6. The plugin's README "State" and commands sections match
   `packaging/omarchy/contract.json`.
7. Versions: if the plugin changed since the last kyrr release, its
   manifest.json version is higher; the plugin hides pages a too old kyrr
   cannot serve (reads `ai` etc. only when present).
8. Update path: the update row runs `update.sh update` (kyrr, plugin, shell
   restart in one go); its test in the plugin CI passed.
9. Store: if the plugin changed, the report says so and names the commit SHA
   that has to go into the store's "Plugin verification" form.
