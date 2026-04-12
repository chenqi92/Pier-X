import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

Rectangle {
    id: root

    property var connectionsModel: null
    property int selectedSection: 0

    signal addConnectionRequested
    signal connectionActivated(int index)
    signal connectionDeleted(int index)
    signal connectionSftpRequested(int index)
    signal connectionDuplicated(int index)
    signal openLocalTerminalRequested(string path)
    signal openMarkdownRequested(string filePath)

    implicitWidth: Theme.sidebarWidth
    color: Theme.bgPanel

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp3

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: Theme.controlHeight + Theme.sp1
            radius: Theme.radiusLg
            color: Theme.bgInset
            border.color: Theme.borderSubtle
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp0_5
                spacing: Theme.sp0_5

                SidebarTabButton {
                    Layout.fillWidth: true
                    title: qsTr("Files")
                    icon: "folder"
                    active: root.selectedSection === 0
                    onClicked: root.selectedSection = 0
                }

                SidebarTabButton {
                    Layout.fillWidth: true
                    title: qsTr("Servers")
                    icon: "server"
                    active: root.selectedSection === 1
                    onClicked: root.selectedSection = 1
                }
            }
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: root.selectedSection

            LocalFilesPane {
                Layout.fillWidth: true
                Layout.fillHeight: true
                onMarkdownRequested: (filePath) => root.openMarkdownRequested(filePath)
                onOpenTerminalRequested: (path) => root.openLocalTerminalRequested(path)
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp3

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 60
                        radius: Theme.radiusLg
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp2
                            spacing: Theme.sp2

                            Rectangle {
                                width: 24
                                height: 24
                                radius: Theme.radiusMd
                                color: Theme.accentMuted
                                border.color: Theme.borderSubtle
                                border.width: 1

                                Image {
                                    anchors.centerIn: parent
                                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/server.svg"
                                    sourceSize: Qt.size(Theme.iconSm, Theme.iconSm)
                                    layer.enabled: true
                                    layer.effect: MultiEffect {
                                        colorization: 1.0
                                        colorizationColor: Theme.accent
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0

                                Text {
                                    text: qsTr("Saved Connections")
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeBody
                                    font.weight: Theme.weightSemibold
                                    color: Theme.textPrimary
                                }

                                Text {
                                    text: qsTr("%1 hosts ready for SSH, SFTP, and service panels.")
                                          .arg(root.connectionsModel ? root.connectionsModel.count : 0)
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textTertiary
                                    wrapMode: Text.WordWrap
                                }
                            }

                            IconButton {
                                icon: "plus"
                                tooltip: qsTr("Add connection")
                                onClicked: root.addConnectionRequested()
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        radius: Theme.radiusLg
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1

                        Item {
                            anchors.fill: parent
                            anchors.margins: Theme.sp2

                            Text {
                                anchors.centerIn: parent
                                visible: !root.connectionsModel || root.connectionsModel.count === 0
                                text: qsTr("No saved connections yet.\nCreate one to unlock SSH, SFTP, Docker, logs, and database tools.")
                                horizontalAlignment: Text.AlignHCenter
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textTertiary
                                wrapMode: Text.WordWrap
                                width: parent.width - Theme.sp6
                            }

                            ListView {
                                id: connList
                                anchors.fill: parent
                                clip: true
                                interactive: true
                                model: root.connectionsModel
                                spacing: Theme.sp1
                                visible: root.connectionsModel && root.connectionsModel.count > 0

                                delegate: Rectangle {
                                    id: connRow

                                    required property int index
                                    required property string name
                                    required property string username
                                    required property string host
                                    required property int port
                                    required property string keyPath
                                    required property bool usesAgent

                                    property bool confirmingDelete: false
                                    readonly property string authLabel: usesAgent
                                            ? qsTr("Agent")
                                            : keyPath.length > 0 ? qsTr("Key") : qsTr("Password")
                                    readonly property color authTint: usesAgent
                                            ? Theme.accent
                                            : keyPath.length > 0 ? Theme.statusSuccess : Theme.statusWarning

                                    width: ListView.view.width
                                    implicitHeight: 58
                                    radius: Theme.radiusMd
                                    color: confirmingDelete
                                           ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.08)
                                           : connArea.containsMouse ? Theme.bgHover : Theme.bgCanvas
                                    border.color: confirmingDelete
                                                  ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.22)
                                                  : connArea.containsMouse ? Theme.borderStrong : Theme.borderSubtle
                                    border.width: 1

                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp3
                                        anchors.rightMargin: Theme.sp2
                                        spacing: Theme.sp2
                                        visible: !connRow.confirmingDelete

                                        Rectangle {
                                            width: 24
                                            height: 24
                                            radius: Theme.radiusMd
                                            color: Qt.rgba(connRow.authTint.r, connRow.authTint.g, connRow.authTint.b, Theme.dark ? 0.16 : 0.10)
                                            border.color: Qt.rgba(connRow.authTint.r, connRow.authTint.g, connRow.authTint.b, Theme.dark ? 0.24 : 0.18)
                                            border.width: 1

                                            Rectangle {
                                                anchors.centerIn: parent
                                                width: 6
                                                height: 6
                                                radius: 3
                                                color: connRow.authTint
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: Theme.sp0_5

                                            Text {
                                                text: connRow.name
                                                font.family: Theme.fontUi
                                                font.pixelSize: Theme.sizeBody
                                                font.weight: Theme.weightMedium
                                                color: Theme.textPrimary
                                                elide: Text.ElideRight
                                                Layout.fillWidth: true
                                            }

                                            Text {
                                                text: connRow.username + "@" + connRow.host + ":" + connRow.port
                                                font.family: Theme.fontMono
                                                font.pixelSize: Theme.sizeCaption
                                                color: Theme.textTertiary
                                                elide: Text.ElideMiddle
                                                Layout.fillWidth: true
                                            }
                                        }

                                        Rectangle {
                                            implicitHeight: 20
                                            implicitWidth: authText.implicitWidth + Theme.sp2 * 2
                                            radius: Theme.radiusPill
                                            color: Qt.rgba(connRow.authTint.r, connRow.authTint.g, connRow.authTint.b, Theme.dark ? 0.14 : 0.08)
                                            border.color: Qt.rgba(connRow.authTint.r, connRow.authTint.g, connRow.authTint.b, Theme.dark ? 0.22 : 0.16)
                                            border.width: 1

                                            Text {
                                                id: authText
                                                anchors.centerIn: parent
                                                text: connRow.authLabel
                                                font.family: Theme.fontUi
                                                font.pixelSize: Theme.sizeSmall
                                                font.weight: Theme.weightMedium
                                                color: connRow.authTint
                                            }
                                        }

                                        IconButton {
                                            icon: "x"
                                            iconSize: Theme.iconXs
                                            tooltip: qsTr("Delete")
                                            opacity: connArea.containsMouse ? 1.0 : 0.0
                                            onClicked: connRow.confirmingDelete = true
                                        }
                                    }

                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp3
                                        anchors.rightMargin: Theme.sp2
                                        spacing: Theme.sp2
                                        visible: connRow.confirmingDelete

                                        Text {
                                            Layout.fillWidth: true
                                            text: qsTr("Delete “%1”?").arg(connRow.name)
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            font.weight: Theme.weightMedium
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                        }

                                        GhostButton {
                                            text: qsTr("Cancel")
                                            onClicked: connRow.confirmingDelete = false
                                        }

                                        PrimaryButton {
                                            text: qsTr("Delete")
                                            onClicked: root.connectionDeleted(connRow.index)
                                        }
                                    }

                                    MouseArea {
                                        id: connArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        acceptedButtons: Qt.LeftButton | Qt.RightButton
                                        cursorShape: connRow.confirmingDelete ? Qt.ArrowCursor : Qt.PointingHandCursor
                                        z: -1
                                        onClicked: (mouse) => {
                                            if (mouse.button === Qt.RightButton) {
                                                const pos = connRow.mapToItem(root, mouse.x, mouse.y + connRow.height)
                                                contextMenu.targetIndex = connRow.index
                                                contextMenu.x = Math.max(Theme.sp2,
                                                                         Math.min(pos.x,
                                                                                  root.width - contextMenu.width - Theme.sp2))
                                                contextMenu.y = Math.max(Theme.sp2,
                                                                         Math.min(pos.y,
                                                                                  root.height - contextMenu.height - Theme.sp2))
                                                contextMenu.visible = true
                                                return
                                            }

                                            if (connRow.confirmingDelete)
                                                connRow.confirmingDelete = false
                                            else
                                                root.connectionActivated(connRow.index)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        visible: contextMenu.visible
        z: 199
        onClicked: contextMenu.visible = false
    }

    Rectangle {
        id: contextMenu

        property int targetIndex: -1

        visible: false
        z: 200
        width: 176
        height: ctxCol.implicitHeight + Theme.sp1 * 2
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: Theme.dark ? 0.30 : 0.14
            shadowBlur: 0.9
            shadowVerticalOffset: 8
        }

        Column {
            id: ctxCol
            anchors.fill: parent
            anchors.margins: Theme.sp1
            spacing: Theme.sp0_5

            Repeater {
                model: [
                    { label: qsTr("Connect"), action: "connect" },
                    { label: qsTr("SFTP"), action: "sftp" },
                    { label: qsTr("Duplicate"), action: "duplicate" },
                    { label: qsTr("Delete"), action: "delete" }
                ]

                delegate: Rectangle {
                    width: ctxCol.width
                    implicitHeight: Theme.controlHeight
                    radius: Theme.radiusSm
                    color: ctxItemArea.containsMouse
                         ? (modelData.action === "delete"
                            ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.10)
                            : Theme.bgHover)
                         : "transparent"

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp3
                        anchors.rightMargin: Theme.sp2
                        verticalAlignment: Text.AlignVCenter
                        text: modelData.label
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightMedium
                        color: modelData.action === "delete" ? Theme.statusError : Theme.textPrimary
                    }

                    MouseArea {
                        id: ctxItemArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            const idx = contextMenu.targetIndex
                            contextMenu.visible = false
                            if (modelData.action === "connect")
                                root.connectionActivated(idx)
                            else if (modelData.action === "sftp")
                                root.connectionSftpRequested(idx)
                            else if (modelData.action === "duplicate")
                                root.connectionDuplicated(idx)
                            else if (modelData.action === "delete")
                                root.connectionDeleted(idx)
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        width: 1
        color: Theme.borderSubtle
    }

    component SidebarTabButton: Rectangle {
        property string title: ""
        property string icon: ""
        property bool active: false
        signal clicked

        implicitHeight: Theme.controlHeight
        radius: Theme.radiusMd
        color: active ? Theme.bgSurface : tabMouse.containsMouse ? Theme.bgHover : "transparent"
        border.color: active ? Theme.borderSubtle : "transparent"
        border.width: active ? 1 : 0

        Row {
            anchors.centerIn: parent
            spacing: Theme.sp1_5

            Image {
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
                sourceSize: Qt.size(Theme.iconSm, Theme.iconSm)
                layer.enabled: true
                layer.effect: MultiEffect {
                    colorization: 1.0
                    colorizationColor: active ? Theme.accent : Theme.textTertiary
                }
            }

            Text {
                text: title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                font.weight: active ? Theme.weightMedium : Theme.weightRegular
                color: active ? Theme.textPrimary : Theme.textSecondary
            }
        }

        MouseArea {
            id: tabMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }
    }
}
