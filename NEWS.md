# What's new

What kyrr shows under "What's new" next to the update note: a few plain
sentences, only what a user notices (new things, changed handling, bugs they
ran into), no internals. English first, then `### Deutsch`. Name the section
after the version when it is released (`## 0.6.0`, say); `scripts/release-notes.py`
puts it into the release description, where kyrr reads it. Never put `-->` in
the text.

An item for one system only starts with `(Windows)` or `(Linux)`, as in
`- (Linux) …`; kyrr shows it only there, without the mark. Everything else
is for both, so it names neither. Linux lists the versions from 0.5.3 on.

Every version needs at least one item Windows sees (one for both, or one
marked `(Windows)`), or the Windows test fails. A release that only changes
Linux still gets a plain line like `(Windows) General bug fixes.`, never a
line saying nothing changes on Windows.

## Unreleased

- While you dictate, a small wave shows at the bottom of the screen: it moves with your voice while kyrr listens and runs faster while kyrr writes.
- Dictating works for up to 15 minutes at a stretch now, not just 2. A long dictation can take a while to be written after you let go.
- The short explanation in the dictation setup reads simpler now.
- (Linux) On Hyprland, kyrr opened from the tray appears as a floating window instead of a tiled one.

### Deutsch

- Während du diktierst, zeigt sich unten am Bildschirm eine kleine Welle: Sie bewegt sich mit deiner Stimme, solange kyrr zuhört, und läuft schneller, während kyrr schreibt.
- Diktieren geht jetzt bis zu 15 Minuten am Stück, nicht nur 2. Nach dem Loslassen kann es bei langen Diktaten dauern, bis der Text da ist.
- Die kurze Erklärung beim Einrichten des Diktierens ist jetzt einfacher formuliert.
- (Linux) Unter Hyprland erscheint kyrr aus dem Tray als schwebendes Fenster statt gekachelt.

## 0.5.5

- (Linux) Dictating letters your keyboard layout does not have, like umlauts on a US keyboard, now lands in the window by itself, without Ctrl+V.
- (Linux) The window's X minimizes kyrr every time, not just the first time.
- (Linux) Going back from advanced to simple settings makes the window smaller again.
- (Windows) General bug fixes.

### Deutsch

- (Linux) Diktierst du Buchstaben, die deine Tastaturbelegung nicht hat, etwa Umlaute auf einer US-Tastatur, landet der Text jetzt von selbst im Fenster, ohne Strg+V.
- (Linux) Das X des Fensters minimiert kyrr jedes Mal, nicht nur beim ersten Mal.
- (Linux) Zurück von erweiterten auf einfache Einstellungen wird das Fenster wieder kleiner.
- (Windows) Allgemeine Fehlerbehebungen.

## 0.5.4

- (Linux) On Hyprland (Omarchy, say), Sway, river and niri kyrr now sets up the dictation key itself, and the start at sign-in where your desktop needs it. On GNOME 46 and 47 it adds the key to GNOME's own keyboard shortcuts.
- (Linux) No PipeWire on your system? "Set up PipeWire" installs it for you.
- (Linux) "Off to the corner" only shows where it can do something, and kyrr no longer crashes when you sign out.
- Where something needs your help, kyrr says plainly what to do, without file names or error codes.
- If you unplug a microphone just as kyrr moves to it, kyrr stays on the one you had.
- The buttons under Settings, About no longer get cut off.
- The status line has a few new words to pass the time. Burpinsured, for one.

### Deutsch

