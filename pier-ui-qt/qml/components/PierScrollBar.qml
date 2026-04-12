import QtQuick
import QtQuick.Controls
import Pier

ScrollBar {
    id: root

    property bool compact: true

    implicitWidth: orientation === Qt.Vertical ? (compact ? 10 : 12) : 0
    implicitHeight: orientation === Qt.Horizontal ? (compact ? 10 : 12) : 0
    padding: compact ? Theme.sp0_5 : Theme.sp1

    contentItem: Rectangle {
        implicitWidth: root.orientation === Qt.Vertical ? 6 : root.availableWidth
        implicitHeight: root.orientation === Qt.Horizontal ? 6 : root.availableHeight
        radius: Theme.radiusPill
        color: !root.enabled ? Theme.textDisabled
             : (root.pressed || root.hovered) ? Theme.textSecondary
             : Theme.textTertiary
        opacity: root.policy === ScrollBar.AlwaysOff ? 0.0 : (root.active || root.hovered || root.pressed ? 0.86 : 0.54)

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
    }

    background: Rectangle {
        radius: Theme.radiusPill
        color: root.active || root.hovered ? Theme.bgHover : "transparent"
    }
}
