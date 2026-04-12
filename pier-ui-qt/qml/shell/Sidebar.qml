import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Left sidebar — adapted toward the original Pier shell:
// a Files / Servers switcher instead of a flat connection list.
Rectangle {
    id: root

    property var connectionsModel: null
    property int selectedSection: 0

    signal addConnectionRequested
    signal connectionActivated(int index)
    signal connectionDeleted(int index)
    signal connectionSftpRequested(int index)
    signal connectionDuplicated(int index)
    signal openLocalTerminalRequested
    signal openMarkdownRequested(string filePath)

    implicitWidth: 220
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp2
        spacing: Theme.sp2

        // Segmented tab switcher
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 32
            radius: Theme.radiusSm
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1

            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            RowLayout {
                anchors.fill: parent
                anchors.margins: 3
                spacing: 2

                Repeater {
                    model: [
                        { title: qsTr("Files"), index: 0 },
                        { title: qsTr("Servers"), index: 1 }
                    ]

                    delegate: Rectangle {
                        required property var modelData

                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        radius: Theme.radiusSm - 1
                        color: root.selectedSection === modelData.index
                               ? Theme.bgElevated
                               : "transparent"

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        // Subtle shadow on the active tab
                        layer.enabled: root.selectedSection === modelData.index
                        layer.effect: MultiEffect {
                            shadowEnabled: true
                            shadowColor: "#000000"
                            shadowOpacity: 0.08
                            shadowBlur: 0.2
                            shadowVerticalOffset: 1
                        }

                        Text {
                            anchors.centerIn: parent
                            text: modelData.title
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            font.weight: root.selectedSection === modelData.index
                                         ? Theme.weightSemibold
                                         : Theme.weightRegular
                            color: root.selectedSection === modelData.index
                                   ? Theme.textPrimary
                                   : Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }
                        }

                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.selectedSection = modelData.index
                        }
                    }
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
                onOpenTerminalRequested: root.openLocalTerminalRequested()
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp3

                    RowLayout {
                        Layout.fillWidth: true

                        SectionLabel {
                            text: qsTr("Connections")
                            Layout.fillWidth: true
                        }

                        IconButton {
                            icon: "plus"
                            tooltip: qsTr("Add connection")
                            onClicked: root.addConnectionRequested()
                        }
                    }

                    Text {
                        visible: !root.connectionsModel || root.connectionsModel.count === 0
                        text: qsTr("No connections yet.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textTertiary
                        wrapMode: Text.WordWrap

                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    }

                    ListView {
                        id: connList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        interactive: true
                        model: root.connectionsModel
                        spacing: Theme.sp0_5
                        visible: root.connectionsModel && root.connectionsModel.count > 0

                        delegate: Rectangle {
                            id: connRow

                            required property int index
                            required property string name
                            required property string username
                            required property string host
                            required property int port

                            property bool confirmingDelete: false

                            width: ListView.view.width
                            implicitHeight: 40
                            color: confirmingDelete
                                   ? Qt.rgba(Theme.statusError.r,
                                             Theme.statusError.g,
                                             Theme.statusError.b,
                                             0.08)
                                   : (connArea.containsMouse ? Theme.bgHover : "transparent")
                            radius: Theme.radiusSm

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp1
                                spacing: Theme.sp1
                                visible: !connRow.confirmingDelete

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 0

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
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                        elide: Text.ElideRight
                                        Layout.fillWidth: true
                                    }
                                }

                                Rectangle {
                                    id: deleteBtn
                                    Layout.preferredWidth: 22
                                    Layout.preferredHeight: 22
                                    radius: Theme.radiusSm
                                    color: deleteArea.containsMouse
                                           ? Qt.rgba(Theme.statusError.r,
                                                     Theme.statusError.g,
                                                     Theme.statusError.b,
                                                     0.16)
                                           : "transparent"
                                    opacity: connArea.containsMouse || deleteArea.containsMouse
                                             ? 1.0 : 0.0

                                    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
                                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                    Image {
                                        anchors.centerIn: parent
                                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/x.svg"
                                        sourceSize: Qt.size(12, 12)
                                        layer.enabled: true
                                        layer.effect: MultiEffect {
                                            colorization: 1.0
                                            colorizationColor: deleteArea.containsMouse
                                                               ? Theme.statusError
                                                               : Theme.textTertiary
                                        }
                                    }

                                    MouseArea {
                                        id: deleteArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: (mouse) => {
                                            connRow.confirmingDelete = true
                                            mouse.accepted = true
                                        }
                                    }
                                }
                            }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp1
                                spacing: Theme.sp2
                                visible: connRow.confirmingDelete

                                Text {
                                    Layout.fillWidth: true
                                    text: qsTr("Delete “%1”?").arg(connRow.name)
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeCaption
                                    font.weight: Theme.weightMedium
                                    color: Theme.textPrimary
                                    elide: Text.ElideRight
                                }

                                Rectangle {
                                    Layout.preferredWidth: cancelLabel.implicitWidth + Theme.sp2 * 2
                                    Layout.preferredHeight: 20
                                    radius: Theme.radiusSm
                                    color: cancelArea.containsMouse ? Theme.bgHover : "transparent"
                                    border.color: Theme.borderDefault
                                    border.width: 1

                                    Text {
                                        id: cancelLabel
                                        anchors.centerIn: parent
                                        text: qsTr("Cancel")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        font.weight: Theme.weightMedium
                                        color: Theme.textSecondary
                                    }

                                    MouseArea {
                                        id: cancelArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: (mouse) => {
                                            connRow.confirmingDelete = false
                                            mouse.accepted = true
                                        }
                                    }
                                }

                                Rectangle {
                                    Layout.preferredWidth: confirmLabel.implicitWidth + Theme.sp2 * 2
                                    Layout.preferredHeight: 20
                                    radius: Theme.radiusSm
                                    color: confirmArea.containsMouse
                                           ? Qt.rgba(Theme.statusError.r,
                                                     Theme.statusError.g,
                                                     Theme.statusError.b,
                                                     0.24)
                                           : Qt.rgba(Theme.statusError.r,
                                                     Theme.statusError.g,
                                                     Theme.statusError.b,
                                                     0.16)

                                    Text {
                                        id: confirmLabel
                                        anchors.centerIn: parent
                                        text: qsTr("Delete")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        font.weight: Theme.weightMedium
                                        color: Theme.statusError
                                    }

                                    MouseArea {
                                        id: confirmArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: (mouse) => {
                                            root.connectionDeleted(connRow.index)
                                            mouse.accepted = true
                                        }
                                    }
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
                                        var pos = connRow.mapToItem(root, mouse.x, mouse.y + connRow.height)
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
                                    if (connRow.confirmingDelete) {
                                        connRow.confirmingDelete = false
                                    } else {
                                        root.connectionActivated(connRow.index)
                                    }
                                }
                            }
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Right-click a server to open SFTP, duplicate it, or delete it.")
                        visible: root.connectionsModel && root.connectionsModel.count > 0
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        wrapMode: Text.WordWrap
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
        width: 164
        height: ctxCol.implicitHeight + Theme.sp1 * 2
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusMd

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.32
            shadowBlur: 0.6
            shadowVerticalOffset: 4
        }

        Column {
            id: ctxCol
            anchors.fill: parent
            anchors.margins: Theme.sp1

            Repeater {
                model: [
                    { label: qsTr("Connect"), action: "connect" },
                    { label: qsTr("SFTP"), action: "sftp" },
                    { label: qsTr("Duplicate"), action: "duplicate" },
                    { label: qsTr("Delete"), action: "delete" }
                ]

                delegate: Rectangle {
                    width: ctxCol.width
                    implicitHeight: 28
                    color: ctxItemArea.containsMouse
                         ? (modelData.action === "delete"
                            ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.10)
                            : Theme.bgHover)
                         : "transparent"
                    radius: Theme.radiusSm

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        verticalAlignment: Text.AlignVCenter
                        text: modelData.label
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
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
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
