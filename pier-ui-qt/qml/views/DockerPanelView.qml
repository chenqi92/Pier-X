import QtQuick
import QtQuick.Layouts
import Pier

// Docker container panel — M5c per-service tool.
//
// Layout
// ──────
//   ┌───────────────────────────────────────────────────┐
//   │ user@host     [show stopped ✓]  [↻ Refresh]       │  top bar
//   ├───────────────────────────────────────────────────┤
//   │ ● web         nginx:stable      Up 5m   [▶][⏸][↻][📜][✕]
//   │ ● cache       redis:7-alpine    Up 5m   ...
//   │ ○ db          postgres:16       Exited  ...
//   │ ...                                               │
//   └───────────────────────────────────────────────────┘
//
// Click a row's "logs" button to open a new Log viewer tab
// that runs `docker logs -f --tail 500 <id>`.
//
// Backend params mirror TerminalView / SftpBrowserView so
// the Main.qml Loader delegate can dispatch uniformly.
Rectangle {
    id: root

    property string sshHost: ""
    property int    sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool   sshUsesAgent: false

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierDockerClient {
        id: client
        onActionFinished: (ok, message) => {
            if (!ok) console.warn("docker action:", message)
        }
    }

    // The delete-confirm row state lives up here so it
    // survives the ListView delegate's reuseItems recycle.
    property string pendingDeleteId: ""

    Component.onCompleted: _dispatchConnect()

    function _dispatchConnect() {
        if (root.sshHost.length === 0 || root.sshUser.length === 0) {
            console.warn("DockerPanelView: missing host/user")
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
        client.connectTo(root.sshHost, root.sshPort, root.sshUser,
                         kind, secret, extra)
    }

    // Open a new Log viewer tab tailing this container. Goes
    // through Main.qml because tab creation isn't a per-view
    // responsibility.
    function _openLogsFor(id, name) {
        var conn = {
            name: name || id,
            host: root.sshHost,
            port: root.sshPort,
            username: root.sshUser,
            password: root.sshPassword,
            credentialId: root.sshCredentialId,
            keyPath: root.sshKeyPath,
            passphraseCredentialId: root.sshPassphraseCredentialId,
            usesAgent: root.sshUsesAgent
        }
        // docker logs -f --tail 500 <id>: 500 lines of history
        // so the user doesn't open into a blank viewer, then
        // live tail from there.
        var cmd = "docker logs -f --tail 500 " + id
        window.openLogTab(conn, cmd, name || id)
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
                text: client.target.length > 0
                      ? client.target
                      : qsTr("Docker")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 160
                Layout.maximumWidth: 280

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            Item { Layout.fillWidth: true }

            // Show-stopped toggle. Clicking refreshes with the
            // new flag (setShowStopped triggers refresh on the
            // C++ side).
            Rectangle {
                id: stoppedToggle
                implicitWidth: stoppedLabel.implicitWidth + Theme.sp3 * 2
                implicitHeight: 24
                radius: Theme.radiusPill
                color: stoppedMouse.containsMouse
                       ? Theme.bgHover
                       : (client.showStopped
                          ? Theme.accentSubtle
                          : Theme.bgSurface)
                border.color: client.showStopped
                              ? Theme.borderFocus
                              : Theme.borderSubtle
                border.width: 1

                Behavior on color        { ColorAnimation { duration: Theme.durFast } }
                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                Text {
                    id: stoppedLabel
                    anchors.centerIn: parent
                    text: client.showStopped
                          ? qsTr("Show stopped ✓")
                          : qsTr("Show stopped")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: client.showStopped ? Theme.accent : Theme.textSecondary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                MouseArea {
                    id: stoppedMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: client.showStopped = !client.showStopped
                }
            }

            GhostButton {
                text: qsTr("↻ Refresh")
                enabled: client.status === PierDockerClient.Connected
                onClicked: client.refresh()
            }
        }

        // ─── Container list ─────────────────────────────
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
                model: client
                spacing: 0
                reuseItems: true

                delegate: Rectangle {
                    id: row
                    required property int    index
                    required property string containerId
                    required property string image
                    required property string names
                    required property string statusText
                    required property string state
                    required property bool   isRunning
                    required property string ports

                    readonly property bool confirming:
                        root.pendingDeleteId === row.containerId

                    width: ListView.view.width
                    implicitHeight: 32
                    color: rowMouse.containsMouse
                           ? Theme.bgHover
                           : "transparent"
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    // ── Idle layout (name + image + status + actions) ──
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        spacing: Theme.sp2
                        visible: !row.confirming

                        // State dot.
                        Rectangle {
                            width: 8
                            height: 8
                            radius: 4
                            color: row.isRunning
                                   ? Theme.statusSuccess
                                   : Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                            SequentialAnimation on opacity {
                                running: row.isRunning
                                loops: Animation.Infinite
                                NumberAnimation { from: 1.0; to: 0.5; duration: 1200 }
                                NumberAnimation { from: 0.5; to: 1.0; duration: 1200 }
                            }
                        }

                        Text {
                            Layout.preferredWidth: 160
                            text: row.names.length > 0
                                  ? row.names
                                  : row.containerId.slice(0, 12)
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeBody
                            font.weight: row.isRunning
                                         ? Theme.weightMedium
                                         : Theme.weightRegular
                            color: Theme.textPrimary
                            elide: Text.ElideRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        Text {
                            Layout.fillWidth: true
                            text: row.image
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.textSecondary
                            elide: Text.ElideRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        Text {
                            Layout.preferredWidth: 180
                            text: row.statusText
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            color: row.isRunning
                                   ? Theme.statusSuccess
                                   : Theme.textTertiary
                            horizontalAlignment: Text.AlignRight
                            elide: Text.ElideRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        // ── Row action buttons ──
                        // Start (only when stopped).
                        DockerRowButton {
                            glyph: "▶"
                            tooltip: qsTr("Start")
                            visible: !row.isRunning
                            onClicked: client.start(row.containerId)
                        }
                        // Stop (only when running).
                        DockerRowButton {
                            glyph: "⏸"
                            tooltip: qsTr("Stop")
                            visible: row.isRunning
                            onClicked: client.stopContainer(row.containerId)
                        }
                        DockerRowButton {
                            glyph: "↻"
                            tooltip: qsTr("Restart")
                            visible: row.isRunning
                            onClicked: client.restart(row.containerId)
                        }
                        DockerRowButton {
                            glyph: "📜"
                            tooltip: qsTr("Live logs")
                            onClicked: root._openLogsFor(row.containerId, row.names)
                        }
                        DockerRowButton {
                            glyph: "✕"
                            tooltip: qsTr("Remove")
                            danger: true
                            onClicked: root.pendingDeleteId = row.containerId
                        }
                    }

                    // ── Confirm layout ──────────────────
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        spacing: Theme.sp2
                        visible: row.confirming

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Remove '%1'?")
                                      .arg(row.names.length > 0
                                           ? row.names
                                           : row.containerId.slice(0, 12))
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: Theme.statusError
                            elide: Text.ElideRight
                        }
                        GhostButton {
                            text: qsTr("Cancel")
                            onClicked: root.pendingDeleteId = ""
                        }
                        // Force-remove for running containers —
                        // a plain "rm" would fail otherwise.
                        PrimaryButton {
                            text: row.isRunning
                                  ? qsTr("Force remove")
                                  : qsTr("Remove")
                            onClicked: {
                                client.remove(row.containerId, row.isRunning)
                                root.pendingDeleteId = ""
                            }
                        }
                    }
                }

                // Busy placeholder for the first listing.
                Text {
                    anchors.centerIn: parent
                    visible: client.busy && listView.count === 0
                    text: qsTr("Querying docker…")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textSecondary
                }
                // Empty state — connected, not busy, no containers.
                Text {
                    anchors.centerIn: parent
                    visible: client.status === PierDockerClient.Connected
                             && !client.busy
                             && listView.count === 0
                    text: client.showStopped
                          ? qsTr("(no containers)")
                          : qsTr("(no running containers)")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textTertiary
                }
            }
        }

        // ─── Footer ──────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 20
            color: "transparent"

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                text: client.containerCount + " "
                      + (client.containerCount === 1 ? qsTr("container") : qsTr("containers"))
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
        visible: client.status === PierDockerClient.Connecting
              || client.status === PierDockerClient.Failed

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
                    text: client.status === PierDockerClient.Connecting
                          ? qsTr("Connecting to Docker")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: client.target.length > 0 ? client.target : qsTr("Docker")
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
                    visible: client.status === PierDockerClient.Failed
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp2
                    text: client.errorMessage.length > 0
                          ? client.errorMessage
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
                        onClicked: client.stop()
                    }
                    PrimaryButton {
                        text: qsTr("Retry")
                        visible: client.status === PierDockerClient.Failed
                        onClicked: _dispatchConnect()
                    }
                }
            }
        }
    }

    // ─── Local row-button component ──────────────────────
    // Small pill with a single glyph + hover tooltip.
    // Kept in-file because it's only used by this view and
    // lives under the row layout's hit target.
    component DockerRowButton : Rectangle {
        id: rowBtn
        required property string glyph
        required property string tooltip
        property bool danger: false
        signal clicked()

        implicitWidth: 22
        implicitHeight: 22
        radius: Theme.radiusSm
        color: btnMouse.containsMouse
               ? (rowBtn.danger ? Theme.statusError : Theme.accentSubtle)
               : "transparent"
        border.color: btnMouse.containsMouse
                      ? (rowBtn.danger ? Theme.statusError : Theme.accent)
                      : Theme.borderSubtle
        border.width: 1

        Behavior on color        { ColorAnimation { duration: Theme.durFast } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

        Text {
            anchors.centerIn: parent
            text: rowBtn.glyph
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: btnMouse.containsMouse
                   ? (rowBtn.danger ? Theme.bgCanvas : Theme.accent)
                   : Theme.textSecondary

            Behavior on color { ColorAnimation { duration: Theme.durFast } }
        }

        MouseArea {
            id: btnMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: rowBtn.clicked()
        }

        PierToolTip {
            visible: btnMouse.containsMouse
            text: rowBtn.tooltip
        }
    }
}
