import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Left sidebar — connection list and local actions.
Rectangle {
    id: root

    property var connectionsModel: null
    signal addConnectionRequested
    signal connectionActivated(int index)
    signal connectionDeleted(int index)
    signal connectionSftpRequested(int index)
    signal connectionDuplicated(int index)
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
                icon: "plus"
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

                // Two-state delete: hovered × click flips this
                // to true, which swaps the row contents to a
                // small inline confirmation strip. Clicking
                // anywhere else in the sidebar or on the row
                // itself resets it.
                property bool confirmingDelete: false

                width: ListView.view.width
                implicitHeight: 32
                color: confirmingDelete
                       ? Qt.rgba(Theme.statusError.r,
                                 Theme.statusError.g,
                                 Theme.statusError.b,
                                 0.08)
                       : (connArea.containsMouse ? Theme.bgHover : "transparent")
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                // ── Default state: name / target / × ──────
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

                // ── Confirm state: prompt + Cancel / Delete ──
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

                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    }

                    // Cancel pill — monochromatic, low visual weight.
                    Rectangle {
                        Layout.preferredWidth: cancelLabel.implicitWidth + Theme.sp2 * 2
                        Layout.preferredHeight: 20
                        radius: Theme.radiusSm
                        color: cancelArea.containsMouse ? Theme.bgHover : "transparent"
                        border.color: Theme.borderDefault
                        border.width: 1

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

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

                    // Destructive confirm pill — accent red
                    // background at 16%, error-colored label.
                    // Breaks the single-accent rule of the
                    // design system ONLY because this is a
                    // destructive action — §2 status colors are
                    // the explicit exception.
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

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

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
                                // Fire AND leave confirmingDelete
                                // true — the model removes the
                                // row so this delegate gets
                                // unmounted anyway.
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
                            // Show context menu
                            contextMenu.targetIndex = connRow.index
                            contextMenu.x = mouse.x + connRow.x
                            contextMenu.y = mouse.y + connRow.y + connRow.height
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

        // ─── Context menu ──────────────────────────────
        // Self-drawn popup for right-click actions on a connection.
        // Floats over the sidebar content.
        Rectangle {
            id: contextMenu
            property int targetIndex: -1

            visible: false
            z: 200
            width: 160
            height: ctxCol.implicitHeight + Theme.sp1 * 2
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusMd

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
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
                             ? (modelData.action === "delete" ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.1) : Theme.bgHover)
                             : "transparent"
                        radius: Theme.radiusSm

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2
                            verticalAlignment: Text.AlignVCenter
                            text: modelData.label
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightRegular
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

            // Dismiss overlay — closes the menu when clicking outside it
            MouseArea {
                parent: root
                anchors.fill: parent
                visible: contextMenu.visible
                z: 199
                onClicked: contextMenu.visible = false
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
