# After publishing (Haiku)

Input: the published GitHub release vX.Y.Z, the live website (add `?v=<now>`
to URLs, pages are cached for 15 minutes), the repository page.

1. Release assets: Windows setup (versioned and unversioned `kyrr-setup.exe`),
   AppImage (versioned and unversioned `kyrr-x86_64.AppImage`), tarball, each
   with its `.sha256` that matches the file.
2. Release notes: complete, both What's-new blocks present, links work.
3. README and website (both languages) show this version where they show a
   version, the download buttons get this version.
4. `releases/latest/download/...` URLs used by docs, site and the Omarchy
   plugin resolve to this release.
5. Repository About, topics, social preview fine.
6. AUR (`kyrr-bin`) and winget workflows ran or their state is known.
7. Omarchy: if the plugin changed, its main is the tested commit and the
   store's "Plugin verification" form needs that SHA (say so to Flo).
8. Fix what is wrong where it is safe (texts, README via PR); report the rest.
