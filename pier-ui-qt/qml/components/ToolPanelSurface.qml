import QtQuick
import Pier

Rectangle {
    id: root

    default property alias content: contentItem.data
    property int padding: Theme.sp3
    property bool inset: false

    implicitWidth: contentItem.childrenRect.width + root.padding * 2
    implicitHeight: contentItem.childrenRect.height + root.padding * 2
    color: inset ? Theme.bgInset : Theme.bgPanel
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Item {
        id: contentItem
        anchors.fill: parent
        anchors.margins: root.padding
    }
}
