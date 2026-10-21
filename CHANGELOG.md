# Changelog

## 0.6.0 (2026-10-21)

- Dictation, words in the pill: fewer wrong and missing words. A word shows
  once 1.2 s of the recording followed it (was 0.8 s), and after a pause
  the next look starts 1.5 s before where the last one ended (was 0.8 s), so
  speech that had just begun is not cut off. Looks cover a little more
  audio, so they take a little more computing time. In a test with 27
  synthetic clips, 71 of 1536 shown words differed from what was said, down
  from 90 of 1518, at about 0.1 s more delay on average.
- Settings rebuilt for overview: the "Settings" page is now "Global
  settings" (effort, language, start with the system, microphone, updates,
  about). Each switch on the main page has its own gear that opens only
  that card's settings: noise (filter strength, level check, listening),
  voice (sound, automatic volume), dictation (key, hold or tap). Every
  page opens in "Simple"; "Advanced" on the same page adds the rest (noise
  model, own sound, compressor, S sounds, rumble, pops, distance
  compensation, words while you talk, update settings). One column
  everywhere, Windows and Linux alike. Simple or Advanced is no longer
  remembered between visits.
- "Feedback" under Global settings, About (and on the website and in the
  README): opens an e-mail to the maintainer with the kyrr version, the
  system (Windows, or distribution, desktop and install type on Linux) and
  the language filled in. kyrr sends nothing itself; no log is attached.
- Dictation: a key press while the speech model still loads (a few seconds
  after kyrr starts or was installed) starts the recording at once, and it
  is recognised when the model is there. Before, the recording waited for
  the model, so the first words were missing or cut and came out wrong.
- Linux, AppImage: pasting the install commands again no longer needs kyrr
  closed first. The commands download to `kyrr.new` and move it over `kyrr`
  (a running AppImage cannot be written to), and `kyrr --replace` ends the
  running kyrr like quitting does (SIGTERM, the default microphone is given
  back), waits until it is gone (up to 20 s; if it hangs longer, the old one
  just shows its window) and starts in its place. If the download fails, the
  old file stays and is merely restarted. Only
  a process called kyrr is ended, found by the socket it listens on; a
  Flatpak kyrr is left to Flatpak.
- Distance compensation is out of beta and on by default, for existing
  users too. The 0.5.8 switch (`VoiceDistance`) is no longer read; the
  setting is now `VoiceDistanceOff`, so an off set in 0.5.8 does not carry
  over. Settings, Advanced, "Distance compensation" switches it off.
- Dictation wave: the words are written by an invisible pen that glides
  along the line at an even pace (the average of how fast words came in
  lately, plus a share of what waits), instead of every word fading in on
  its own as soon as a pass delivered it. Each letter fades in over 0.8 s
  after the pen passed it, rising a little; the pill widens and the line
  scrolls with the pen, not in steps per batch of words. In a recording
  of the effect the words stand about 0.3 to 0.5 s later than with the old
  look, in return for no stop and go.
- Live words: after a look that found nothing new, the next look starts
  0.8 s before where that one ended, so a pause no longer makes the
  stretch grow up to 8 s. In a simulation of a computer 4x slower than the
  test machine, the words after a 15 s pause showed about 1.5 s sooner;
  the share of CPU time spent on looks stayed about the same.
- Flatpak on Hyprland, sway, niri and river: kyrr types and shows the
  dictation wave there too. The block kyrr writes into the compositor's
  config (dictation key, start at sign-in) now also links the compositor's
  Wayland socket into kyrr's runtime folder, because the sandbox's own
  socket hides the virtual keyboard and layer surfaces. Without the link,
  kyrr says that the AppImage can type there. The start at sign-in on these
  compositors also goes through their config in the Flatpak.
- Flatpak: the tray icon works (registers under its unique D-Bus name, as
  sandboxed apps must; `vendor/ksni` patches the tray library for that).
  On CPUs without AVX2 the noise filter uses the shipped ONNX Runtime.
  `packaging/flatpak/flathub.py` writes the files for Flathub, and a CI
  workflow builds and lints them the way Flathub does.
- AUR: `kyrr-bin` at the current version, built and checked on Arch for
  every change to the package, and pushed to the AUR after each release once the key is
  set up.
- Setup in three steps (Windows and Linux alike): microphone, then
  language and dictation model (with the key and typing questions on
  Linux), then the performance setting.
