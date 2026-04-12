import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property color accentColor: Theme.accent
    property int padding: Theme.sp2
    default property alias content: contentColumn.data

    implicitWidth: contentColumn.implicitWidth + root.padding * 2
    implicitHeight: contentColumn.implicitHeight + root.padding * 2 + accentLine.height
    radius: Theme.radiusMd
    color: Qt.tint(
               Theme.bgPanel,
               Qt.rgba(root.accentColor.r,
                        root.accentColor.g,
                        root.accentColor.b,
                        Theme.dark ? 0.10 : 0.05))
    border.color: Qt.rgba(root.accentColor.r,
                          root.accentColor.g,
                          root.accentColor.b,
                          Theme.dark ? 0.22 : 0.14)
    border.width: 1
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Rectangle {
        id: accentLine
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 2
        color: Qt.rgba(root.accentColor.r,
                       root.accentColor.g,
                       root.accentColor.b,
                       Theme.dark ? 0.92 : 0.72)
    }

    ColumnLayout {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: root.padding
        anchors.topMargin: root.padding + Theme.sp0_5
        spacing: Theme.sp2
    }
}
