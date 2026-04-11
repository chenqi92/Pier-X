import QtQuick
import QtQuick.Layouts
import Pier

// SFTP file browser — right-side alternative to TerminalView.
//
// Layout:
//   ┌───────────────────────────────────────────────────┐
//   │ [⟵] /var/log                          [↻]        │  path bar
//   ├───────────────────────────────────────────────────┤
//   │ 📁 apache2                                        │  listing
//   │ 📁 nginx                                          │
//   │ 📄 syslog                  2.3 MB                 │
//   │ ...                                               │
//   └───────────────────────────────────────────────────┘
//
// Keyboard / mouse: double-click a directory to enter it,
// up arrow button or the back key to go up. Files don't do
// anything yet — upload / download / edit land as follow-up
// polish.
Rectangle {
    id: root

    // Backend fields — identical shape to TerminalView so
    // Main.qml's Repeater delegate can bind uniformly.
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

    PierSftpBrowser {
        id: browser

        onOperationFinished: (ok, message) => {
            if (!ok) console.warn("sftp op:", message)
        }
    }

    // Kick off the connection on first mount.
    Component.onCompleted: _dispatchConnect()

    function _dispatchConnect() {
        if (root.sshHost.length === 0 || root.sshUser.length === 0) {
            console.warn("SftpBrowserView: missing host/user")
            return
        }
        // Translate the same auth field matrix TerminalView
        // uses into the SFTP authKind + secret + extra triple.
        var kind = 0
        var secret = ""
        var extra = ""
        if (root.sshUsesAgent) {
            kind = 3  // PIER_AUTH_AGENT
        } else if (root.sshKeyPath.length > 0) {
            kind = 2  // PIER_AUTH_KEY
            secret = root.sshKeyPath
            extra = root.sshPassphraseCredentialId
        } else if (root.sshCredentialId.length > 0) {
            kind = 1  // PIER_AUTH_CREDENTIAL
            secret = root.sshCredentialId
        } else {
            kind = 0  // PIER_AUTH_PASSWORD
            secret = root.sshPassword
        }
        browser.connectTo(root.sshHost, root.sshPort, root.sshUser,
                          kind, secret, extra)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // ─── Path bar ────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            IconButton {
                glyph: "←"
                tooltip: qsTr("Go up")
                onClicked: browser.navigateUp()
                enabled: browser.currentPath !== "/" && browser.currentPath.length > 0
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 28
                color: Theme.bgSurface
                border.color: Theme.borderDefault
                border.width: 1
                radius: Theme.radiusSm

                Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp3
                    text: browser.currentPath.length > 0 ? browser.currentPath : qsTr("—")
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            IconButton {
                glyph: "↻"
                tooltip: qsTr("Refresh")
                onClicked: browser.refresh()
                enabled: browser.status === PierSftpBrowser.Connected
            }
        }

        // ─── Listing ─────────────────────────────────────
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
                model: browser

                delegate: Rectangle {
                    id: entry
                    required property int index
                    required property string name
                    required property string path
                    required property bool isDir
                    required property bool isLink
                    required property var size

                    width: ListView.view.width
                    implicitHeight: 26
                    color: mouseArea.containsMouse ? Theme.bgHover : "transparent"
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        spacing: Theme.sp2

                        // Icon glyph (monochrome Unicode for now;
                        // real SVG icons land with M6 polish).
                        Text {
                            text: entry.isDir
                                  ? "▸"
                                  : (entry.isLink ? "↪" : "·")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: entry.isDir ? Theme.accent : Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }
                        }

                        Text {
                            Layout.fillWidth: true
                            text: entry.name
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: entry.isDir
                                         ? Theme.weightMedium
                                         : Theme.weightRegular
                            color: Theme.textPrimary
                            elide: Text.ElideRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        Text {
                            visible: !entry.isDir && entry.size > 0
                            text: formatSize(entry.size)
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.textTertiary
                            Layout.alignment: Qt.AlignRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }
                    }

                    MouseArea {
                        id: mouseArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: entry.isDir
                                     ? Qt.PointingHandCursor
                                     : Qt.ArrowCursor
                        onDoubleClicked: {
                            if (entry.isDir) {
                                browser.listDir(entry.path)
                            }
                        }
                    }
                }
            }

            // Empty state — shown when connected but the
            // current directory has no visible entries.
            Text {
                anchors.centerIn: parent
                visible: browser.status === PierSftpBrowser.Connected
                         && !browser.busy
                         && listView.count === 0
                text: qsTr("(empty directory)")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            // Busy spinner during list_dir requests.
            Text {
                anchors.centerIn: parent
                visible: browser.busy && listView.count === 0
                text: qsTr("Loading…")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textSecondary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }
    }

    // ─── Connecting / Failed overlay ───────────────────
    // Reuses the same shape as TerminalView's overlay: a
    // dimming scrim, a centered card, SectionLabel + target +
    // status text + Cancel / Retry buttons.
    Rectangle {
        id: overlay

        anchors.fill: parent
        visible: browser.status === PierSftpBrowser.Connecting
              || browser.status === PierSftpBrowser.Failed

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
                    text: browser.status === PierSftpBrowser.Connecting
                          ? qsTr("Opening SFTP")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: browser.target.length > 0 ? browser.target : qsTr("SFTP")
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
                    visible: browser.status === PierSftpBrowser.Failed
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp2
                    text: browser.errorMessage.length > 0
                          ? browser.errorMessage
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
                        onClicked: browser.stop()
                    }
                    PrimaryButton {
                        text: qsTr("Retry")
                        visible: browser.status === PierSftpBrowser.Failed
                        onClicked: _dispatchConnect()
                    }
                }
            }
        }
    }

    // ─── Helpers ─────────────────────────────────────────
    function formatSize(bytes) {
        if (bytes < 1024) return bytes + " B"
        if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB"
        if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + " MB"
        return (bytes / (1024 * 1024 * 1024)).toFixed(2) + " GB"
    }
}
