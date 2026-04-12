import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../views"
import "../components"

Rectangle {
    id: root

    property string activeTool: "git"
    property string activeBackend: ""
    property string gitContextPath: ""

    // Shared SSH session from the active terminal tab.
    // Tools use connectToSession(sharedSession) instead of
    // opening their own independent SSH connections.
    property var sharedSession: null

    property string sshHost: ""
    property int sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool sshUsesAgent: false

    property string redisHost: ""
    property int redisPort: 0
    property int redisDb: 0
    property string logCommand: ""
    property string mysqlHost: ""
    property int mysqlPort: 3306
    property string mysqlUser: ""
    property string mysqlPassword: ""
    property string mysqlDatabase: ""
    property string pgHost: ""
    property int pgPort: 5432
    property string pgUser: ""
    property string pgDatabase: ""

    // Content area expanded/collapsed. The tool strip is always
    // visible; only the content pane hides on collapse.
    property bool contentExpanded: true
    clip: true

    // Effective session = tab's shared SSH session (null when local)
    readonly property var effectiveSession: (root.sharedSession && root.sharedSession.connected)
                                           ? root.sharedSession : null

    signal closePanelRequested()

    readonly property bool hasRemoteContext: root.activeBackend === "ssh"
                                            || root.sshHost.length > 0
                                            || root.effectiveSession !== null
    readonly property bool currentToolReady: root.toolAvailable(root.activeTool)
    readonly property string panelTitle: {
        switch (root.activeTool) {
        case "git": return qsTr("Git")
        case "monitor": return qsTr("Server Monitor")
        case "docker": return qsTr("Docker")
        case "mysql": return qsTr("MySQL")
        case "redis": return qsTr("Redis")
        case "log": return qsTr("Logs")
        case "sftp": return qsTr("SFTP")
        case "sqlite": return qsTr("SQLite")
        default: return qsTr("Tool Panel")
        }
    }
    readonly property string panelSubtitle: {
        if (!root.currentToolReady)
            return qsTr("Connect to a server or open a supported context to unlock this panel.")
        if (root.activeTool === "git")
            return root.gitContextPath.length > 0 ? root.gitContextPath : PierCore.workingDirectory
        if (root.activeTool === "mysql" && root.mysqlHost.length > 0)
            return root.mysqlUser.length > 0
                    ? root.mysqlUser + "@" + root.mysqlHost + ":" + root.mysqlPort
                    : root.mysqlHost + ":" + root.mysqlPort
        if (root.activeTool === "redis" && root.redisHost.length > 0)
            return root.redisHost + ":" + root.redisPort
        if (root.effectiveSession && root.effectiveSession.connected)
            return root.effectiveSession.target
        if (root.sshHost.length > 0)
            return root.sshUser.length > 0 ? root.sshUser + "@" + root.sshHost + ":" + root.sshPort : root.sshHost
        return "localhost"
    }

    function toolAvailable(tool) {
        // Git is always available (local repo)
        if (tool === "git") return true
        // These tools work both locally and remotely
        if (tool === "monitor") return true
        if (tool === "docker") return true
        if (tool === "mysql") return true
        if (tool === "redis") return true
        if (tool === "log") return true
        if (tool === "sqlite") return true
        // SFTP only makes sense for remote connections
        if (tool === "sftp") return root.hasRemoteContext
        return root.hasRemoteContext
    }

    color: Theme.bgPanel

    RowLayout {
        anchors.fill: parent
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp2
            visible: root.contentExpanded

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 48
                color: Theme.bgPanel
                border.width: 0

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp2
                    spacing: Theme.sp2

                    Rectangle {
                        width: 8
                        height: 8
                        radius: 4
                        color: root.currentToolReady
                               ? (root.activeTool === "git" ? Theme.accent : Theme.statusSuccess)
                               : Theme.textDisabled
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            text: root.panelTitle
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBodyLg
                            font.weight: Theme.weightSemibold
                            color: Theme.textPrimary
                            elide: Text.ElideRight
                        }

                        Text {
                            text: root.panelSubtitle
                            font.family: root.activeTool === "git" ? Theme.fontMono : Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                            elide: Text.ElideMiddle
                            Layout.fillWidth: true
                        }
                    }

                    IconButton {
                        compact: true
                        icon: "link"
                        tooltip: root.effectiveSession
                                 ? qsTr("Disconnect from %1").arg(root.effectiveSession.target)
                                 : qsTr("No remote connection")
                        visible: root.effectiveSession !== null
                        onClicked: root.effectiveSession.close()
                    }

                    IconButton {
                        compact: true
                        icon: "x"
                        tooltip: qsTr("Collapse panel")
                        onClicked: root.contentExpanded = false
                    }
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.borderSubtle
                }
            }

            // ── Mode indicator strip ──────────────────────
            // Shows whether tools are operating locally or via SSH.
            // No manual connect form — tools follow the terminal.
            Rectangle {
                Layout.fillWidth: true
                visible: root.activeTool !== "git" && root.activeTool !== "sqlite"
                implicitHeight: 28
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2

                    Rectangle {
                        width: 6; height: 6; radius: 3
                        color: root.effectiveSession ? Theme.statusSuccess : Theme.accent
                    }

                    Text {
                        text: root.effectiveSession
                              ? root.effectiveSession.target
                              : "localhost"
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textSecondary
                        elide: Text.ElideMiddle
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.effectiveSession ? qsTr("SSH") : qsTr("Local")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        font.weight: Theme.weightMedium
                        color: root.effectiveSession ? Theme.statusSuccess : Theme.accent
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.bgPanel
                border.width: 0
                radius: 0

                StackLayout {
                    anchors.fill: parent
                    currentIndex: {
                        switch (root.activeTool) {
                        case "git":     return 0
                        case "monitor": return 2
                        case "docker":  return 3
                        case "mysql":   return 4
                        case "redis":   return 5
                        case "log":     return 6
                        case "sftp":    return root.hasRemoteContext ? 7 : 1
                        case "sqlite":  return 8
                        default:        return 0
                        }
                    }

                    GitPanelView {
                        repoPath: root.gitContextPath.length > 0 ? root.gitContextPath : PierCore.workingDirectory
                    }

                    Item {
                        Rectangle {
                            anchors.centerIn: parent
                            width: Math.min(parent.width - Theme.sp8, 300)
                            height: 156
                            radius: Theme.radiusMd
                            color: Theme.bgInset
                            border.color: Theme.borderSubtle
                            border.width: 1

                            ColumnLayout {
                                anchors.centerIn: parent
                                width: parent.width - Theme.sp6 * 2
                                spacing: Theme.sp2

                                Text {
                                    Layout.fillWidth: true
                                    text: root.panelTitle
                                    horizontalAlignment: Text.AlignHCenter
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeH3
                                    font.weight: Theme.weightSemibold
                                    color: Theme.textPrimary
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: qsTr("This panel becomes available once an SSH session or supported service context is active.")
                                    horizontalAlignment: Text.AlignHCenter
                                    wrapMode: Text.WordWrap
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textTertiary
                                }
                            }
                        }
                    }

                    Loader {
                        active: root.activeTool === "monitor"
                        sourceComponent: ServerMonitorView {
                            sharedSession: root.effectiveSession
                            sshHost: root.sshHost
                            sshPort: root.sshPort
                            sshUser: root.sshUser
                            sshPassword: root.sshPassword
                            sshCredentialId: root.sshCredentialId
                            sshKeyPath: root.sshKeyPath
                            sshPassphraseCredentialId: root.sshPassphraseCredentialId
                            sshUsesAgent: root.sshUsesAgent
                        }
                    }

                    Loader {
                        active: root.activeTool === "docker"
                        sourceComponent: DockerPanelView {
                            sharedSession: root.effectiveSession
                            sshHost: root.sshHost
                            sshPort: root.sshPort
                            sshUser: root.sshUser
                            sshPassword: root.sshPassword
                            sshCredentialId: root.sshCredentialId
                            sshKeyPath: root.sshKeyPath
                            sshPassphraseCredentialId: root.sshPassphraseCredentialId
                            sshUsesAgent: root.sshUsesAgent
                        }
                    }

                    Loader {
                        active: root.activeTool === "mysql"
                        sourceComponent: MySqlBrowserView {
                            mysqlHost: root.mysqlHost
                            mysqlPort: root.mysqlPort
                            mysqlUser: root.mysqlUser
                            mysqlPassword: root.mysqlPassword
                            mysqlDatabase: root.mysqlDatabase
                        }
                    }

                    Loader {
                        active: root.activeTool === "redis"
                        sourceComponent: RedisBrowserView {
                            redisHost: root.redisHost
                            redisPort: root.redisPort
                            redisDb: root.redisDb
                        }
                    }

                    Loader {
                        active: root.activeTool === "log"
                        sourceComponent: LogViewerView {
                            sharedSession: root.effectiveSession
                            sshHost: root.sshHost
                            sshPort: root.sshPort
                            sshUser: root.sshUser
                            sshPassword: root.sshPassword
                            sshCredentialId: root.sshCredentialId
                            sshKeyPath: root.sshKeyPath
                            sshPassphraseCredentialId: root.sshPassphraseCredentialId
                            sshUsesAgent: root.sshUsesAgent
                            logCommand: root.logCommand
                        }
                    }

                    Loader {
                        active: root.activeTool === "sftp" && root.currentToolReady
                        sourceComponent: SftpBrowserView {
                            sharedSession: root.effectiveSession
                            sshHost: root.sshHost
                            sshPort: root.sshPort
                            sshUser: root.sshUser
                            sshPassword: root.sshPassword
                            sshCredentialId: root.sshCredentialId
                            sshKeyPath: root.sshKeyPath
                            sshPassphraseCredentialId: root.sshPassphraseCredentialId
                            sshUsesAgent: root.sshUsesAgent
                        }
                    }

                    // 8: SQLite
                    Loader {
                        active: root.activeTool === "sqlite"
                        sourceComponent: SqliteBrowserView {}
                    }
                }
            }
        }

        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: Theme.toolRailWidth
            color: Theme.bgInset
            border.color: Theme.borderSubtle
            border.width: 1

            ColumnLayout {
                anchors.fill: parent
                anchors.topMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                spacing: Theme.sp1

                ToolStripButton {
                    icon: "git-branch"
                    tooltip: qsTr("Git")
                    active: root.activeTool === "git"
                    enabled: true
                    onClicked: { root.activeTool = "git"; root.contentExpanded = true }
                }

                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    width: 18
                    height: 1
                    color: Theme.borderSubtle
                }

                ToolStripButton {
                    icon: "activity"
                    tooltip: qsTr("Server Monitor")
                    active: root.activeTool === "monitor"
                    enabled: root.toolAvailable("monitor")
                    onClicked: { root.activeTool = "monitor"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "container"
                    tooltip: qsTr("Docker")
                    active: root.activeTool === "docker"
                    enabled: root.toolAvailable("docker")
                    onClicked: { root.activeTool = "docker"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "database"
                    tooltip: qsTr("MySQL")
                    active: root.activeTool === "mysql"
                    enabled: root.toolAvailable("mysql")
                    onClicked: { root.activeTool = "mysql"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "bolt"
                    tooltip: qsTr("Redis")
                    active: root.activeTool === "redis"
                    enabled: root.toolAvailable("redis")
                    onClicked: { root.activeTool = "redis"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "scroll-text"
                    tooltip: qsTr("Logs")
                    active: root.activeTool === "log"
                    enabled: root.toolAvailable("log")
                    onClicked: { root.activeTool = "log"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "folder-sync"
                    tooltip: qsTr("SFTP")
                    active: root.activeTool === "sftp"
                    enabled: root.toolAvailable("sftp")
                    visible: root.hasRemoteContext
                    onClicked: { root.activeTool = "sftp"; root.contentExpanded = true }
                }

                ToolStripButton {
                    icon: "hard-drive"
                    tooltip: qsTr("SQLite")
                    active: root.activeTool === "sqlite"
                    enabled: true
                    onClicked: { root.activeTool = "sqlite"; root.contentExpanded = true }
                }

                Item { Layout.fillHeight: true }

                ToolStripButton {
                    icon: root.contentExpanded ? "x" : "plus"
                    tooltip: root.contentExpanded ? qsTr("Collapse panel") : qsTr("Expand panel")
                    active: false
                    enabled: true
                    onClicked: root.contentExpanded = !root.contentExpanded
                }
            }
        }
    }

    component ToolStripButton: Rectangle {
        property string icon: ""
        property string tooltip: ""
        property bool active: false
        signal clicked()

        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: 34
        Layout.preferredHeight: 34
        radius: Theme.radiusMd
        color: active ? Theme.bgSelected : stripMouse.containsMouse ? Theme.bgHover : "transparent"
        border.color: active ? Theme.borderFocus : "transparent"
        border.width: active ? 1 : 0
        opacity: enabled ? 1.0 : 0.40

        Rectangle {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            width: 2
            height: 16
            radius: 1
            color: parent.active ? Theme.accent : "transparent"
        }

        Image {
            anchors.centerIn: parent
            source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
            sourceSize: Qt.size(Theme.iconMd, Theme.iconMd)
            layer.enabled: true
            layer.effect: MultiEffect {
                colorization: 1.0
                colorizationColor: parent.active ? Theme.accent : Theme.textSecondary
            }
        }

        MouseArea {
            id: stripMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: parent.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            enabled: parent.enabled
            onClicked: parent.clicked()
        }

        PierToolTip {
            visible: stripMouse.containsMouse
            text: tooltip
        }
    }
}