- Effort setting, "Lean" or "Best quality" (Settings, Simple; asked in the
  setup): Lean is the small noise model and the words after you stop
  talking, Best quality the large model and words while you talk. The first
  time the dictation model loads, kyrr times one look; a slow computer (over
  0.4 s for 4 s of audio, or no AVX2) gets a suggestion for Lean, once.
  Lean dictations end after 2 minutes, others after 15; at the limit kyrr
  types what it heard instead of dropping the rest, and the wave turns
  yellow (30 s left) and red (10 s left). Text heard while the dictation key
  is still down waits for its release (at most 3 s).
- Live words: placed by the model's own word times; a look starts 1.5 s
  before the last word shown, a word shows once 0.8 s of recording followed
  it or two looks agree. On a computer three times slower the words show
  after about 2.4 s instead of 6 s. A computer needing over 2.5 s per look
  stops showing words for that dictation.
- Status line: 20 phrases per state, picked at random, never the same one
  twice in a row; 14 grumbles in the pill after a dictation that heard
  nothing. 12 px hints, the tray menu uses the card titles.
- Performance: pieces of a long dictation are recognised while the key is
  held (2 minutes of speech: 8.3 s wait after the key before, 0.9 s now);
  ONNX Runtime without a memory arena and without spinning threads (peak RAM
  1.92 to 1.59 GB, about 17 % less CPU for the final recognition); the noise
  filter's int8 kernels also come as AVX-VNNI and AVX-512 VNNI builds, bit
  for bit like AVX2 (Quality 29 % to 20 % of one core).
- Stability: a model download that stalls (60 s without data) is asked again
  from where it broke off, and a cancel stops at once; a damaged model is
  deleted and fetched again by Retry; a language change waits until a
  dictation is typed; one update download at a time, written to a .part
  file first; `filter.json` is written atomically and a broken
  `config.json` no longer resets everything; Linux: kyrr sets the virtual
  microphone up again after a PipeWire restart, waits for PipeWire at the
  start, and a second start at the same moment hands over; the default
  microphone is given back correctly even after a crash; Windows: pasting
  and the audio filter's setup handle more edge cases. Voice shaping: the
  filters no longer slow down after long silences and the low cut no longer
  clicks when switched on.
- Linux: the window is drawn by the software renderer only (the OpenGL
  renderer crashed at start on Wayland without libwayland-egl and kept over
  100 MB of RAM). The AppImage and the tarball are a third smaller (69 to
  46 MB: the noise models without their `.onnx` files). An AppImage on a
  system with a too old glibc says so in a dialog.
- Security: downloads and the update check are HTTPS only, redirects
  included; newer root certificates (RUSTSEC-2026-0381); third-party CI
  actions pinned to commits.
- Keyboard and screen reader access for the remaining click-only controls,
  better contrast for small notes.
- Windows: the README and the website say that dictation does not work while
  a window running as administrator is in front (Windows hides the keyboard
  from normal programs there).
- Release: the Flatpak bundle (`kyrr-x86_64.flatpak`, plus `.sha256`) is
  attached to the release; release notes start with the plain summary from
  NEWS.md and fold the technical section away.

## 0.5.8 (2026-10-20)

- Dictation: while you speak, the words kyrr hears so far run through the
  wave as one line. The pill widens, the words fade in one after the other
  and the oldest run out on the left. A word shows once two passes of the
  model agree on it, so nothing on the line jumps. What gets typed is
  unchanged: the whole recording, recognised once the key is let go. It
  can be switched off in the advanced settings. Windows and Linux alike.
- Dictation in more languages: next to "Deutsch" and "English" (in the setup
  and under Settings, General, Language) there is now "Multilingual". It adds
  French, Spanish, Italian, Dutch, Polish, Portuguese and more; kyrr notices
  the language by itself. One more download of about 900 MB (NVIDIA
  parakeet-tdt-0.6b-v3, converted like the German model). The window then
  uses the system's language.
- Distance compensation: a slow detector on the raw microphone tells near
  from far by how steeply the voice falls after a syllable (the room's echo
  limits that fall). When far, a four-band expander takes the echo tail down
  by up to 8 dB, with a little bass back, less box at 450 Hz and a touch of
  presence, faded in over two seconds before the automatic volume. Close to
  the mic the audio stays the same bit for bit. Beta, off by default: a
  switch in the advanced settings; while it is on, the main page suggests
  moving closer when the mic sounds far.
- Linux: when kyrr cannot install an update itself, "Download page" opens the
  download page on the website, with the install steps, instead of the GitHub
  release page. Windows uses the same page for a failed download.

## 0.5.7 (2026-10-19)

- German texts: "Geräuschunterdrückung" instead of "Rauschunterdrückung" (it
  removes background noise, not a hiss) and "Diktierfunktion" instead of
  "Diktat" as the name of the feature, in the app, the website and the
  desktop keywords.
