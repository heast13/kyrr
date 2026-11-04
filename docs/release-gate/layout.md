# Layout, scaling, looks (Haiku)

Input: the pinned run's pictures, built on this machine in `<out>/layout/`:
`problems.txt` and the contact sheets
`sheet-<look>-<lang>-<scale>-<n>.png`, each with a `.txt` beside it that names
its numbered pages. Windows pictures are not in the cloud (item 5). The plugin
load test's pictures: see omarchy.md.

1. The layout check ran for the pinned commit: `layout/problems.txt` is empty,
   and all four look sets are there (`light`, `dark`, `tokyo-night`,
   `catppuccin-latte`, each in German and English at every scale). A missing
   set, or a run that failed (`<out>/layout.log`), is a FAIL.
   Windows: the CI step "Layout" of the release PR passed (`layout-windows`).
   If its log is readable through GitHub, check it; otherwise WARN.
2. Open every picture in `<out>/layout/`. Per picture: nothing cut off (text,
   buttons, cards), nothing overlapping, no text running out of its box, no
   empty band where the window is taller than its content, icons sharp at
   every scale.
   Labels in particular: for every label that is new or changed in this
   version, find it in the pictures at 1.0 and 2.0 and read it to the last
   letter, in German and English, in every look. A label that ends in "…" or
   whose last word is missing is a FAIL, whatever `problems.txt` says (it
   cannot see text cut off inside its own box; its `labels:` lines only
   catch rows that are too long on paper). The bar menu's labels are checked
   by the plugin test `tests/labels.test.mjs`; for new ones look at the
   Quickshell load test pictures of the plugin's CI too.
3. German is the long language: every German page fits as well as English.
4. Looks: for each page, find its sheet in each look (the `.txt` beside each
   sheet names the pages) and look at the four side by side: `light` and
   `dark` (system scheme), and on Linux `tokyo-night` (Omarchy dark) and
   `catppuccin-latte` (Omarchy light). Check: text readable everywhere, nothing
   invisible on its background, focus rings visible, a dark look is dark and a
   light look is light. The automatic comparison only sees content that
   vanishes between a pair (light/dark, tokyo-night/catppuccin-latte). It does
   not judge contrast, focus rings or colours, so check those on the pictures
   yourself, and WARN when they are borderline.
5. Same page, same look on Windows and Linux (fonts aside). No Windows pictures
   here: this is a WARN, not a FAIL, pointing to Flo's Windows 11 check (the
   real-device list, docs/release-gate.md section 4), unless the Windows
   pictures or the Windows CI log become readable.
6. The voice memo text in both widths (520 and 1040) and the compare view.
7. The dictation pill (if it changed): position, size and text at each scale.
8. Keyboard: from the code (`app.slint` FocusScope, key handlers) and the
   pictures, every control can be reached with Tab and used with Space or
   Enter; Esc closes what it should. Name any control the keyboard cannot
   reach (FAIL).