- (Linux) Unter Hyprland (etwa Omarchy), Sway, river und niri richtet kyrr die Diktiertaste jetzt selbst ein, und wo dein Desktop es braucht auch den Start bei der Anmeldung. Unter GNOME 46 und 47 trägt es die Taste in GNOMEs eigene Tastenkürzel ein.
- (Linux) Kein PipeWire auf deinem System? „PipeWire einrichten“ installiert es für dich.
- (Linux) „Ab in die Ecke“ erscheint nur noch, wo es etwas bewirkt, und kyrr stürzt beim Abmelden nicht mehr ab.
- Wo kyrr deine Hilfe braucht, sagt es einfach, was zu tun ist, ohne Dateinamen oder Fehlercodes.
- Steckst du ein Mikrofon genau dann aus, wenn kyrr gerade darauf wechselt, bleibt kyrr beim bisherigen.
- Die Knöpfe unter Einstellungen, Über kyrr werden nicht mehr abgeschnitten.
- Die Statuszeile hat ein paar neue Wörter zum Zeitvertreib. Rülpsversichert, zum Beispiel.

## 0.5.3

- (Linux) kyrr is here for Linux: the noise removal works on a microphone of its own called "kyrr", which your apps get as their microphone, and dictation types into any window with F9.
- (Linux) On GNOME and KDE your desktop asks once for the key and once for permission to type; kyrr walks you through both.
- (Linux) kyrr follows your microphone when you plug in another one or pick one in the sound settings, and tells you when it is muted.
- (Linux) Without a tray icon, the window's X only minimizes kyrr where your desktop allows it; "Quit" ends it.
- (Windows) kyrr is now available for Linux too.
- (Windows) The small wave next to the status line needs far less processor time.

### Deutsch

- (Linux) kyrr gibt es jetzt für Linux: Die Entfernung von Störgeräuschen läuft über ein eigenes Mikrofon namens „kyrr“, das deine Apps als Mikrofon bekommen, und das Diktat tippt mit F9 in jedes Fenster.
- (Linux) Unter GNOME und KDE fragt dein Desktop einmal nach der Taste und einmal, ob kyrr tippen darf; kyrr führt dich durch beides.
- (Linux) kyrr folgt deinem Mikrofon, wenn du ein anderes einsteckst oder in den Toneinstellungen auswählst, und sagt dir, wenn es stummgeschaltet ist.
- (Linux) Ohne Tray-Symbol minimiert das X am Fenster kyrr nur, wo dein Desktop das kann; „Beenden“ beendet es.
- (Windows) kyrr gibt es jetzt auch für Linux.
- (Windows) Die kleine Welle neben der Statuszeile braucht viel weniger Rechenleistung.

## 0.5.2

- kyrr tells you when Windows has your microphone muted (or it sends nothing at all), and a click on "Switch it on" fixes it. It also says when no microphone is found and which one was just plugged in.
- New "Quit" button on the main page, bottom left.
- S sounds are softened a little and the automatic volume is on from the start.
- After an update kyrr shows what the new version brings, and Settings, About has a "What's new" button that lists every version, newest first.

### Deutsch

- kyrr sagt dir, wenn Windows dein Mikrofon stummgeschaltet hat (oder es gar nichts liefert), und ein Klick auf „Einschalten“ behebt das. Es meldet auch, wenn kein Mikrofon gefunden wird und welches gerade eingesteckt wurde.
- Neuer „Beenden“-Knopf unten links auf der Hauptseite.
- S-Laute werden von Anfang an leicht gedämpft und die automatische Lautstärke ist gleich an.
- Nach einem Update zeigt kyrr, was die neue Version bringt, und unter Einstellungen, Über gibt es einen Knopf „Was ist neu“, der alle Versionen auflistet, die neueste oben.

## 0.5.1

- kyrr now follows your microphone: plug in another one (or the same one in a different USB port) and, once Windows uses it as your microphone, the noise removal moves along by itself. Windows asks for permission once and the sound pauses for a moment. On a RØDE PodMic USB it used to stay silent; that works now.
- If Windows takes kyrr off your microphone (after an update, say), kyrr puts itself back.

### Deutsch

- kyrr folgt jetzt deinem Mikrofon: Steckst du ein anderes ein (oder dasselbe an einen anderen USB-Anschluss), zieht die Rauschunterdrückung von selbst mit, sobald Windows es als dein Mikrofon nimmt. Windows fragt einmal um Erlaubnis, der Ton pausiert kurz. Beim RØDE PodMic USB blieb sie vorher stumm, jetzt geht es.
- Nimmt Windows kyrr vom Mikrofon (zum Beispiel nach einem Update), setzt kyrr sich selbst wieder drauf.

