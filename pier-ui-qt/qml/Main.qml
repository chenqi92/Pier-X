import QtQuick
import QtQuick.Window
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier

ApplicationWindow {
    id: window
    width: 1280
    height: 800
    minimumWidth: 800
    minimumHeight: 500
    visible: true
    title: qsTr("Pier-X")

    color: Theme.bgCanvas
    Behavior on color {
        ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType }
    }

    // ─────────────────────────────────────────────────────
    // Three-column IDE shell skeleton
    // (left sidebar · vertical separator · main content)
    // ─────────────────────────────────────────────────────
    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Left sidebar ────────────────────────────────────
        Rectangle {
            Layout.preferredWidth: 240
            Layout.fillHeight: true
            color: Theme.bgPanel

            Behavior on color {
                ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType }
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp4
                spacing: Theme.sp3

                Text {
                    text: qsTr("Pier-X")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeH2
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Text {
                    text: qsTr("Cross-platform terminal manager")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Item { Layout.fillHeight: true }

                // Bottom section: version label
                Text {
                    text: qsTr("v") + Qt.application.version
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }
        }

        // Vertical separator ──────────────────────────────
        Rectangle {
            Layout.preferredWidth: 1
            Layout.fillHeight: true
            color: Theme.borderSubtle
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        // Main content area ───────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.bgCanvas
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                anchors.centerIn: parent
                spacing: Theme.sp4

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
                    text: qsTr("Cross-platform terminal management, built on Qt 6 + Rust core.")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBodyLg
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignHCenter
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                // Theme toggle (primary button) ───────────
                Rectangle {
                    id: themeButton
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: Theme.sp4
                    implicitHeight: 32
                    implicitWidth: themeButtonLabel.implicitWidth + Theme.sp4 * 2
                    color: themeButtonArea.containsMouse ? Theme.accentHover : Theme.accent
                    radius: Theme.radiusSm
                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    Text {
                        id: themeButtonLabel
                        anchors.centerIn: parent
                        text: Theme.dark ? qsTr("Switch to light") : qsTr("Switch to dark")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightMedium
                        color: Theme.textInverse
                    }

                    MouseArea {
                        id: themeButtonArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: Theme.dark = !Theme.dark
                    }
                }
            }
        }
    }
}
