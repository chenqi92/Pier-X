import QtQuick
import QtQuick.Layouts
import Pier

// Welcome / empty state — shown when no session is open.
Item {
    id: root

    // Forwarded to Main.qml which owns the tab model and the
    // connection dialog. Using signals (not direct imperative
    // calls) keeps WelcomeView reusable and testable in isolation.
    signal openLocalTerminalRequested()
    signal newSshRequested()

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
                onClicked: root.newSshRequested()
            }
            GhostButton {
                text: qsTr("Open local terminal")
                onClicked: root.openLocalTerminalRequested()
            }
        }

        // Status pills row — live metadata sourced from pier-core.
        // Both the Qt runtime version and the pier-core crate
        // version are real bindings now (M1 bridge); they update
        // whenever the underlying library changes at build time.
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
