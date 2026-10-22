pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// PickScribe bar icon and panel. All daemon traffic goes through argv
// arrays, never a shell, because history text and terms are user text.
Panel {
  id: root
  moduleName: "pickforge.pickscribe"
  ipcTarget: "pickforge.pickscribe"
  manageIpc: false

  readonly property string executable: String(setting("executable", "pickscribe") || "pickscribe")
  readonly property string configPath: Quickshell.env("HOME") + "/.config/pickscribe/pickscribe.toml"

  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color accent: Commons.Color.accent
  readonly property color urgent: bar ? bar.urgent : Commons.Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property string glyphMic: String.fromCodePoint(0xF036C)
  readonly property string glyphMicOff: String.fromCodePoint(0xF036D)
  readonly property string glyphDelete: String.fromCodePoint(0xF01B4)
  readonly property string glyphClose: String.fromCodePoint(0xF0156)
  readonly property string glyphRecord: String.fromCodePoint(0xF044A)
  readonly property string glyphStop: String.fromCodePoint(0xF04DB)

  // Daemon state from `pickscribe watch`.
  property bool daemonUp: false
  property var snapshot: null
  readonly property string daemonState: daemonUp && snapshot ? String(snapshot.state || "idle") : "offline"
  readonly property bool listening: daemonState === "listening"
  readonly property bool thinking: daemonState === "thinking"
  readonly property bool offline: daemonState === "offline"
  readonly property bool rawMode: !!(snapshot && snapshot.raw)
  readonly property var last: snapshot && snapshot.last ? snapshot.last : null
  property var historyRev: undefined
  property var dictionaryRev: undefined
  property var suggestionCount: undefined

  property var history: []
  property var dictionary: []
  property var suggestions: []

  property string errorText: ""
  property string copiedId: ""
  property bool confirmClear: false

  function stateLabel() {
    if (offline) return "Not running"
    if (listening) return rawMode ? "Listening, raw" : "Listening"
    if (thinking) return "Thinking"
    return "Idle"
  }

  // `env` turns a missing binary into a quiet exit 127 instead of a
  // Quickshell start warning on every retry.
  function command(program, args) { return ["env", "--", program].concat(args) }

  function versionText() {
    if (!snapshot) return ""
    var parts = []
    if (snapshot.version) parts.push("v" + snapshot.version)
    if (snapshot.commit) parts.push(String(snapshot.commit).slice(0, 7))
    return parts.join(" ")
  }

  function acceptSnapshot(line) {
    var text = String(line || "").trim()
    if (text === "") return
    var value
    try { value = JSON.parse(text) } catch (e) { return }
    if (!value || typeof value !== "object") return
    snapshot = value
    daemonUp = true
    if (value.history_rev !== historyRev) {
      historyRev = value.history_rev
      historyReader.load()
      suggestionReader.load()
    }
    if (value.dictionary_rev !== dictionaryRev) {
      dictionaryRev = value.dictionary_rev
      dictReader.load()
    }
    if (value.suggestions !== suggestionCount) {
      suggestionCount = value.suggestions
      suggestionReader.load()
    }
  }

  // Accepting adds the correction to the dictionary; ignoring means it is never suggested again.
  function answerSuggestion(suggestion, accept) {
    runAction(["suggestions", accept ? "accept" : "ignore", String(suggestion.id)], function() {
      suggestionReader.load()
      dictReader.load()
    })
  }

  function parseList(text, what) {
    var value = null
    try { value = JSON.parse(text) } catch (e) { value = null }
    if (Array.isArray(value)) return value
    showError("Unreadable " + what + " from pickscribe")
    return []
  }

  function showError(message) {
    var text = String(message || "").trim()
    errorText = text === "" ? "Command failed" : text.split("\n")[0]
    errorTimer.restart()
  }

  // One action at a time, in order. Each job is an argv tail plus an
  // optional success callback.
  property var actionQueue: []
  property var currentAction: null

  function runAction(args, onSuccess) {
    actionQueue = actionQueue.concat([{ args: args, onSuccess: onSuccess || null }])
    pumpActions()
  }

  function pumpActions() {
    if (actionProc.running || actionQueue.length === 0) return
    currentAction = actionQueue[0]
    actionQueue = actionQueue.slice(1)
    actionProc.command = root.command(root.executable, currentAction.args)
    actionProc.running = true
  }

  function toggleRecording() {
    root.close()
    stopTimer.restart()
  }

  Timer {
    id: stopTimer
    interval: 100
    onTriggered: root.runAction(["toggle"])
  }
  function cancelRecording() { runAction(["cancel"]) }

  function copyEntry(entry) {
    var id = String(entry.id)
    runAction(["history", "copy", id], function() {
      root.copiedId = id
      copiedTimer.restart()
    })
  }

  function deleteEntry(entry) {
    runAction(["history", "delete", String(entry.id)], function() { historyReader.load() })
  }

  function clearHistory() {
    confirmClear = false
    runAction(["history", "clear"], function() { historyReader.load() })
  }

  function entryLabel(entry) {
    var heard = entry && entry.heard !== null && entry.heard !== undefined ? String(entry.heard) : ""
    return heard !== "" ? heard + " → " + entry.text : String(entry.text)
  }

  function entryArgs(entry) {
    var args = [String(entry.text)]
    if (entry.heard !== null && entry.heard !== undefined && String(entry.heard) !== "")
      args.push("--heard", String(entry.heard))
    return args
  }

  function addEntry() {
    var text = termField.text.trim()
    var heard = heardField.text.trim()
    if (text === "") return
    var args = ["dict", "add", text]
    if (heard !== "") args.push("--heard", heard)
    runAction(args, function() {
      termField.text = ""
      heardField.text = ""
      dictReader.load()
    })
  }

  function removeEntry(entry) {
    runAction(["dict", "remove"].concat(entryArgs(entry)), function() { dictReader.load() })
  }

  function openSettings() {
    Quickshell.execDetached(["omarchy-launch-editor", root.configPath])
    root.close()
  }

  function formatMs(ms) {
    var n = Number(ms)
    if (!isFinite(n) || n < 0) return "-"
    if (n >= 10000) return (n / 1000).toFixed(0) + " s"
    if (n >= 1000) return (n / 1000).toFixed(1) + " s"
    return Math.round(n) + " ms"
  }

  function formatTime(at) {
    var date = new Date(Number(at) * 1000)
    if (isNaN(date.getTime())) return ""
    var now = new Date()
    if (date.toDateString() === now.toDateString()) return Qt.formatDateTime(date, "HH:mm")
    if (date.getFullYear() === now.getFullYear()) return Qt.formatDateTime(date, "d MMM")
    return Qt.formatDateTime(date, "d MMM yy")
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onOpenedChanged: if (opened) {
    confirmClear = false
    if (panelFlick) panelFlick.contentY = 0
    historyReader.load()
    dictReader.load()
    suggestionReader.load()
  }

  onExecutableChanged: {
    daemonUp = false
    if (watchProc.running) watchProc.running = false
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function state(): string {
      return JSON.stringify({
        state: root.daemonState, opened: root.opened, executable: root.executable,
        history: root.history.length, dictionary: root.dictionary.length, suggestions: root.suggestions.length,
        error: root.errorText
      })
    }
  }

  // ---------------------------------------------------------------- processes

  Process {
    id: watchProc
    command: root.command(root.executable, ["watch"])
    running: true
    stdout: SplitParser {
      onRead: function(line) { root.acceptSnapshot(line) }
    }
    onRunningChanged: if (!running) root.daemonUp = false
  }

  // Restarts `watch` 2 s after it stops, or when the command never started.
  Timer {
    interval: 2000
    repeat: true
    running: !watchProc.running
    onTriggered: watchProc.running = true
  }

  Process {
    id: actionProc
    stdout: StdioCollector {}
    stderr: StdioCollector { id: actionError }
    onExited: function(code) {
      var job = root.currentAction
      root.currentAction = null
      if (code === 0) {
        if (job && job.onSuccess) job.onSuccess()
      } else {
        root.showError(actionError.text || ("pickscribe " + (job ? job.args[0] : "") + " failed"))
      }
      Qt.callLater(root.pumpActions)
    }
  }

  JsonReader {
    id: historyReader
    args: root.command(root.executable, ["history", "--json"])
    onLoaded: function(text) { root.history = root.parseList(text, "history") }
    onFailed: function(message) { if (root.daemonUp) root.showError(message) }
  }

  JsonReader {
    id: dictReader
    args: root.command(root.executable, ["dict", "--json"])
    onLoaded: function(text) { root.dictionary = root.parseList(text, "dictionary") }
    onFailed: function(message) { if (root.daemonUp) root.showError(message) }
  }

  JsonReader {
    id: suggestionReader
    args: root.command(root.executable, ["suggestions", "--json"])
    onLoaded: function(text) { root.suggestions = root.parseList(text, "suggestions") }
    onFailed: function(message) { if (root.daemonUp) root.showError(message) }
  }

  Timer { id: errorTimer; interval: 6000; onTriggered: root.errorText = "" }
  Timer { id: copiedTimer; interval: 1500; onTriggered: root.copiedId = "" }

  // ---------------------------------------------------------------- bar icon

  // Listening: a quick, shallow pulse. Thinking: a slow, deeper breath.
  // The pulse fades the glyph color so the button keeps its own opacity.
  property real pulse: 1.0
  readonly property color iconColor: root.listening && !root.rawMode ? root.accent : root.barForeground

  SequentialAnimation {
    running: root.listening
    loops: Animation.Infinite
    NumberAnimation { target: root; property: "pulse"; to: 0.55; duration: 650; easing.type: Easing.InOutSine }
    NumberAnimation { target: root; property: "pulse"; to: 1.0; duration: 650; easing.type: Easing.InOutSine }
    onStopped: root.pulse = 1
  }

  SequentialAnimation {
    running: root.thinking
    loops: Animation.Infinite
    NumberAnimation { target: root; property: "pulse"; to: 0.3; duration: 1100; easing.type: Easing.InOutSine }
    NumberAnimation { target: root; property: "pulse"; to: 1.0; duration: 1100; easing.type: Easing.InOutSine }
    onStopped: root.pulse = 1
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.offline ? root.glyphMicOff : root.glyphMic
    dimmed: root.offline
    foreground: Qt.rgba(root.iconColor.r, root.iconColor.g, root.iconColor.b, root.pulse)
    tooltipText: "PickScribe: " + root.stateLabel()
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.MiddleButton) root.toggleRecording()
      else if (buttonCode === Qt.LeftButton) root.toggle()
    }
  }

  // ---------------------------------------------------------------- panel

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(680))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      blocked: termField.activeFocus || heardField.activeFocus

      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onMoveRequested: function(dx, dy) {
        if (dy !== 0)
          panelFlick.contentY = Math.max(0, Math.min(panelFlick.contentY + dy * Style.space(56),
                                                     panelFlick.contentHeight - panelFlick.height))
      }

      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(12)

          // ---------- Header ----------
          PanelHero {
            width: parent.width
            title: "PickScribe"
            meta: root.stateLabel()
            detail: root.versionText()
            foreground: root.foreground
            fontFamily: root.fontFamily

            iconComponent: Component {
              Text {
                textFormat: Text.PlainText
                text: root.offline ? root.glyphMicOff : root.glyphMic
                color: root.listening ? root.accent : root.foreground
                opacity: root.offline ? 0.45 : root.pulse
                font.family: root.fontFamily
                font.pixelSize: Style.font.display
              }
            }
          }

          Text {
            textFormat: Text.PlainText
            width: parent.width
            visible: !!root.snapshot && root.daemonUp
            text: root.snapshot && root.snapshot.api_key ? "API key set, AI cleanup is on"
                                                         : "No API key, Right Alt pastes raw text"
            color: root.snapshot && root.snapshot.api_key ? root.dim : root.urgent
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }

          Text {
            textFormat: Text.PlainText
            width: parent.width
            visible: root.offline
            text: "The daemon is not running. Start pickscribe daemon to dictate."
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }

          Row {
            width: parent.width
            spacing: Style.spacing.md

            readonly property bool twoButtons: root.listening
            readonly property real cellWidth: twoButtons ? (width - spacing) / 2 : width
            // The record button carries an icon that is taller than the
            // text, so both buttons take the taller height.
            readonly property real buttonHeight: Math.max(recordButton.implicitHeight, cancelButton.implicitHeight)

            Button {
              id: recordButton
              width: parent.cellWidth
              height: parent.buttonHeight
              bordered: true
              enabled: !root.offline && !root.thinking
              opacity: enabled ? 1 : 0.5
              iconText: root.listening ? root.glyphStop : root.glyphRecord
              text: root.listening ? "Stop and paste" : (root.thinking ? "Thinking" : "Record")
              foreground: root.listening ? root.accent : root.foreground
              fontFamily: root.fontFamily
              onClicked: root.toggleRecording()
            }

            Button {
              id: cancelButton
              visible: parent.twoButtons
              width: parent.cellWidth
              height: parent.buttonHeight
              bordered: true
              text: "Cancel"
              foreground: root.foreground
              fontFamily: root.fontFamily
              onClicked: root.cancelRecording()
            }
          }

          Text {
            textFormat: Text.PlainText
            width: parent.width
            visible: root.errorText !== ""
            text: root.errorText
            color: root.urgent
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }

          // ---------- Last dictation ----------
          PanelSeparator { foreground: root.foreground }

          Column {
            width: parent.width
            spacing: Style.space(8)

            Item {
              width: parent.width
              implicitHeight: lastHeader.implicitHeight

              PanelSectionHeader {
                id: lastHeader
                text: "LAST DICTATION"
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              ModeBadge {
                visible: !!root.last
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                mode: root.last ? String(root.last.mode || "") : ""
              }
            }

            Text {
              visible: !root.last
              textFormat: Text.PlainText
              text: "No dictation yet"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
            }

            Row {
              id: timingRow
              visible: !!root.last
              width: parent.width
              spacing: Style.spacing.md
              readonly property real cellWidth: (width - spacing * 3) / 4

              TimingCell { label: "Audio"; value: root.last ? root.formatMs(root.last.audio_ms) : "" }
              TimingCell { label: "Transcribe"; value: root.last ? root.formatMs(root.last.transcribe_ms) : "" }
              TimingCell {
                label: "Cleanup"
                value: !root.last ? "" : (root.last.mode === "raw" && !(root.last.cleanup_ms > 0) ? "-" : root.formatMs(root.last.cleanup_ms))
              }
              TimingCell { label: "Total"; value: root.last ? root.formatMs(root.last.total_ms) : ""; strong: true }
            }
          }

          // ---------- Suggestions ----------
          PanelSeparator {
            visible: root.suggestions.length > 0
            foreground: root.foreground
          }

          Column {
            visible: root.suggestions.length > 0
            width: parent.width
            spacing: Style.space(8)

            PanelSectionHeader {
              text: "DID YOU MEAN"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Repeater {
              model: root.suggestions
              SuggestionRow {
                required property var modelData
                width: parent.width
                suggestion: modelData
              }
            }
          }

          // ---------- History ----------
          PanelSeparator { foreground: root.foreground }

          Column {
            width: parent.width
            spacing: Style.space(6)

            Item {
              width: parent.width
              implicitHeight: Math.max(historyHeader.implicitHeight, clearRow.implicitHeight)

              PanelSectionHeader {
                id: historyHeader
                anchors.verticalCenter: parent.verticalCenter
                text: "HISTORY"
                foreground: root.foreground
                fontFamily: root.fontFamily
              }

              Row {
                id: clearRow
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: Style.spacing.sm
                visible: root.history.length > 0

                Text {
                  visible: root.confirmClear
                  anchors.verticalCenter: parent.verticalCenter
                  textFormat: Text.PlainText
                  text: "Clear all " + root.history.length + "?"
                  color: root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                }

                SmallButton {
                  text: root.confirmClear ? "Clear" : "Clear history"
                  tint: root.confirmClear ? root.urgent : root.dim
                  onClicked: root.confirmClear ? root.clearHistory() : (root.confirmClear = true)
                }

                SmallButton {
                  visible: root.confirmClear
                  text: "Cancel"
                  tint: root.dim
                  onClicked: root.confirmClear = false
                }
              }
            }

            Text {
              visible: root.history.length === 0
              textFormat: Text.PlainText
              text: root.offline ? "History shows up when the daemon runs" : "Nothing dictated yet"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
            }

            ListView {
              id: historyList
              visible: root.history.length > 0
              width: parent.width
              height: Math.min(contentHeight, Style.space(250))
              clip: true
              spacing: Style.space(2)
              boundsBehavior: Flickable.StopAtBounds
              interactive: contentHeight > height
              model: root.history
              ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

              delegate: HistoryRow {
                required property var modelData
                width: historyList.width - Style.space(8)
                entry: modelData
              }
            }
          }

          // ---------- Dictionary ----------
          PanelSeparator { foreground: root.foreground }

          Column {
            width: parent.width
            spacing: Style.space(8)

            PanelSectionHeader {
              text: "DICTIONARY"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Text {
              visible: root.dictionary.length === 0
              textFormat: Text.PlainText
              text: "No terms yet. Add names and words Whisper gets wrong."
              width: parent.width
              wrapMode: Text.WordWrap
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
            }

            Flow {
              visible: root.dictionary.length > 0
              width: parent.width
              spacing: Style.spacing.sm

              Repeater {
                model: root.dictionary
                DictChip {
                  required property var modelData
                  entry: modelData
                }
              }
            }

            Row {
              width: parent.width
              spacing: Style.spacing.sm
              readonly property real fieldWidth: (width - addButton.width - spacing * 2) / 2

              TextField {
                id: termField
                width: parent.fieldWidth
                placeholderText: "Term"
                foreground: root.foreground
                font.family: root.fontFamily
                onAccepted: root.addEntry()
                Keys.onEscapePressed: keyCatcher.forceActiveFocus()
              }

              TextField {
                id: heardField
                width: parent.fieldWidth
                placeholderText: "Heard as (optional)"
                foreground: root.foreground
                font.family: root.fontFamily
                onAccepted: root.addEntry()
                Keys.onEscapePressed: keyCatcher.forceActiveFocus()
              }

              Button {
                id: addButton
                height: termField.height
                bordered: true
                text: "Add"
                enabled: termField.text.trim() !== ""
                opacity: enabled ? 1 : 0.5
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: root.addEntry()
              }
            }
          }

          // ---------- Footer ----------
          PanelSeparator { foreground: root.foreground }

          Item {
            width: parent.width
            implicitHeight: settingsButton.implicitHeight

            Button {
              id: settingsButton
              anchors.left: parent.left
              iconText: String.fromCodePoint(0xF0493)
              text: "Settings"
              foreground: root.foreground
              fontFamily: root.fontFamily
              fontSize: Style.font.bodySmall
              tooltipText: root.configPath
              onClicked: root.openSettings()
            }
          }
        }
      }
    }
  }

  // ---------------------------------------------------------------- components

  // Runs a command that prints JSON. A load that arrives while one is in
  // flight runs once more after it, so the result is never stale.
  component JsonReader: Process {
    id: reader
    property var args: []
    property bool again: false
    // Carries the raw text: a parsed array passed through a var signal
    // argument arrives as a list that Array.isArray rejects.
    signal loaded(string text)
    signal failed(string message)

    function load() {
      if (running) { again = true; return }
      command = args
      running = true
    }

    stdout: StdioCollector { id: readerOut }
    stderr: StdioCollector { id: readerErr }
    onExited: function(code) {
      if (code === 0) {
        reader.loaded(readerOut.text)
      } else {
        reader.failed(readerErr.text || (reader.args.slice(2).join(" ") + " failed"))
      }
      if (again) { again = false; Qt.callLater(reader.load) }
    }
  }

  component TimingCell: Column {
    property string label: ""
    property string value: ""
    property bool strong: false
    width: timingRow.cellWidth
    spacing: Style.space(1)

    Text {
      textFormat: Text.PlainText
      text: parent.value
      color: root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.subtitle
      font.bold: parent.strong
    }
    Text {
      textFormat: Text.PlainText
      text: parent.label
      color: root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.caption
    }
  }

  component ModeBadge: BorderSurface {
    property string mode: ""
    readonly property bool ai: mode === "ai"
    implicitWidth: badgeText.implicitWidth + Style.space(8)
    implicitHeight: badgeText.implicitHeight + Style.space(2)
    color: ai ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.14) : "transparent"
    borderSpec: Border.flat(ai ? Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.55)
                              : Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.3), 1)
    radius: Style.cornerRadius

    Text {
      id: badgeText
      anchors.centerIn: parent
      textFormat: Text.PlainText
      text: parent.ai ? "AI" : "RAW"
      color: parent.ai ? root.accent : root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: true
    }
  }

  component SmallButton: Text {
    id: small
    property color tint: root.dim
    signal clicked()
    textFormat: Text.PlainText
    color: smallMouse.containsMouse ? root.foreground : tint
    font.family: root.fontFamily
    font.pixelSize: Style.font.caption
    font.underline: smallMouse.containsMouse

    MouseArea {
      id: smallMouse
      anchors.fill: parent
      anchors.margins: -Style.space(3)
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: small.clicked()
    }
  }

  component HistoryRow: Item {
    id: row
    property var entry: ({})
    readonly property bool copied: root.copiedId !== "" && root.copiedId === String(entry.id)
    readonly property bool notPasted: entry.delivered === false

    implicitHeight: Math.max(meta.implicitHeight, body.implicitHeight) + Style.space(12)

    Rectangle {
      anchors.fill: parent
      radius: Style.cornerRadius
      color: rowMouse.containsMouse || deleteButton.hovering ? Style.hoverFillFor(root.foreground, root.accent) : "transparent"
      Behavior on color { ColorAnimation { duration: 80 } }
    }

    MouseArea {
      id: rowMouse
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: root.copyEntry(row.entry)
    }

    Column {
      id: meta
      anchors.left: parent.left
      anchors.leftMargin: Style.space(6)
      anchors.top: parent.top
      anchors.topMargin: Style.space(6)
      width: Style.space(58)
      spacing: Style.space(3)

      Text {
        textFormat: Text.PlainText
        text: row.copied ? "Copied" : root.formatTime(row.entry.at)
        color: row.copied ? root.accent : root.dim
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: row.copied
      }

      ModeBadge { mode: String(row.entry.mode || "") }
    }

    Column {
      id: body
      anchors.left: meta.right
      anchors.leftMargin: Style.space(6)
      anchors.right: deleteButton.left
      anchors.rightMargin: Style.space(4)
      anchors.top: parent.top
      anchors.topMargin: Style.space(6)
      spacing: Style.space(2)

      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: String(row.entry.text || "")
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.Wrap
        maximumLineCount: 2
        elide: Text.ElideRight
      }

      Text {
        visible: row.notPasted
        textFormat: Text.PlainText
        text: "Not pasted, click to copy"
        color: root.accent
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
      }
    }

    PanelActionButton {
      id: deleteButton
      property bool hovering: false
      anchors.right: parent.right
      anchors.rightMargin: Style.space(2)
      anchors.verticalCenter: parent.verticalCenter
      iconText: root.glyphDelete
      tooltipText: "Delete"
      foreground: root.dim
      hoverColor: root.urgent
      fontFamily: root.fontFamily
      opacity: rowMouse.containsMouse || hovering ? 1 : 0.6
      onHovered: function(isHovered) { hovering = isHovered }
      onClicked: root.deleteEntry(row.entry)
    }
  }

  component SuggestionRow: Item {
    id: suggestionRow
    property var suggestion: ({})
    implicitHeight: suggestionText.implicitHeight + Style.space(4)

    Column {
      id: suggestionText
      anchors.left: parent.left
      anchors.right: suggestionButtons.left
      anchors.rightMargin: Style.spacing.sm
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(2)

      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: String(suggestionRow.suggestion.heard || "") + " → " + String(suggestionRow.suggestion.meant || "")
        elide: Text.ElideRight
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
      }

      Text {
        width: parent.width
        textFormat: Text.PlainText
        text: String(suggestionRow.suggestion.context || "")
        wrapMode: Text.WordWrap
        maximumLineCount: 2
        elide: Text.ElideRight
        color: root.dim
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }
    }

    Row {
      id: suggestionButtons
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.spacing.sm

      Button {
        bordered: true
        text: "Add"
        foreground: root.foreground
        fontFamily: root.fontFamily
        onClicked: root.answerSuggestion(suggestionRow.suggestion, true)
      }

      Button {
        bordered: true
        text: "Ignore"
        foreground: root.foreground
        fontFamily: root.fontFamily
        onClicked: root.answerSuggestion(suggestionRow.suggestion, false)
      }
    }
  }

  component DictChip: BorderSurface {
    id: chip
    property var entry: ({})
    readonly property bool correction: entry.heard !== null && entry.heard !== undefined && String(entry.heard) !== ""

    implicitWidth: chipRow.implicitWidth + Style.space(8) + Style.space(2)
    implicitHeight: chipRow.implicitHeight + Style.space(4)
    color: Style.normalFillFor(root.foreground, root.accent)
    borderSpec: Border.controlSpec("normal", root.foreground, root.accent)
    radius: Style.cornerRadius

    Row {
      id: chipRow
      anchors.left: parent.left
      anchors.leftMargin: Style.space(8)
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(2)

      Text {
        anchors.verticalCenter: parent.verticalCenter
        visible: chip.correction
        textFormat: Text.PlainText
        text: chip.correction ? String(chip.entry.heard) + " →" : ""
        color: root.dim
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      Text {
        anchors.verticalCenter: parent.verticalCenter
        textFormat: Text.PlainText
        text: String(chip.entry.text || "")
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      PanelActionButton {
        anchors.verticalCenter: parent.verticalCenter
        iconText: root.glyphClose
        tooltipText: "Remove"
        size: Style.space(18)
        fontSize: Style.font.bodySmall
        foreground: root.dim
        hoverColor: root.urgent
        fontFamily: root.fontFamily
        onClicked: root.removeEntry(chip.entry)
      }
    }
  }
}