- The dictation wave looks calmer and fits the system: a grey pill, dark
  or light as the system is set, with kyrr's green bars. The bars no longer
  flutter and rise and fall more gently; while kyrr listens each bar follows
  one pitch band of the voice (low sounds in the middle, high ones at the
  sides) instead of a fixed pattern. Costs next to nothing: seven bands
  from the newest 32 ms of the microphone, 30 times a second.
- Linux, Wayland (Hyprland): dictated text no longer loses its spaces in
  browser text fields (claude.ai as a web app, for example). Browsers look at
  the key code, and the space sat on the code of Esc; kyrr now keeps its keys
  off Esc, Enter, Backspace, the modifiers and the like.

## 0.5.6 (2026-10-17)

- While you dictate, a small dark pill with kyrr's teal wave shows at the
  bottom of the screen: the bars move with your voice while kyrr listens,
  and a quicker wave runs through them while it writes. It fades out when
  the text is in. It never takes the focus and lets clicks through, so the
  text still lands where you were typing. Windows: on the screen of the
  window you work in. Linux: a layer surface on Hyprland, KDE, Sway and
  other wlroots desktops; on GNOME and X11 desktops an X window (through
  XWayland on GNOME), which needs a compositor.
- A dictation can now be 15 minutes long instead of 2. The recording is
  recognized in pieces of about 30 seconds, cut at pauses, and typed or
  pasted as one text. (Windows) A long text typed key by key, because the
  clipboard was out of reach, goes out in batches, which makes lost keys
  less likely.
- The sentence at the top of setup step 2 reads simpler now: hold the key,
  speak, let go.
- Linux, Hyprland: kyrr opened from the tray comes up as a floating window
  again instead of being tiled.
- Linux, Wayland (Hyprland): the second dictation no longer comes out as
  other letters, and neither does the end of a long one. Every text now
  gets a keyboard of its own and a single keymap.

## 0.5.5 (2026-10-15)

- Linux, GNOME and KDE: a dictation with letters your keyboard layout does
  not have (German umlauts on a US layout, say) is now pasted into the
  window by itself instead of only landing on the clipboard. kyrr presses
  Shift+Insert for it, which pastes in terminals too.
- Linux on Wayland: closing the window (or "Off to the corner") without a
  tray minimizes it every time, not only the first time, and the window
  fits its content again after it came back. The note under the buttons
  that only said so is gone.
- Linux: switching the settings from advanced back to simple makes the
  window smaller again, also on Hyprland, which kept the bigger size.
- The Linux version is marked as a beta: in kyrr's About, on the website,
  in the README and in the release notes.

## 0.5.4 (2026-10-15)

- Fewer technical texts in the window: when kyrr cannot set the dictation
  key up by itself, it no longer shows config file paths and lines (they go
  to the log) but asks for the command on a key in the desktop's keyboard
  settings. A taken key just says it is taken. Failed tests, listening and
  setup say what to check instead of showing raw error text, and the
  installer no longer shows exit codes.
- On Hyprland (Omarchy, say), Sway, river and niri the dictation key works
  without any hand work: kyrr adds it to the key bindings itself, in a block
  between two marker lines that it keeps up to date when the key changes,
  and leaves the rest of the file as it is. Holding the key works everywhere
  but on niri, which only knows presses: there each press starts or ends
  dictation. A key that another bind of yours already uses is not touched;
  kyrr says so and you pick another one. Where the config cannot be written
  (a read-only config, a Flatpak without access), kyrr asks for its
  command on a key in the desktop's keyboard settings.
- Where such a desktop is started without uwsm or a systemd session (and so
  skips the usual start-at-sign-in entries), kyrr puts its start at sign-in
  into the same config, in a block of its own that goes when you switch the
  start off.
- GNOME 46 and 47 have no shortcut dialog for apps on
  Wayland: kyrr now adds the dictation key to GNOME's own keyboard
  shortcuts itself. Each press starts or ends dictation.
- No PipeWire? kyrr no longer just says so: "Set up PipeWire" installs it
  with your system's package manager (apt, dnf, pacman or zypper) after your
  system asks for your password, then kyrr starts again. PipeWire takes
  over from PulseAudio, which the package manager removes. Where that cannot
  work, kyrr shows the command for a terminal. The AppImage now also starts
  on systems without the PipeWire library at all. In a Flatpak kyrr says
  what to install.
