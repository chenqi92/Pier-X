import QtQuick
import Pier

// Compact binary switch — closer to the native IDE/settings
// affordance than a text button that says "On" / "Off".
Rectangle {
    id: root

    property bool checked: false
    signal toggled(bool checked)

    implicitWidth: 38
    implicitHeight: 22
    radius: Theme.radiusPill
    color: root.checked ? Theme.accent : Theme.bgActive
    border.color: root.checked ? Theme.accent : Theme.borderDefault
    border.width: 1
    opacity: enabled ? 1.0 : 0.45

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    Rectangle {
        id: thumb
        width: 16
        height: 16
        radius: 8
        y: 3
        x: root.checked ? root.width - width - 3 : 3
        color: "#ffffff"
        border.color: Qt.rgba(0, 0, 0, 0.10)
        border.width: 1

        Behavior on x { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
    }

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        enabled: root.enabled
        onClicked: {
            root.checked = !root.checked
            root.toggled(root.checked)
        }
    }

    Keys.onSpacePressed: {
        root.checked = !root.checked
        root.toggled(root.checked)
    }
    Keys.onReturnPressed: {
        root.checked = !root.checked
        root.toggled(root.checked)
    }
}
