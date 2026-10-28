# Website, README, GitHub (Haiku)

Input: README.md, docs/*.md, site/ (both languages), the repository page on
GitHub, `python3 scripts/release-gate.py --online` output.

1. Every new feature of this version is on the website (both languages) and
   in README where comparable features are; nothing promised that the app
   does not do; nothing removed still described.
2. Install and update instructions are right for this version on every
   system: Windows installer and winget, Linux copy-paste install, Omarchy
   one command from the plugin store; autostart foldout and Linux uninstall
   are there; no Windows uninstall section (Flo's rule).
3. Screenshots show the current UI (compare with the layout pictures); alt
   texts present.
4. All links resolve (`--online`), incl. download links that will exist
   after the release (`releases/latest/download/...` names match what
   `release.yml` uploads).
5. Website style rules (README-crew.md), German and English pages say the
   same, `site/sitemap.xml` lists both.
6. Repository About (description, website, topics) still fits.
7. docs/faq.md and docs/how-it-works.md: new features explained where they
   belong; answers still true.
