import QtQuick
import QtQuick.Layouts
import Pier

// Left sidebar — connection list and local actions.
Rectangle {
    id: root

    property var connectionsModel: null
    signal addConnectionRequested
    signal connectionActivated(int index)
    signal connectionDeleted(int index)
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
                id: connRow

                // Capture the index that the model exposes to
                // this delegate, since `index` is shadowed by
                // the inline MouseArea below.
                required property int index
                required property string name
                required property string username
                required property string host
                required property int port

                width: ListView.view.width
                implicitHeight: 32
                color: connArea.containsMouse ? Theme.bgHover : "transparent"
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp1
                    spacing: Theme.sp1

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

                    // Hover-revealed Delete button. Sits above
                    // the click area below so its own MouseArea
                    // wins for clicks on the icon. The whole
                    // row's MouseArea handles clicks on the
                    // text + everything else.
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

                        Text {
                            anchors.centerIn: parent
                            text: "×"
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBodyLg
                            font.weight: Theme.weightMedium
                            color: deleteArea.containsMouse
                                   ? Theme.statusError
                                   : Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }
                        }

                        MouseArea {
                            id: deleteArea
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            // M3c4 polish: a confirmation dialog
                            // would be nicer than instant delete.
                            // For M3c3 we accept that the action
                            // is destructive-but-undoable (the
                            // user can re-add via the dialog).
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
                    cursorShape: Qt.PointingHandCursor
                    // Higher z stays on the fill of the row but
                    // does NOT receive clicks targeted at the
                    // deleteBtn above (deleteArea has the higher
                    // visual stacking order via the RowLayout).
                    z: -1
                    onClicked: root.connectionActivated(connRow.index)
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
