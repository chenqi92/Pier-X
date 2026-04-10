import QtQuick
import QtQuick.Layouts
import Pier

// Left sidebar — connection list / navigation.
// Currently shows an empty state until the connection manager lands.
Rectangle {
    id: root

    implicitWidth: 240
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp4
        spacing: Theme.sp3

        SectionLabel { text: "Connections" }

        Text {
            Layout.topMargin: Theme.sp1
            text: "No connections yet."
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            id: addLink
            text: "+ Add connection"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: Theme.weightMedium
            color: addArea.containsMouse ? Theme.accentHover : Theme.accent

            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            MouseArea {
                id: addArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: console.log("Add connection — TODO")
            }
        }

        Item { Layout.fillHeight: true }

        SectionLabel { text: "Local" }

        Text {
            text: "Open terminal"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            color: localArea.containsMouse ? Theme.textPrimary : Theme.textSecondary

            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            MouseArea {
                id: localArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: console.log("Open local terminal — TODO")
            }
        }
    }

    // Right 1px border
    Rectangle {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
