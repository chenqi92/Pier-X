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

    signal closePanelRequested()

    readonly property bool hasRemoteContext: root.activeBackend === "ssh" || root.sshHost.length > 0
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
        default: return qsTr("Tool Panel")
        }
    }
    readonly property string panelSubtitle: {
        if (!root.currentToolReady)
            return qsTr("Connect to a server or open a supported context to unlock this panel.")
        if (root.activeTool === "git")
            return PierCore.workingDirectory
        if (root.activeTool === "mysql" && root.mysqlHost.length > 0)
            return root.mysqlUser.length > 0
                    ? root.mysqlUser + "@" + root.mysqlHost + ":" + root.mysqlPort
                    : root.mysqlHost + ":" + root.mysqlPort
        if (root.activeTool === "redis" && root.redisHost.length > 0)
            return root.redisHost + ":" + root.redisPort
        return root.sshUser.length > 0 ? root.sshUser + "@" + root.sshHost + ":" + root.sshPort : root.sshHost
    }

    function toolAvailable(tool) {
        if (tool === "git")
            return true
        if (tool === "mysql")
            return root.mysqlHost.length > 0 || root.hasRemoteContext
        if (tool === "redis")
            return root.redisHost.length > 0 || root.hasRemoteContext
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

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 60
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1

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
                            font.pixelSize: Theme.sizeBody
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
                        icon: "x"
                        tooltip: qsTr("Close panel")
                        onClicked: root.closePanelRequested()
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.leftMargin: Theme.sp2
                Layout.bottomMargin: Theme.sp2
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusLg

                StackLayout {
                    anchors.fill: parent
                    currentIndex: {
                        if (root.activeTool === "git")
                            return 0
                        if (!root.currentToolReady)
                            return 1
                        switch (root.activeTool) {
                        case "monitor": return 2
                        case "docker": return 3
                        case "mysql": return 4
                        case "redis": return 5
                        case "log": return 6
                        case "sftp": return 7
                        default: return 0
                        }
                    }

                    GitPanelView {
                        repoPath: PierCore.workingDirectory
                    }

                    Item {
                        Rectangle {
                            anchors.centerIn: parent
                            width: Math.min(parent.width - Theme.sp8, 300)
                            height: 156
                            radius: Theme.radiusLg
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
                        active: root.activeTool === "monitor" && root.currentToolReady
                        sourceComponent: ServerMonitorView {
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
                        active: root.activeTool === "docker" && root.currentToolReady
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

                    Loader {
                        active: root.activeTool === "mysql" && root.currentToolReady
                        sourceComponent: MySqlBrowserView {
                            mysqlHost: root.mysqlHost
                            mysqlPort: root.mysqlPort
                            mysqlUser: root.mysqlUser
                            mysqlPassword: root.mysqlPassword
                            mysqlDatabase: root.mysqlDatabase
                        }
                    }

                    Loader {
                        active: root.activeTool === "redis" && root.currentToolReady
                        sourceComponent: RedisBrowserView {
                            redisHost: root.redisHost
                            redisPort: root.redisPort
                            redisDb: root.redisDb
                        }
                    }

                    Loader {
                        active: root.activeTool === "log" && root.currentToolReady
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

                    Loader {
                        active: root.activeTool === "sftp" && root.currentToolReady
                        sourceComponent: SftpBrowserView {
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
                    onClicked: root.activeTool = "git"
                }

                Rectangle {
                    Layout.alignment: Qt.AlignHCenter
                    width: 18
                    height: 1
                    color: Theme.borderSubtle
                }

                ToolStripButton {
                    icon: "server"
                    tooltip: qsTr("Server Monitor")
                    active: root.activeTool === "monitor"
                    enabled: root.toolAvailable("monitor")
                    onClicked: root.activeTool = "monitor"
                }

                ToolStripButton {
                    icon: "container"
                    tooltip: qsTr("Docker")
                    active: root.activeTool === "docker"
                    enabled: root.toolAvailable("docker")
                    onClicked: root.activeTool = "docker"
                }

                ToolStripButton {
                    icon: "database"
                    tooltip: qsTr("MySQL")
                    active: root.activeTool === "mysql"
                    enabled: root.toolAvailable("mysql")
                    onClicked: root.activeTool = "mysql"
                }

                ToolStripButton {
                    icon: "layers"
                    tooltip: qsTr("Redis")
                    active: root.activeTool === "redis"
                    enabled: root.toolAvailable("redis")
                    onClicked: root.activeTool = "redis"
                }

                ToolStripButton {
                    icon: "file-text"
                    tooltip: qsTr("Logs")
                    active: root.activeTool === "log"
                    enabled: root.toolAvailable("log")
                    onClicked: root.activeTool = "log"
                }

                ToolStripButton {
                    icon: "folder"
                    tooltip: qsTr("SFTP")
                    active: root.activeTool === "sftp"
                    enabled: root.toolAvailable("sftp")
                    onClicked: root.activeTool = "sftp"
                }

                Item { Layout.fillHeight: true }

                ToolStripButton {
                    icon: "x"
                    tooltip: qsTr("Close panel")
                    active: false
                    enabled: true
                    onClicked: root.closePanelRequested()
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
        Layout.preferredWidth: 30
        Layout.preferredHeight: 30
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
