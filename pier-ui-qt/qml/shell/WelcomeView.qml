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

    ColumnLayout {
        anchors.centerIn: parent
        width: 520
        spacing: Theme.sp4

        SectionLabel {
            text: qsTr("Welcome")
            Layout.alignment: Qt.AlignHCenter
        }

        Text {
            text: qsTr("Pier-X is taking shape.")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeDisplay
            font.weight: Theme.weightMedium
            font.letterSpacing: -0.7
            color: Theme.textPrimary
            Layout.alignment: Qt.AlignHCenter

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            text: qsTr("Cross-platform terminal management,\nbuilt on Qt 6 + Rust core.")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBodyLg
            color: Theme.textSecondary
            horizontalAlignment: Text.AlignHCenter
            Layout.alignment: Qt.AlignHCenter

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        // Action buttons
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.sp3
            spacing: Theme.sp2

            PrimaryButton {
                text: qsTr("New SSH connection")
                onClicked: root.newSshRequested()
            }
            GhostButton {
                text: qsTr("Open local terminal")
                onClicked: root.openLocalTerminalRequested()
            }
        }

        // ─── Recent connections ────────────────────────
        // Show up to 6 saved connections as clickable cards
        // for one-click reconnection.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.sp4
            spacing: Theme.sp2
            visible: root.connectionsModel && root.connectionsModel.count > 0

            Text {
                text: qsTr("Recent connections")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
                Layout.alignment: Qt.AlignHCenter

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            // Card grid — 2 columns, up to 3 rows (6 connections max).
            Grid {
                Layout.alignment: Qt.AlignHCenter
                columns: 2
                spacing: Theme.sp2

                Repeater {
                    model: {
                        if (!root.connectionsModel) return 0
                        return Math.min(root.connectionsModel.count, 6)
                    }
                    delegate: Rectangle {
                        required property int index
                        width: 250
                        height: 56
                        color: cardMouse.containsMouse ? Theme.bgHover : Theme.bgSurface
                        border.color: cardMouse.containsMouse ? Theme.borderStrong : Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusMd

                        Behavior on color        { ColorAnimation { duration: Theme.durFast } }
                        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            spacing: Theme.sp2

                            // Terminal icon
                            Image {
                                source: "qrc:/qt/qml/Pier/resources/icons/lucide/terminal.svg"
                                sourceSize: Qt.size(16, 16)
                                Layout.alignment: Qt.AlignVCenter
                                layer.enabled: true
                                layer.effect: MultiEffect {
                                    colorization: 1.0
                                    colorizationColor: Theme.accent
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

                                    Behavior on color { ColorAnimation { duration: Theme.durFast } }
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

                                    Behavior on color { ColorAnimation { duration: Theme.durFast } }
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

        // Status pills row — live metadata sourced from pier-core.
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.sp4
            spacing: Theme.sp2

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
}
