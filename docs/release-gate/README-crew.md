# For every reviewer of the release gate

You check one area of a kyrr release before it is tagged (see
`docs/release-gate.md`). Your brief is the other file you were given. Rules
for all areas:

- **Every item of your brief gets a verdict.** PASS, FAIL or WARN, each with
  its evidence: file:line, the command and its output, a URL, a picture's
  name. "Looks fine" is no evidence. An item you could not check is a FAIL
  with the reason ("could not run X because Y"), never a quiet skip.
- **FAIL** = a user would hit it, or a rule below is broken. **WARN** = worth
  a look, not a blocker. When unsure, FAIL and say why.
- **Look at what changed, and around it.** `git log --oneline vW..HEAD` and
  `git diff vW..HEAD` (W = previous version) show what is new; check those
  parts closely, then the rest of your area briefly, because old things break
  too.
- **Both systems, three surfaces.** Windows, Linux (GNOME, KDE, …) and the
  Omarchy bar menu (repo heast13/omarchy-kyrr) are meant to look, read and
  behave the same; a difference is a FAIL unless the system forces it and it
  is written down.
- **Don't fix, report.** Change no files in the repositories. Scratch files go
  in your scratchpad and are deleted when you are done.
- **Report** to `/mnt/project-files/notes/release-gate/<version>/<area>.md`:
  first line `RESULT: PASS|FAIL (n FAIL, m WARN)`, then one line per item
  (`FAIL 3. … — evidence`), FAILs first. Your reply is that file's first
  20 lines.

Project rules every area knows (from Flo):

- App texts are for normal users: simple, helpful, no file paths, commands
  or protocol names. German: "Geräuschunterdrückung" (not
  Rauschunterdrückung), "Diktierfunktion" (not Diktat); "diktieren" is fine.
- Flatpak is gone for good: never in texts, site, notes or releases.
- Website: factual, short, for non-technical people, no slogans, no AI
  filler; playful only in the word-creation line and the dictation sayings.
- Newest systems are the reference (current Windows 11, newest Ubuntu,
  Fedora, GNOME, KDE, Arch, Omarchy 4.0.6 and newest).
- The Omarchy update is one click: kyrr, the plugin and the bar restart,
  never a terminal command for the user.