- In a Flatpak, kyrr now looks for the desktop's tray like everywhere else
  (it used to assume there was none), so the icon shows up where the
  sandbox lets kyrr reach the tray host. Without a tray host kyrr no longer
  logs a tray error at start; it waits for one, as before.
- "Off to the corner" is hidden where it could do nothing: no tray, and a
  desktop that cannot minimize (Hyprland and other tiling ones). The hint
  next to it already says that closing the window quits kyrr there.
- A second start at sign-in while kyrr already runs no longer opens its
  window.
- A microphone that is unplugged while kyrr moves to it (on Windows while
  the permission prompt is open) no longer leaves kyrr on no microphone:
  the move changes nothing and kyrr stays where it was, without restarting
  the audio. A move attaches to the new microphone before it lets go of the
  old one, and a filter that is set up but on no microphone goes back onto
  the default one by itself, or offers it as a button. Windows and Linux.
- kyrr no longer crashes at sign-out on Linux once the AppImage is gone.
- Settings, About: the buttons sit on two rows, so they fit in German and
  with wider fonts.
- The status line has more words to pass the time while the filter runs
  (Burpinsured, Fanmuzzled, Snoreproof and friends); "Keyboard clatter
  ban" gave way to "Keyclackfree".

## 0.5.3 (2026-10-14)

- kyrr for Linux, as an AppImage (`kyrr-x86_64.AppImage`, plus a tarball
  for packagers). 64-bit Intel or AMD, glibc 2.39 or newer (Ubuntu 24.04,
  Fedora 40, Debian 13, Arch), PipeWire. The noise filter is a virtual
  PipeWire microphone called "kyrr" that becomes the default; it follows
  the microphone your system would use, as on Windows, and says when that
  microphone is muted. GNOME's orange microphone icon only shows while
  something really records.
- Dictation on Linux: the key goes through the desktop's global shortcuts
  on GNOME and KDE (the desktop asks once, kyrr walks you through it) and is
  read directly elsewhere; typing goes through the desktop's remote desktop
  permission, the Wayland virtual keyboard or X11, with the clipboard as the
  fallback.
- On Linux without a tray icon, the window's X minimizes kyrr on desktops
  that can restore it (GNOME, KDE, Cinnamon, Xfce and the like) and quits it
  on tiling ones. Linux is only offered updates that come with an AppImage.
- The wave next to the status line steps on a timer instead of every frame
  and needs far less processor time. A settings file that does not read is
  kept as config.json.bak instead of being overwritten.

## 0.5.2 (2026-10-14)

