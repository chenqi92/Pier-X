import QtQuick
import QtQuick.Layouts
import Pier

// Left sidebar — connection list and local actions.
Rectangle {
    id: root

    property var connectionsModel: null
    signal addConnectionRequested
    signal connectionActivated(int index)
    signal openLocalTerminalRequested

    implicitWidth: 240
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp4
        spacing: Theme.sp3

        // ─── Connections section ───────────────────────
        RowLayout {
            Layout.fillWidth: true
            SectionLabel {
                text: qsTr("Connections")
                Layout.fillWidth: true
            }
            IconButton {
                glyph: "+"
                tooltip: qsTr("Add connection")
                onClicked: root.addConnectionRequested()
            }
        }

        // Empty state
        Text {
            visible: !root.connectionsModel || root.connectionsModel.count === 0
            text: qsTr("No connections yet.")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        // Connection list
        ListView {
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            interactive: false
            model: root.connectionsModel
            spacing: Theme.sp0_5
            visible: root.connectionsModel && root.connectionsModel.count > 0

            delegate: Rectangle {
                width: ListView.view.width
                implicitHeight: 32
                color: connArea.containsMouse ? Theme.bgHover : "transparent"
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2
                    spacing: 0

                    Text {
                        text: model.name
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightMedium
                        color: Theme.textPrimary
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                    Text {
                        text: model.username + "@" + model.host + ":" + model.port
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    id: connArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.connectionActivated(index)
                }
            }
        }

        Item { Layout.fillHeight: true }

        // ─── Local section ─────────────────────────────
        SectionLabel { text: qsTr("Local") }

        Text {
            text: qsTr("Open terminal")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            color: localArea.containsMouse ? Theme.textPrimary : Theme.textSecondary

            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            MouseArea {
                id: localArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openLocalTerminalRequested()
            }
        }
    }

    // Right border
    Rectangle {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
