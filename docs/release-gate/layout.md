# Layout, scaling, light and dark (Haiku)

Input: the layout check's pictures (CI artifacts `layout-windows` and
`layout-linux` of the release PR) and the plugin load test's pictures.

1. The layout check passed on Windows and on Linux (CI step "Layout" of the
   release PR; its `problems.txt` in the artifact is empty).
2. Open every picture. Per picture: nothing cut off (text, buttons, cards),
   nothing overlapping, no text running out of its box, no empty band where
   the window is taller than its content, icons sharp at every scale.
3. German is the long language: every German page fits as well as English.
4. Light and dark (and an Omarchy theme): text readable everywhere, nothing
   invisible on its background, focus rings visible.
5. Same page, same look on Windows and Linux (fonts aside).
6. The voice memo text in both widths (520 and 1040) and the compare view.
7. The dictation pill (if it changed): position, size and text at each scale.
8. Keyboard: from the code (`app.slint` FocusScope, key handlers) and the
   pictures, every control can be reached with Tab and used with Space or
   Enter; Esc closes what it should. Name any control the keyboard cannot
   reach (FAIL).
