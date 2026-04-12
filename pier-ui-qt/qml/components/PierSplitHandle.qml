import QtQuick
import QtQuick.Controls
import Pier

Rectangle {
    id: root

    // `vertical` describes the visible divider line. A horizontal SplitView
    // lays items side by side, so its divider line is vertical.
    property bool vertical: true

    implicitWidth: vertical ? 8 : 0
    implicitHeight: vertical ? 0 : 8
    color: SplitHandle.pressed ? Theme.splitHandleActive
         : SplitHandle.hovered ? Theme.splitHandleHover
         : Theme.splitHandleIdle

    Behavior on color { ColorAnimation { duration: Theme.durFast } }

    Rectangle {
        anchors.centerIn: parent
        width: vertical ? 1 : Math.max(parent.width - Theme.sp4, Theme.sp8)
        height: vertical ? Math.max(parent.height - Theme.sp4, Theme.sp8) : 1
        radius: 1
        color: SplitHandle.pressed ? Theme.splitHandleLineActive
             : SplitHandle.hovered ? Theme.splitHandleLine
             : "transparent"

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
    }
}