- A microphone Windows has muted is said on the main page ("Your mic is muted
  in Windows.") with a "Switch it on" link that lifts the mute, and dictation
  tells why nothing was written.
- No microphone at all: "No microphone found. kyrr keeps looking."; a newly
  plugged one is named in green for ten seconds ("Microphone connected: ...").
- The main page has a "Quit" button, bottom left; "Off to the corner" moved to
  the right.
- New defaults: S sounds softened "light" and automatic volume on. The main
  page's voice card is called "Improve voice".
- "What's new" opens by itself after an update and, under Settings, About,
  at any time, listing every version from the newest down. The text is built
  into the app (NEWS.md), so it also works when you install by hand.

## 0.5.1 (2026-10-14)

- Plugging another microphone in or out is followed by itself: kyrr checks
  every few seconds which microphone Windows uses and moves the noise filter
  there (Windows asks for permission once, then sound pauses for a moment).
  A microphone on another USB port counts as a new one. If you decline, a
  button on the main page offers it. A microphone you pick by hand with
  "Change microphone" stays while Windows' default stays as it was, and so
  does one kyrr was already set up on.
- If Windows takes kyrr off the microphone (an update, most likely), kyrr
  puts itself back once by itself; the "Repair" button stays as the fallback.
- The microphone list in the window stays current while it is open.
- Microphones Windows had not given a "supported modes" entry (seen on a
  RØDE PodMic USB) now get the noise filter too: kyrr adds the entry when it
  attaches. A microphone attached earlier without it is repaired by itself.

## 0.5.0 (2026-10-13)

- The main window keeps its height; it cannot be pulled taller or shorter
  any more. Settings stay resizable.
- A small red note at the bottom of the main page: when your microphone is
  too quiet or too loud for the automatic volume to fix (turn it up or down
  in Windows), or when "Listen to yourself" is still running. Several notes
  take turns, one line at a time. The listen button is red while it runs.
- Automatic volume, advanced view: "Right now: +9 dB" in green when it
  raises your voice, "−9 dB" in red when it lowers it.
- The advanced settings use the room better: the automatic volume sits in
  the right column, and the window grows up to 1250 pixels where the screen
  has room, so everything shows without scrolling.
- "What's new": an update comes with a short summary in plain words, in the
  update note's new button. kyrr reads it from the answer of its daily
  update check, so there is no extra connection. It shows from the update
  after this one on.

## 0.4.0 (2026-10-11)

- Automatic volume, in Settings below Voice: brings your voice to the same
  level however your microphone is set, with or without the voice shaping.
  It listens only while you speak and moves slowly, so pauses, clicks and a
  fan leave it alone. Off by default; the advanced view shows what it evens
  out right now.
- Pop protection: softens the thump of P and B when you are close to the
  mic, without touching the voice. Off by default, under Settings, Advanced.
- Settings come in Simple and Advanced. Simple shows the basics: filter
  strength, sound, automatic volume, dictation, the check, language,
  autostart and the microphone. Advanced adds the model, the custom sound,
  the compressor (formerly "Even out volume"), S sounds, low cut, pop
  protection and checking for updates; a new version still shows in the
  main window. The credits fold away behind a button in Simple.
- A long microphone name wraps instead of being cut off; shorter texts.
- From 0.3.1, the update installs with one click from kyrr itself.

## 0.3.1 (2026-10-11)

- Updates in one click. Once a day kyrr asks GitHub for the newest version
  number (Settings, General, "Look for updates"; switched off, kyrr only
  asks when you click "Check for updates"). A new version shows as a small
  green note at the bottom of the main window and in the tray menu.
  "Update" downloads it, checks it against the release's SHA256 and
  installs it with only a progress window: Windows asks for permission
  once, and kyrr restarts by itself. The note's × hides it until the next
  version.
- The settings page shows whole where the screen has room (up to 1100
  pixels tall instead of 900), without scrolling.
- This works from 0.3.1 on: coming from 0.3.0, install this version by hand
  once more.

## 0.3.0 (2026-10-11)

- Listen to yourself: a new button under Check plays your microphone live
  on your headphones, the way your apps hear it.
- The before/after test only records now; play the take with Before and
  After whenever you like, and delete it with Delete recording.
- The settings show in two columns side by side, in a wider window that
  grows up to 900 pixels tall where the screen has room and moves instead
  of hiding behind the taskbar. The main view stays narrow.
- Even out volume starts at Medium. Who never changed it gets Medium too.
- With noise removal on, the main view says "Just your voice." instead of
  the microphone's name.
- The button at the bottom of the window only puts it away, like the close
  button. Quit kyrr from the tray icon's menu (right-click).
- Opening kyrr from the tray always brings its window to the front, at the
  size its content needs. It sometimes opened behind the current window,
  or too small after starting with Windows.
- Windows on ARM (Snapdragon and other ARM processors): kyrr no longer sets
  up its noise filter there, which the audio engine cannot load and which
  could leave the microphone silent. Dictation works; installing this
  version also removes the filter an earlier one put there.
- The speech models download from kyrr's own mirrors, on Hugging Face and,
  if that fails, on GitHub; the English model no longer depends on someone
  else's repository (that stays the last resort). Each file is still
  checked against its built-in SHA256.
- The level meter only runs while the window is open.

## 0.2.1 (2026-10-10)

- Dictation in German works on older processors too (Intel before
  Ice Lake/Alder Lake, AMD before Zen 4). On these it typed nothing or a
  stray word. kyrr fetches the corrected speech model by itself (about
  900 MB) and deletes the old one.
- The before/after test records ten seconds instead of five.
- Dictated text is pasted instead of typed key by key, so it arrives whole
  in every app (Windows 11 Notepad dropped words), and "new line" makes a
  new line in chat apps instead of sending the message. The clipboard is put
  back afterwards, and the dictated text stays out of the clipboard history.
- Line breaks also with "nächste Zeile", "nächster Absatz", "Zeilenumbruch",
  "next line", "next paragraph", "newline" and "line break".
- The installer thanks the projects kyrr builds on, on its last page and in
  the NOTICE file next to the app.

## 0.2.0 (2026-10-10)

First public release.

- Remove background noise: DPDFNet inside the Windows audio engine, for every
  app; strength gentle/medium/strong, a large and a small model.
- Voice: neutral, warm, clear, bright or your own sound, even volume, softer
  S sounds, low cut.
- Dictation: hold F9, speak, release; German and English, on your computer.
- Check: live level and a five-second before/after test.
- Check for updates, only when you click it.

Thanks to Fovty (HushMic), Ceva (DPDFNet), primeLine AI (German model),
NVIDIA (Parakeet) and the sherpa-onnx project; see the README.
