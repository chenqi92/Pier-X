import QtQuick
import Pier

// Card — surface container with subtle border.
// Spec: SKILL.md §9.4
Rectangle {
    id: root

    default property alias content: contentItem.data
    property int padding: Theme.sp4

    color: Theme.bgSurface
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusMd

    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    Item {
        id: contentItem
        anchors.fill: parent
        anchors.margins: root.padding
    }
}
