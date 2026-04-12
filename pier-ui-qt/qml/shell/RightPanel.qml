import QtQuick
import QtQuick.Layouts
import Pier
import "../views"
import "../components"

// Right Panel container for a specific Terminal tab context.
// Hosts Docker, Logs, MySQL, Redis, etc., tools linked to the SSH session.
Rectangle {
    id: root

    required property string rpTool
    required property string backend
    
    // Remote Context
    property string sshHost: ""
    property int    sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool   sshUsesAgent: false

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

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Header
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 36
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                Text {
                    Layout.fillWidth: true
                    text: {
                        if (root.rpTool === "docker") return qsTr("Docker @ ") + root.sshHost
                        if (root.rpTool === "log") return qsTr("Log Stream")
                        if (root.rpTool === "redis") return qsTr("Redis")
                        if (root.rpTool === "mysql") return qsTr("MySQL")
                        if (root.rpTool === "postgres") return qsTr("PostgreSQL")
                        return qsTr("Tool Panel")
                    }
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                IconButton {
                    icon: "x"
                    onClicked: root.closePanelRequested()
                    Layout.preferredWidth: 24
                    Layout.preferredHeight: 24
                }
            }
        }

        // Content
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: {
                if (root.rpTool === "docker") return 0
                if (root.rpTool === "log") return 1
                if (root.rpTool === "redis") return 2
                if (root.rpTool === "mysql") return 3
                if (root.rpTool === "postgres") return 4
                return -1
            }

            // Index 0: Docker
            Loader {
                active: root.rpTool === "docker"
                sourceComponent: Component {
                    DockerPanelView {
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

            // Index 1: Log
            Loader {
                active: root.rpTool === "log"
                sourceComponent: Component {
                    LogViewerView {
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

            // Index 2: Redis
            Loader {
                active: root.rpTool === "redis"
                sourceComponent: Component {
                    RedisBrowserView {
                        redisHost: root.redisHost
                        redisPort: root.redisPort
                        redisDb: root.redisDb
                    }
                }
            }

            // Index 3: MySQL
            Loader {
                active: root.rpTool === "mysql"
                sourceComponent: Component {
                    MySqlBrowserView {
                        mysqlHost: root.mysqlHost
                        mysqlPort: root.mysqlPort
                        mysqlUser: root.mysqlUser
                        mysqlPassword: root.mysqlPassword
                        mysqlDatabase: root.mysqlDatabase
                    }
                }
            }

            // Index 4: Postgres
            Loader {
                active: root.rpTool === "postgres"
                // Postgres browser is a placeholder conceptually for Pier-X at this point
                Rectangle { color: "transparent" }
            }
        }
    }
}
