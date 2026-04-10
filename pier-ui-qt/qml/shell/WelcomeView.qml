import QtQuick
import QtQuick.Layouts
import Pier

// Welcome / empty state — shown when no session is open.
Item {
    id: root

    ColumnLayout {
        anchors.centerIn: parent
        width: 520
        spacing: Theme.sp4

        SectionLabel {
            text: "Welcome"
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
                onClicked: console.log("New SSH — TODO")
            }
            GhostButton {
                text: qsTr("Open local terminal")
                onClicked: console.log("Open local — TODO")
            }
        }

        // Status pills row — design system showcase
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Theme.sp4
            spacing: Theme.sp2

            StatusPill {
                text: "Qt 6.8 LTS"
                statusColor: Theme.statusSuccess
            }
            StatusPill {
                text: "Rust core: pending"
                statusColor: Theme.statusWarning
            }
            StatusPill {
                text: Theme.dark ? "Dark mode" : "Light mode"
                statusColor: Theme.statusInfo
            }
        }
    }
}
