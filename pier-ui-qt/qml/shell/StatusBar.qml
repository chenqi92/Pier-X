import QtQuick
import QtQuick.Layouts
import Pier

// Bottom status bar — short status text + version label.
Rectangle {
    id: root

    implicitHeight: 24
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        spacing: Theme.sp3

        Text {
            text: qsTr("Ready")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Item { Layout.fillWidth: true }

        Text {
            text: qsTr("Qt") + " " + PierCore.qtVersion
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            // App version (from CMake) · pier-core build info (from Rust FFI).
            // The "·" separator groups them visually without a second Text.
            text: "v" + Qt.application.version + " · core " + PierCore.buildInfo
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }

    // Top 1px border
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
