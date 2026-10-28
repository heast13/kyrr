# After publishing (Haiku)

Input: the published GitHub release vX.Y.Z, the live website (add `?v=<now>`
to URLs, pages are cached for 15 minutes), the repository page, and the output
of `release-gate.py --release --online --after-tag` (item 5).

1. Release assets: Windows setup (versioned and unversioned `kyrr-setup.exe`),
   AppImage (versioned and unversioned `kyrr-x86_64.AppImage`), tarball, each
   with its `.sha256` that matches the file. The cloud proxy blocks the asset
   hosts (README-crew.md): from here this is a WARN, checked from another machine.
2. Release notes: complete, both What's-new blocks present, links work.
3. README and website (both languages) show this version where they show a
   version, the download buttons get this version.
4. `releases/latest/download/...` URLs used by docs, site and the Omarchy
   plugin resolve to this release.
5. Links: `python3 scripts/release-gate.py --release --online --after-tag`. After
   tagging the tag's URLs are real links and must answer (`--after-tag`: a 404
   FAILs). Hosts the cloud proxy blocks appear as "not reachable through the
   cloud proxy, check from another machine": check that list from another
   machine (Flo's PC via Remote Control), or name it in the report as open for Flo.
6. Repository About, topics, social preview fine.
7. AUR (`kyrr-bin`) and winget workflows ran or their state is known.
8. Omarchy: if the plugin changed, its main is the tested commit and the
   store's "Plugin verification" form needs that SHA (say so to Flo).
9. Fix what is wrong where it is safe (texts, README via PR); report the rest.
