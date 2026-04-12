import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../views"
import "../components"

// Permanent right sidebar — mirrors the left sidebar pattern.
// Vertical icon strip for tool switching + content area.
// Local tools (Git) are always available; remote tools
// (Docker, MySQL, Redis, Logs) activate from the current tab context.
Rectangle {
    id: root

    // Current active tab context (bound from Main.qml)
    property string activeTool: "git"
    property string activeBackend: ""

    // SSH context from the active tab
    property string sshHost: ""
    property int    sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool   sshUsesAgent: false

    // Service context
    property string redisHost: ""
    property int    redisPort: 0
    property int    redisDb: 0
    property string logCommand: ""
    property string mysqlHost: ""
    property int    mysqlPort: 3306
    property string mysqlUser: ""
    property string mysqlPassword: ""
    property string mysqlDatabase: ""
    property string pgHost: ""
    property int    pgPort: 5432
    property string pgUser: ""
    property string pgDatabase: ""

    signal closePanelRequested()

    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // ── Content area ────────────────────────────────
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: {
                switch (root.activeTool) {
                case "git":    return 0
                case "docker": return 1
                case "redis":  return 2
                case "mysql":  return 3
                case "log":    return 4
                default:       return 0
                }
            }

            // 0: Git
            GitPanelView {
                repoPath: PierCore.workingDirectory
                onClosePanelRequested: root.closePanelRequested()
            }

            // 1: Docker
            Loader {
                active: root.activeTool === "docker" && root.activeBackend === "ssh"
                sourceComponent: DockerPanelView {
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

            // 2: Redis
            Loader {
                active: root.activeTool === "redis"
                sourceComponent: RedisBrowserView {
                    redisHost: root.redisHost
                    redisPort: root.redisPort
                    redisDb: root.redisDb
                }
            }

            // 3: MySQL
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

            // 4: Log
            Loader {
                active: root.activeTool === "log"
                sourceComponent: LogViewerView {
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
        }

        // ── Vertical tool strip ─────────────────────────
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 36
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                anchors.fill: parent
                anchors.topMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                spacing: Theme.sp0_5

                // ── Local tools ─────────────────────────
                ToolStripButton {
                    icon: "git-branch"
                    tooltip: qsTr("Git")
                    active: root.activeTool === "git"
                    onClicked: root.activeTool = "git"
                }

                // ── Separator ───────────────────────────
                Rectangle {
                    Layout.fillWidth: true
                    Layout.leftMargin: Theme.sp2
                    Layout.rightMargin: Theme.sp2
                    Layout.topMargin: Theme.sp1
                    Layout.bottomMargin: Theme.sp1
                    height: 1
                    color: Theme.borderSubtle
                    visible: root.activeBackend === "ssh"
                }

                // ── Remote tools (only when SSH tab active) ──
                ToolStripButton {
                    icon: "container"
                    tooltip: qsTr("Docker")
                    active: root.activeTool === "docker"
                    visible: root.activeBackend === "ssh"
                    onClicked: root.activeTool = "docker"
                }

                ToolStripButton {
                    icon: "database"
                    tooltip: qsTr("MySQL")
                    active: root.activeTool === "mysql"
                    visible: root.activeBackend === "ssh"
                    onClicked: root.activeTool = "mysql"
                }

                ToolStripButton {
                    icon: "layers"
                    tooltip: qsTr("Redis")
                    active: root.activeTool === "redis"
                    visible: root.activeBackend === "ssh"
                    onClicked: root.activeTool = "redis"
                }

                ToolStripButton {
                    icon: "file-text"
                    tooltip: qsTr("Logs")
                    active: root.activeTool === "log"
                    visible: root.activeBackend === "ssh"
                    onClicked: root.activeTool = "log"
                }

                Item { Layout.fillHeight: true }

                // Close button at bottom
                ToolStripButton {
                    icon: "x"
                    tooltip: qsTr("Close panel")
                    active: false
                    onClicked: root.closePanelRequested()
                }
            }
        }
    }

    // Inline component for tool strip buttons
    component ToolStripButton: Rectangle {
        property string icon: ""
        property string tooltip: ""
        property bool active: false
        signal clicked()

        Layout.alignment: Qt.AlignHCenter
        Layout.preferredWidth: 28
        Layout.preferredHeight: 28
        radius: Theme.radiusSm
        color: active ? Theme.accentMuted
             : stripMouse.containsMouse ? Theme.bgHover
             : "transparent"
        border.color: active ? Theme.borderFocus : "transparent"
        border.width: active ? 1 : 0

        Behavior on color { ColorAnimation { duration: Theme.durFast } }

        Image {
            anchors.centerIn: parent
            source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
            sourceSize: Qt.size(16, 16)
            layer.enabled: true
            layer.effect: MultiEffect {
                colorization: 1.0
                colorizationColor: active ? Theme.accent : Theme.textSecondary
            }
        }

        MouseArea {
            id: stripMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }

        PierToolTip {
            visible: stripMouse.containsMouse
            text: tooltip
        }
    }
}
