import QtQuick
import QtQuick.Controls
import Pier

Slider {
    id: root

    implicitWidth: 168
    implicitHeight: 24

    background: Rectangle {
        x: root.leftPadding
        y: root.topPadding + (root.availableHeight - height) / 2
        width: root.availableWidth
        height: 4
        radius: 2
        color: Theme.bgInset
        border.color: Theme.borderSubtle
        border.width: 1

        Rectangle {
            width: root.visualPosition * parent.width
            height: parent.height
            radius: parent.radius
            color: Theme.accent
        }
    }

    handle: Rectangle {
        x: root.leftPadding + root.visualPosition * (root.availableWidth - width)
        y: root.topPadding + (root.availableHeight - height) / 2
        width: 14
        height: 14
        radius: 7
        color: root.pressed ? Theme.accentHover : Theme.bgSurface
        border.color: root.pressed || root.hovered ? Theme.borderFocus : Theme.borderDefault
        border.width: 1

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    }
}
