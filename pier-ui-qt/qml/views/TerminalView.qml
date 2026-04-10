import QtQuick
import Pier

// Placeholder for the terminal area. Replaced once pier-core's PTY backend
// is wired in via cxx-qt and we have an actual VTE-rendering surface.
Rectangle {
    id: root

    property string title: ""

    color: Theme.bgCanvas

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    Column {
        anchors.centerIn: parent
        spacing: Theme.sp3

        Text {
            text: root.title
            anchors.horizontalCenter: parent.horizontalCenter
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeH2
            font.weight: Theme.weightMedium
            color: Theme.textSecondary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            text: qsTr("Terminal placeholder — pier-core PTY backend pending.")
            anchors.horizontalCenter: parent.horizontalCenter
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeBody
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }
}
