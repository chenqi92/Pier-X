import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Welcome / empty state — shown when no session is open.
// Displays hero text, action buttons, and a "Recent connections"
// quick-access card grid sourced from PierConnectionStore.
Item {
    id: root

    // Forwarded to Main.qml which owns the tab model and the
    // connection dialog. Using signals (not direct imperative
    // calls) keeps WelcomeView reusable and testable in isolation.
    signal openLocalTerminalRequested()
    signal newSshRequested()
    signal connectToSaved(int index)

    // The connections model — wired by Main.qml to the
    // PierConnectionStore instance.
    property var connectionsModel: null
    readonly property int recentConnectionCount: {
        if (!root.connectionsModel)
            return 0
        if (typeof root.connectionsModel.count === "number")
            return root.connectionsModel.count
        return 0
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.sp12 * 2, 480)
        spacing: Theme.sp4

        ColumnLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            spacing: Theme.sp2

            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 28
                Layout.preferredHeight: 28
                radius: Theme.radiusMd
                color: Theme.accentSubtle

                Rectangle {
                    anchors.centerIn: parent
                    width: 8
                    height: 8
                    radius: 4
                    color: Theme.accent
                }
            }

            SectionLabel {
                text: qsTr("Welcome")
                Layout.alignment: Qt.AlignHCenter
            }

            Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Pier-X workspace")
                font.family: Theme.fontUi
                font.pixelSize: 28
                font.weight: Theme.weightSemibold
                font.letterSpacing: -0.4
                color: Theme.textPrimary
                horizontalAlignment: Text.AlignHCenter
            }

            Text {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumWidth: 420
                text: qsTr("Open a local terminal or connect to a server to start working.")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textSecondary
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: Theme.sp1_5

                PrimaryButton {
                    Layout.preferredWidth: 148
                    text: qsTr("New SSH connection")
                    onClicked: root.newSshRequested()
                }

                GhostButton {
                    Layout.preferredWidth: 148
                    text: qsTr("Open local terminal")
                    onClicked: root.openLocalTerminalRequested()
                }
            }

            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: Theme.sp1_5

                StatusPill {
                    text: "Qt " + PierCore.qtVersion
                    statusColor: Theme.statusSuccess
                }

                StatusPill {
                    text: qsTr("core ") + PierCore.version
                    statusColor: Theme.statusSuccess
                }

                StatusPill {
                    text: Theme.dark ? qsTr("Dark mode") : qsTr("Light mode")
                    statusColor: Theme.statusInfo
                }
            }
        }

        Card {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            visible: root.recentConnectionCount > 0
            padding: Theme.sp3

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Text {
                        text: qsTr("Recent connections")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightSemibold
                        color: Theme.textPrimary
                    }

                    Text {
                        text: qsTr("%1 saved").arg(root.recentConnectionCount)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }

                    Item { Layout.fillWidth: true }
                }

                Grid {
                    Layout.fillWidth: true
                    columns: 2
                    rowSpacing: Theme.sp1_5
                    columnSpacing: Theme.sp2

                    Repeater {
                        model: Math.min(root.recentConnectionCount, 6)

                        delegate: Rectangle {
                            required property int index
                            width: (root.width - Theme.sp4 * 2 - Theme.sp2) / 2
                            height: 54
                            color: cardMouse.containsMouse ? Theme.bgHover : Theme.bgInset
                            border.color: cardMouse.containsMouse ? Theme.borderDefault : "transparent"
                            border.width: cardMouse.containsMouse ? 1 : 0
                            radius: Theme.radiusMd

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }
                            Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp3
                                anchors.rightMargin: Theme.sp3
                                spacing: Theme.sp2

                                Rectangle {
                                    Layout.preferredWidth: 20
                                    Layout.preferredHeight: 20
                                    radius: Theme.radiusSm
                                    color: Theme.accentSubtle

                                    Image {
                                        anchors.centerIn: parent
                                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/terminal.svg"
                                        sourceSize: Qt.size(14, 14)
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
                                        text: root.connectionsModel.get(index).name || ""
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                        Layout.fillWidth: true
                                    }

                                    Text {
                                        text: {
                                            const c = root.connectionsModel.get(index)
                                            return (c.username || "") + "@" + (c.host || "") + ":" + (c.port || 22)
                                        }
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                        elide: Text.ElideRight
                                        Layout.fillWidth: true
                                    }
                                }
                            }

                            MouseArea {
                                id: cardMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root.connectToSaved(index)
                            }
                        }
                    }
                }
            }
        }
    }
}
