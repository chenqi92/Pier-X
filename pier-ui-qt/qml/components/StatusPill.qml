import QtQuick
import Pier

// Status indicator pill — colored dot + label.
// Spec: SKILL.md §9.5
Rectangle {
    id: root

    property string text: ""
    property color statusColor: Theme.statusSuccess

    implicitHeight: 20
    implicitWidth: row.implicitWidth + Theme.sp3 * 2

    color: Theme.bgSurface
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusPill

    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: Theme.sp1_5

        Rectangle {
            width: 6
            height: 6
            radius: 3
            color: root.statusColor
            anchors.verticalCenter: parent.verticalCenter
        }

        Text {
            text: root.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textSecondary
            anchors.verticalCenter: parent.verticalCenter

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }
}
