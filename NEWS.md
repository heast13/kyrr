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