## 0.5.0

- The main window keeps its height now, so it no longer gets pulled out of shape.
- A small red note on the main page tells you when your microphone is too quiet or too loud for the automatic volume, or when "Listen to yourself" is still running. The listen button turns red while it runs.
- The automatic volume shows what it does right now as +9 dB (green) or −9 dB (red).
- The advanced settings use the room better: the automatic volume moved to the right column and the window grows taller where the screen has room.

### Deutsch

- Das Hauptfenster behält jetzt seine Höhe und lässt sich nicht mehr verziehen.
- Eine kleine rote Zeile auf der Hauptseite sagt dir, wenn dein Mikro für die automatische Lautstärke zu leise oder zu laut ist, oder wenn „Mithören“ noch läuft. Der Mithören-Knopf ist rot, solange es läuft.
- Die automatische Lautstärke zeigt, was sie gerade tut: +9 dB (grün) oder −9 dB (rot).
- Die erweiterten Einstellungen nutzen den Platz besser: Die automatische Lautstärke wanderte in die rechte Spalte, und das Fenster wird höher, wo der Bildschirm Platz hat.

## 0.4.0

- Automatic volume (in Settings): your voice always comes out at the same level, however your microphone is set. It only listens while you speak.
- Pop protection softens the thump of P and B when you are close to the microphone.
- Settings come as Simple and Advanced: Simple shows the basics, Advanced everything.

### Deutsch

- Automatische Lautstärke (in den Einstellungen): Deine Stimme kommt immer gleich laut an, egal wie dein Mikrofon eingestellt ist. Sie hört nur zu, solange du sprichst.
- Der Poppschutz dämpft das Plopp von P und B, wenn du nah am Mikrofon bist.
- Die Einstellungen gibt es als Einfach und Erweitert: Einfach zeigt das Wichtigste, Erweitert alles.

## 0.3.1

- kyrr updates itself with one click: once a day it looks whether there is a new version, and "Update" installs it.
- The settings page now shows whole on bigger screens, without scrolling.

### Deutsch

- kyrr aktualisiert sich mit einem Klick: Einmal am Tag schaut es nach einer neuen Version, „Update“ installiert sie.
- Die Einstellungen sind auf größeren Bildschirmen jetzt ganz zu sehen, ohne Scrollen.

## 0.3.0

- "Listen to yourself": hear your microphone live on your headphones, the way your apps hear it.
- The before/after test only records now; play the take as often as you like.
- Windows on ARM: kyrr no longer sets up the noise filter there (it cannot run); dictation works.
- The window opens in front, at the right size.

### Deutsch

- „Mithören“: Du hörst dein Mikrofon live im Kopfhörer, so wie deine Apps es hören.
- Der Vorher/Nachher-Test nimmt nur noch auf, die Aufnahme spielst du beliebig oft ab.
- Windows auf ARM: kyrr richtet dort den Rauschfilter nicht mehr ein (er läuft dort nicht), das Diktat funktioniert.
- Das Fenster öffnet sich vorne und in der richtigen Größe.

## 0.2.1

- German dictation now works on older processors too.
- Dictated text is pasted instead of typed, so it arrives whole in every app and "new line" no longer sends chat messages.

### Deutsch

- Das deutsche Diktat funktioniert jetzt auch auf älteren Prozessoren.
- Diktierter Text wird eingefügt statt getippt, kommt also in jeder App vollständig an, und „neue Zeile“ schickt keine Chatnachrichten mehr ab.

## 0.2.0

- The first public version: noise removal for every app, voice sound, dictation in German and English, and a level check.

### Deutsch

- Die erste öffentliche Version: Rauschunterdrückung für jede App, Stimmklang, Diktat auf Deutsch und Englisch und eine Pegelprüfung.
