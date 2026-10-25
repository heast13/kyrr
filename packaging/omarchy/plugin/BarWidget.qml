import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// kyrr in Omarchy's bar. While kyrr runs it keeps one word in
// $XDG_RUNTIME_DIR/kyrr/state: idle, listening, writing or recording (a voice
// memo). No file: kyrr does not run.
BarWidget {
  id: root
  moduleName: "heast13.kyrr"

  readonly property string statePath: (Quickshell.env("XDG_RUNTIME_DIR") || "/tmp") + "/kyrr/state"
  property string kyrrState: ""
  readonly property bool german: Qt.locale().name.indexOf("de") === 0
  readonly property bool busy: kyrrState === "listening" || kyrrState === "writing" || kyrrState === "recording"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // kyrr as a command (AUR package, ~/.local/bin), else its app menu entry.
  readonly property string openCommand: "command -v kyrr >/dev/null && exec kyrr; "
    + "exec gio launch \"${XDG_DATA_HOME:-$HOME/.local/share}/applications/io.github.heast13.kyrr.desktop\""
  readonly property string dictateCommand: "command -v kyrr >/dev/null && exec kyrr --dictate"

  function say(de, en) {
    return german ? de : en
  }

  FileView {
    id: stateFile
    path: root.statePath
    watchChanges: true
    printErrors: false
    onLoaded: {
      var word = text().trim()
      // A read in the middle of kyrr's write: keep what was there.
      if (word !== "") root.kyrrState = word
    }
    onLoadFailed: root.kyrrState = ""
    onFileChanged: reload()
  }

  // The file comes and goes with kyrr, which a watch on it does not see.
  Timer {
    interval: 2000
    running: true
    repeat: true
    onTriggered: stateFile.reload()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // The waveform icon of Material Design Icons, as in Nerd Fonts.
    text: String.fromCodePoint(0xF147D)
    active: root.busy
    activeColor: root.kyrrState === "recording" ? Commons.Color.urgent : Commons.Color.accent
    dimmed: root.kyrrState === ""
    tooltipText: root.kyrrState === "" ? root.say("kyrr starten", "Start kyrr")
      : root.kyrrState === "listening" ? root.say("kyrr hört zu", "kyrr is listening")
      : root.kyrrState === "writing" ? root.say("kyrr schreibt", "kyrr is writing")
      : root.kyrrState === "recording" ? root.say("kyrr nimmt auf", "kyrr is recording")
      : root.say("kyrr: bereit. Mittelklick diktiert.", "kyrr: ready. Middle click dictates.")
    onPressed: function(b) {
      Quickshell.execDetached(["sh", "-c", b === Qt.MiddleButton ? root.dictateCommand : root.openCommand])
    }
  }
}
