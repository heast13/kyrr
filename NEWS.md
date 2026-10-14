# What's new

What kyrr shows under "What's new" next to the update note: a few plain
sentences, only what a user notices (new things, changed handling, bugs they
ran into), no internals. English first, then `### Deutsch`. Name the section
after the version when it is released (`## 0.6.0`, say); `scripts/release-notes.py`
puts it into the release description, where kyrr reads it. Never put `-->` in
the text.

## 0.5.2

- kyrr tells you when Windows has your microphone muted (or it sends nothing at all), and a click on "Switch it on" fixes it. It also says when no microphone is found and which one was just plugged in.
- New "Quit" button on the main page, bottom left.
- S sounds are softened a little and the automatic volume is on from the start.

### Deutsch

- kyrr sagt dir, wenn Windows dein Mikrofon stummgeschaltet hat (oder es gar nichts liefert), und ein Klick auf „Einschalten“ behebt das. Es meldet auch, wenn kein Mikrofon gefunden wird und welches gerade eingesteckt wurde.
- Neuer „Beenden“-Knopf unten links auf der Hauptseite.
- S-Laute werden von Anfang an leicht gedämpft und die automatische Lautstärke ist gleich an.

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
