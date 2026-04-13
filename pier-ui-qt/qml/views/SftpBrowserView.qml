import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

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

    clip: true
    property var sharedSession: null

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
    Component.onCompleted: Qt.callLater(_dispatchConnect)

    Connections {
        target: root.sharedSession
        function onConnectedChanged() {
            if (root.sharedSession && root.sharedSession.connected)
                root._dispatchConnect()
        }
    }

    function _dispatchConnect() {
        if (browser.status === PierSftpBrowser.Connecting
                || browser.status === PierSftpBrowser.Connected)
            return
        if (root.sharedSession && root.sharedSession.connected) {
            browser.connectToSession(root.sharedSession)
            return
        }
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
        anchors.margins: Theme.sp2
        spacing: Theme.sp2

        ToolHeroPanel {
            Layout.fillWidth: true
            compact: true
            accentColor: Theme.accent

            ColumnLayout {
                id: sftpHeader
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    compact: true
                    prominent: true
                    icon: "folder-sync"
                    title: qsTr("SFTP")
                    subtitle: browser.target.length > 0 ? browser.target : qsTr("Remote Files")

                    IconButton {
                        compact: true
                        icon: "arrow-left"
                        tooltip: qsTr("Go up")
                        onClicked: browser.navigateUp()
                        enabled: browser.currentPath !== "/" && browser.currentPath.length > 0
                    }

                    IconButton {
                        compact: true
                        icon: "refresh-cw"
                        tooltip: qsTr("Refresh")
                        onClicked: browser.refresh()
                        enabled: browser.status === PierSftpBrowser.Connected
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    StatusPill {
                        text: browser.status === PierSftpBrowser.Connected
                              ? qsTr("Connected")
                              : (browser.status === PierSftpBrowser.Connecting
                                 ? qsTr("Connecting")
                                 : qsTr("Idle"))
                        tone: browser.status === PierSftpBrowser.Connected ? "info" : "neutral"
                    }

                    StatusPill {
                        visible: listView.count > 0
                        text: qsTr("%1 entries").arg(listView.count)
                        tone: "neutral"
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    ToolFactChip {
                        label: qsTr("Path")
                        value: browser.currentPath
                        monoValue: true
                    }

                    ToolFactChip {
                        label: qsTr("Entries")
                        value: listView.count > 0 ? String(listView.count) : ""
                        monoValue: true
                    }
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    inset: true
                    padding: Theme.sp0
                    implicitHeight: 30

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
            }
        }

        // ─── Listing ─────────────────────────────────────
        ToolPanelSurface {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp2
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    compact: true
                    icon: "folder"
                    title: qsTr("Directory")
                    subtitle: browser.currentPath.length > 0 ? browser.currentPath : qsTr("Waiting for directory listing.")
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 24
                    radius: Theme.radiusSm
                    color: Theme.bgInset
                    border.color: Theme.borderSubtle
                    border.width: 1

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        spacing: Theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Name")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            font.weight: Theme.weightMedium
                            color: Theme.textTertiary
                            elide: Text.ElideRight
                        }

                        Text {
                            Layout.preferredWidth: 84
                            text: qsTr("Modified")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            font.weight: Theme.weightMedium
                            color: Theme.textTertiary
                            horizontalAlignment: Text.AlignRight
                        }

                        Text {
                            Layout.preferredWidth: 82
                            text: qsTr("Size")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            font.weight: Theme.weightMedium
                            color: Theme.textTertiary
                            horizontalAlignment: Text.AlignRight
                        }
                    }
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    inset: true
                    padding: Theme.sp1_5

                    ListView {
                        id: listView
                        anchors.fill: parent
                        clip: true
                        boundsBehavior: Flickable.StopAtBounds
                        model: browser

                        delegate: Rectangle {
                            id: entry
                            required property int index
                            required property string name
                            required property string path
                            required property bool isDir
                            required property bool isLink
                            required property var size
                            required property var modified

                            width: ListView.view.width
                            implicitHeight: 28
                            color: mouseArea.containsMouse ? Theme.bgHover : "transparent"
                            radius: Theme.radiusSm

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                spacing: Theme.sp2

                                Image {
                                    source: entry.isDir
                                            ? "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                                            : (entry.isLink
                                               ? "qrc:/qt/qml/Pier/resources/icons/lucide/link.svg"
                                               : "qrc:/qt/qml/Pier/resources/icons/lucide/file-text.svg")
                                    sourceSize: Qt.size(14, 14)
                                    Layout.alignment: Qt.AlignVCenter
                                    layer.enabled: true
                                    layer.effect: MultiEffect {
                                        colorization: 1.0
                                        colorizationColor: entry.isDir ? Theme.accent : Theme.textTertiary
                                    }
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
                                    Layout.preferredWidth: 84
                                    text: entry.modified > 0 ? formatModified(entry.modified) : "—"
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textTertiary
                                    elide: Text.ElideRight
                                    horizontalAlignment: Text.AlignRight
                                }

                                Text {
                                    Layout.preferredWidth: 82
                                    text: entry.isDir ? "—" : formatSize(entry.size)
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeCaption
                                    color: Theme.textTertiary
                                    Layout.alignment: Qt.AlignRight
                                    horizontalAlignment: Text.AlignRight

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
                }
            }

            // Empty state — shown when connected but the
            // current directory has no visible entries.
            ToolEmptyState {
                anchors.centerIn: parent
                visible: browser.status === PierSftpBrowser.Connected
                         && !browser.busy
                         && listView.count === 0
                compact: true
                icon: "folder"
                title: qsTr("Directory ready")
                description: qsTr("Entries for this path will appear here.")
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

        ToolBanner {
            Layout.fillWidth: true
            tone: "neutral"
            text: qsTr("%1 entries").arg(listView.count)
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

    function formatModified(seconds) {
        if (!seconds || seconds <= 0)
            return "—"
        var d = new Date(seconds * 1000)
        var yy = String(d.getFullYear())
        var mm = String(d.getMonth() + 1).padStart(2, "0")
        var dd = String(d.getDate()).padStart(2, "0")
        return yy + "-" + mm + "-" + dd
    }
}
