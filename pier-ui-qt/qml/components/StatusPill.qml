import QtQuick
import Pier

// Status indicator pill — colored dot + label.
// Spec: SKILL.md §9.5
Rectangle {
    id: root

    property string text: ""
    property string tone: ""
    property color statusColor: {
        switch (root.tone) {
        case "neutral":
            return Theme.textTertiary
        case "info":
            return Theme.accent
        case "warning":
            return Theme.statusWarning
        case "error":
            return Theme.statusError
        default:
            return Theme.statusSuccess
        }
    }

    implicitHeight: 22
    implicitWidth: row.implicitWidth + Theme.sp3 * 2

    color: Qt.rgba(root.statusColor.r, root.statusColor.g, root.statusColor.b, Theme.dark ? 0.14 : 0.10)
    border.color: "transparent"
    border.width: 0
    radius: Theme.radiusPill

    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: Theme.sp1_5

        Rectangle {
            width: 5
            height: 5
            radius: 2.5
            color: root.statusColor
            anchors.verticalCenter: parent.verticalCenter
        }

        Text {
            text: root.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: root.statusColor
            anchors.verticalCenter: parent.verticalCenter

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }
}
