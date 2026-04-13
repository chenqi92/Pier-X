import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property color accentColor: Theme.accent
    property bool compact: false
    property int padding: root.compact ? Theme.sp1_5 : Theme.sp2
    default property alias content: contentColumn.data

    implicitWidth: contentColumn.implicitWidth + root.padding * 2
    implicitHeight: contentColumn.implicitHeight + root.padding * 2 + accentLine.height
    radius: root.compact ? Theme.radiusSm : Theme.radiusMd
    color: Qt.tint(
               Theme.bgPanel,
               Qt.rgba(root.accentColor.r,
                        root.accentColor.g,
                        root.accentColor.b,
                        Theme.dark ? (root.compact ? 0.07 : 0.10)
                                   : (root.compact ? 0.04 : 0.05)))
    border.color: Qt.rgba(root.accentColor.r,
                          root.accentColor.g,
                          root.accentColor.b,
                          Theme.dark ? (root.compact ? 0.16 : 0.22)
                                     : (root.compact ? 0.10 : 0.14))
    border.width: 1
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Rectangle {
        id: accentLine
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: root.compact ? 1 : 2
        color: Qt.rgba(root.accentColor.r,
                       root.accentColor.g,
                       root.accentColor.b,
                       Theme.dark ? (root.compact ? 0.78 : 0.92)
                                  : (root.compact ? 0.62 : 0.72))
    }

    ColumnLayout {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: root.padding
        anchors.topMargin: root.padding + (root.compact ? 1 : Theme.sp0_5)
        spacing: root.compact ? Theme.sp1_5 : Theme.sp2
    }
}
