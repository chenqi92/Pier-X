import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier

// Streaming log viewer — M5b per-service tool.
//
// Layout
// ──────
//   ┌───────────────────────────────────────────────────┐
//   │ user@host   tail -f /var/log/syslog   ● live  ... │  top bar
//   ├───────────────────────────────────────────────────┤
//   │ 12:01 systemd: Started Session 42                 │
//   │ 12:01 kernel:  eth0 link up                       │  colored
//   │ 12:02 nginx:   [error] upstream timed out         │  by level
//   │ ...                                               │
//   └───────────────────────────────────────────────────┘
//
// The view is pure bindings + a single ListView backed by
// PierLogStream (a QAbstractListModel). It does NOT know how
// polling works — the C++ side owns the timer and just
// appends rows.
Rectangle {
    id: root

    // Backend params — same SSH field shape as TerminalView /
    // SftpBrowserView so Main.qml can feed every tab row
    // through the same Loader delegate. The remote command
    // is carried separately in `logCommand`.
    property string sshHost: ""
    property int    sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool   sshUsesAgent: false
    property string logCommand: "tail -f /var/log/syslog"

    // Local view state.
    property bool autoScroll: true

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierLogStream {
        id: stream
    }

    Component.onCompleted: _dispatchConnect()

    function _dispatchConnect() {
        if (root.sshHost.length === 0 || root.sshUser.length === 0) {
            console.warn("LogViewerView: missing host/user")
            return
        }
        var kind = 0
        var secret = ""
        var extra = ""
        if (root.sshUsesAgent) {
            kind = 3
        } else if (root.sshKeyPath.length > 0) {
            kind = 2
            secret = root.sshKeyPath
            extra = root.sshPassphraseCredentialId
        } else if (root.sshCredentialId.length > 0) {
            kind = 1
            secret = root.sshCredentialId
        } else {
            kind = 0
            secret = root.sshPassword
        }
        stream.connectTo(root.sshHost, root.sshPort, root.sshUser,
                         kind, secret, extra,
                         root.logCommand)
    }

    // Pick a colour for a log line. The C++ model classifies
    // each row as Stdout / Stderr / Exit / Error; we further
    // highlight stdout lines that look like they contain an
    // error or warning keyword so the usual syslog/nginx
    // patterns pop without needing a regex config.
    function _colorFor(kind, text) {
        if (kind === PierLogStream.ErrorKind) return Theme.statusError
        if (kind === PierLogStream.Exit)      return Theme.textTertiary
        if (kind === PierLogStream.Stderr)    return Theme.statusError

        var upper = text.toUpperCase()
        if (upper.indexOf("ERROR") >= 0 || upper.indexOf("FATAL") >= 0
            || upper.indexOf("FAIL") >= 0) {
            return Theme.statusError
        }
        if (upper.indexOf("WARN") >= 0) {
            return Theme.statusWarning
        }
        if (upper.indexOf("INFO") >= 0) {
            return Theme.textSecondary
        }
        return Theme.textPrimary
    }

    function _colorForLevel(level, kind, text) {
        if (kind === PierLogStream.Exit) return Theme.textTertiary
        if (level === PierLogStream.FatalLevel) return Theme.statusError
        if (level === PierLogStream.ErrorLevel) return Theme.statusError
        if (level === PierLogStream.WarnLevel) return Theme.statusWarning
        if (level === PierLogStream.InfoLevel) return Theme.textSecondary
        if (level === PierLogStream.DebugLevel) return Theme.textTertiary
        return _colorFor(kind, text)
    }

    function _badgeText(level) {
        if (level === PierLogStream.DebugLevel) return "DBG"
        if (level === PierLogStream.InfoLevel)  return "INF"
        if (level === PierLogStream.WarnLevel)  return "WRN"
        if (level === PierLogStream.ErrorLevel) return "ERR"
        if (level === PierLogStream.FatalLevel) return "FTL"
        return ""
    }

    function _badgeFill(level) {
        if (level === PierLogStream.DebugLevel) return Theme.bgHover
        if (level === PierLogStream.InfoLevel)  return Theme.accentSubtle
        if (level === PierLogStream.WarnLevel)  return Qt.rgba(Theme.statusWarning.r, Theme.statusWarning.g, Theme.statusWarning.b, 0.16)
        if (level === PierLogStream.ErrorLevel) return Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.14)
        if (level === PierLogStream.FatalLevel) return Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.22)
        return "transparent"
    }

    function _badgeTextColor(level) {
        if (level === PierLogStream.InfoLevel) return Theme.accent
        if (level === PierLogStream.WarnLevel) return Theme.statusWarning
        if (level === PierLogStream.ErrorLevel || level === PierLogStream.FatalLevel) return Theme.statusError
        return Theme.textTertiary
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // ─── Top bar ─────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Text {
                text: stream.target.length > 0 ? stream.target : qsTr("Log")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 120
                Layout.maximumWidth: 240

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 24
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusSm

                Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                Text {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2
                    verticalAlignment: Text.AlignVCenter
                    text: stream.command.length > 0
                          ? stream.command
                          : root.logCommand
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textSecondary
                    elide: Text.ElideRight

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            // Live dot — pulses green while the remote process
            // is running, goes gray when it exits.
            Rectangle {
                implicitWidth: 8
                implicitHeight: 8
                radius: 4
                color: stream.alive
                       ? Theme.statusSuccess
                       : Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                SequentialAnimation on opacity {
                    running: stream.alive
                    loops: Animation.Infinite
                    NumberAnimation { from: 1.0; to: 0.4; duration: 800 }
                    NumberAnimation { from: 0.4; to: 1.0; duration: 800 }
                }
            }

            Text {
                text: stream.alive
                      ? qsTr("live")
                      : (stream.status === PierLogStream.Finished
                         ? qsTr("exit %1").arg(stream.exitCode)
                         : qsTr("—"))
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            GhostButton {
                text: root.autoScroll ? qsTr("Auto ✓") : qsTr("Auto")
                onClicked: root.autoScroll = !root.autoScroll
            }

            GhostButton {
                text: qsTr("Clear")
                onClicked: stream.clear()
            }

            GhostButton {
                text: stream.alive ? qsTr("Stop") : qsTr("Retry")
                onClicked: {
                    if (stream.alive) {
                        stream.stop()
                    } else {
                        stream.clear()
                        _dispatchConnect()
                    }
                }
            }
        }

        // ─── Filter bar ──────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 36
            color: Theme.bgPanel
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                Text {
                    text: qsTr("Filter")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }

                PierTextField {
                    id: filterField
                    Layout.fillWidth: true
                    placeholder: qsTr("Search logs or enter a regex")
                    text: stream.filterText
                    onTextChanged: stream.filterText = text
                }

                Rectangle {
                    id: regexToggle
                    implicitWidth: regexLabel.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 22
                    radius: Theme.radiusSm
                    color: regexMouse.containsMouse
                           ? Theme.bgHover
                           : (stream.regexMode ? Theme.accentSubtle : "transparent")
                    border.color: stream.regexMode ? Theme.borderFocus : Theme.borderSubtle
                    border.width: 1

                    Text {
                        id: regexLabel
                        anchors.centerIn: parent
                        text: ".*"
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: stream.regexMode ? Theme.accent : Theme.textSecondary
                    }

                    MouseArea {
                        id: regexMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: stream.regexMode = !stream.regexMode
                    }
                }

                Rectangle {
                    implicitWidth: levelRepeaterRow.implicitWidth
                    implicitHeight: 22
                    color: "transparent"

                    Row {
                        id: levelRepeaterRow
                        anchors.centerIn: parent
                        spacing: Theme.sp1

                        Rectangle {
                            implicitWidth: debugText.implicitWidth + Theme.sp2 * 2
                            implicitHeight: 22
                            radius: Theme.radiusSm
                            color: debugMouse.containsMouse
                                   ? Theme.bgHover
                                   : (stream.debugEnabled ? _badgeFill(PierLogStream.DebugLevel) : "transparent")
                            border.color: stream.debugEnabled ? Theme.borderDefault : Theme.borderSubtle
                            border.width: 1

                            Text {
                                id: debugText
                                anchors.centerIn: parent
                                text: "DBG"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: stream.debugEnabled ? _badgeTextColor(PierLogStream.DebugLevel) : Theme.textTertiary
                            }

                            MouseArea {
                                id: debugMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: stream.debugEnabled = !stream.debugEnabled
                            }
                        }

                        Rectangle {
                            implicitWidth: infoText.implicitWidth + Theme.sp2 * 2
                            implicitHeight: 22
                            radius: Theme.radiusSm
                            color: infoMouse.containsMouse
                                   ? Theme.bgHover
                                   : (stream.infoEnabled ? _badgeFill(PierLogStream.InfoLevel) : "transparent")
                            border.color: stream.infoEnabled ? Theme.borderDefault : Theme.borderSubtle
                            border.width: 1

                            Text {
                                id: infoText
                                anchors.centerIn: parent
                                text: "INF"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: stream.infoEnabled ? _badgeTextColor(PierLogStream.InfoLevel) : Theme.textTertiary
                            }

                            MouseArea {
                                id: infoMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: stream.infoEnabled = !stream.infoEnabled
                            }
                        }

                        Rectangle {
                            implicitWidth: warnText.implicitWidth + Theme.sp2 * 2
                            implicitHeight: 22
                            radius: Theme.radiusSm
                            color: warnMouse.containsMouse
                                   ? Theme.bgHover
                                   : (stream.warnEnabled ? _badgeFill(PierLogStream.WarnLevel) : "transparent")
                            border.color: stream.warnEnabled ? Theme.borderDefault : Theme.borderSubtle
                            border.width: 1

                            Text {
                                id: warnText
                                anchors.centerIn: parent
                                text: "WRN"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: stream.warnEnabled ? _badgeTextColor(PierLogStream.WarnLevel) : Theme.textTertiary
                            }

                            MouseArea {
                                id: warnMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: stream.warnEnabled = !stream.warnEnabled
                            }
                        }

                        Rectangle {
                            implicitWidth: errorText.implicitWidth + Theme.sp2 * 2
                            implicitHeight: 22
                            radius: Theme.radiusSm
                            color: errorMouse.containsMouse
                                   ? Theme.bgHover
                                   : (stream.errorEnabled ? _badgeFill(PierLogStream.ErrorLevel) : "transparent")
                            border.color: stream.errorEnabled ? Theme.borderDefault : Theme.borderSubtle
                            border.width: 1

                            Text {
                                id: errorText
                                anchors.centerIn: parent
                                text: "ERR"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: stream.errorEnabled ? _badgeTextColor(PierLogStream.ErrorLevel) : Theme.textTertiary
                            }

                            MouseArea {
                                id: errorMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: stream.errorEnabled = !stream.errorEnabled
                            }
                        }

                        Rectangle {
                            implicitWidth: fatalText.implicitWidth + Theme.sp2 * 2
                            implicitHeight: 22
                            radius: Theme.radiusSm
                            color: fatalMouse.containsMouse
                                   ? Theme.bgHover
                                   : (stream.fatalEnabled ? _badgeFill(PierLogStream.FatalLevel) : "transparent")
                            border.color: stream.fatalEnabled ? Theme.borderDefault : Theme.borderSubtle
                            border.width: 1

                            Text {
                                id: fatalText
                                anchors.centerIn: parent
                                text: "FTL"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: stream.fatalEnabled ? _badgeTextColor(PierLogStream.FatalLevel) : Theme.textTertiary
                            }

                            MouseArea {
                                id: fatalMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: stream.fatalEnabled = !stream.fatalEnabled
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: stream.regexError.length > 0 ? 22 : 0
            visible: stream.regexError.length > 0
            color: Qt.rgba(Theme.statusWarning.r, Theme.statusWarning.g, Theme.statusWarning.b, 0.10)
            border.color: Theme.statusWarning
            border.width: 1
            radius: Theme.radiusSm

            Text {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                verticalAlignment: Text.AlignVCenter
                text: stream.regexError
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.statusWarning
                elide: Text.ElideRight
            }
        }

        // ─── Line list ───────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.bgPanel
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            ListView {
                id: listView
                anchors.fill: parent
                anchors.margins: Theme.sp1
                clip: true
                model: stream
                spacing: 0
                reuseItems: true

                // Follow the tail as rows arrive, but only
                // when the user hasn't scrolled away — this is
                // the same affordance every tail -f viewer has.
                // We inspect `atYEnd` on each count change:
                // - autoScroll ON + atYEnd or count just grew:
                //   positionViewAtEnd().
                onCountChanged: {
                    if (root.autoScroll) {
                        Qt.callLater(listView.positionViewAtEnd)
                    }
                }

                delegate: Rectangle {
                    id: row
                    required property int    index
                    required property int    kind
                    required property int    level
                    required property string text

                    width: ListView.view.width
                    implicitHeight: 24
                    color: index % 2 === 0 ? "transparent" : Theme.bgHover
                    radius: Theme.radiusSm

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        spacing: Theme.sp2

                        Rectangle {
                            visible: row.level !== PierLogStream.UnknownLevel
                            implicitWidth: badgeText.implicitWidth + Theme.sp1_5 * 2
                            implicitHeight: 16
                            radius: Theme.radiusXs
                            color: _badgeFill(row.level)
                            border.color: "transparent"
                            Layout.alignment: Qt.AlignVCenter

                            Text {
                                id: badgeText
                                anchors.centerIn: parent
                                text: _badgeText(row.level)
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                font.weight: Theme.weightMedium
                                color: _badgeTextColor(row.level)
                            }
                        }

                        Text {
                            id: rowText
                            Layout.fillWidth: true
                            text: row.text
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            font.weight: row.kind === PierLogStream.Stderr
                                         || row.kind === PierLogStream.ErrorKind
                                         || row.level === PierLogStream.ErrorLevel
                                         || row.level === PierLogStream.FatalLevel
                                         ? Theme.weightMedium
                                         : Theme.weightRegular
                            color: _colorForLevel(row.level, row.kind, row.text)
                            wrapMode: Text.NoWrap
                            elide: Text.ElideRight
                            verticalAlignment: Text.AlignVCenter

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }
                    }
                }

                // Busy placeholder while waiting for first
                // row to arrive.
                Text {
                    anchors.centerIn: parent
                    visible: listView.count === 0
                    text: stream.status === PierLogStream.Connected
                          && stream.totalLineCount === 0
                          ? qsTr("Waiting for output…")
                          : (stream.totalLineCount > 0
                             ? qsTr("No lines match the current filters")
                             : qsTr("Waiting for output…"))
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textTertiary
                }
            }
        }

        // ─── Footer: count + exit code ──────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 20
            color: "transparent"

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                text: stream.lineCount === stream.totalLineCount
                      ? (stream.totalLineCount + " "
                         + (stream.totalLineCount === 1 ? qsTr("line") : qsTr("lines")))
                      : qsTr("%1 of %2 lines")
                            .arg(stream.lineCount)
                            .arg(stream.totalLineCount)
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }
    }

    // ─── Connecting / Failed overlay ───────────────────
    Rectangle {
        id: overlay

        anchors.fill: parent
        visible: stream.status === PierLogStream.Connecting
              || stream.status === PierLogStream.Failed

        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.88)

        Behavior on opacity { NumberAnimation { duration: Theme.durNormal } }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            preventStealing: true
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        Rectangle {
            id: card
            anchors.centerIn: parent
            width: Math.min(420, parent.width - Theme.sp8 * 2)
            implicitHeight: cardColumn.implicitHeight + Theme.sp5 * 2

            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                id: cardColumn
                anchors.fill: parent
                anchors.margins: Theme.sp5
                spacing: Theme.sp3

                SectionLabel {
                    text: stream.status === PierLogStream.Connecting
                          ? qsTr("Opening log stream")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: stream.target.length > 0
                          ? stream.target
                          : qsTr("Log")
                    Layout.alignment: Qt.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeH3
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: card.width - Theme.sp5 * 2

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Text {
                    visible: stream.command.length > 0
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: "$ " + stream.command
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textSecondary
                    elide: Text.ElideMiddle
                }

                Text {
                    visible: stream.status === PierLogStream.Failed
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp2
                    text: stream.errorMessage.length > 0
                          ? stream.errorMessage
                          : qsTr("Unknown error")
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.statusError
                }

                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp3
                    spacing: Theme.sp2

                    Item { Layout.fillWidth: true }

                    GhostButton {
                        text: qsTr("Cancel")
                        onClicked: stream.stop()
                    }
                    PrimaryButton {
                        text: qsTr("Retry")
                        visible: stream.status === PierLogStream.Failed
                        onClicked: _dispatchConnect()
                    }
                }
            }
        }
    }
}
