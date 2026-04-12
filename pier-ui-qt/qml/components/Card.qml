import QtQuick
import QtQuick.Effects
import Pier

Rectangle {
    id: root

    default property alias content: contentItem.data
    property int padding: Theme.sp4

    color: Theme.bgSurface
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusLg

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    layer.enabled: !Theme.dark
    layer.effect: MultiEffect {
        shadowEnabled: true
        shadowColor: "#000000"
        shadowOpacity: 0.05
        shadowBlur: 0.4
        shadowVerticalOffset: 2
    }

    Item {
        id: contentItem
        anchors.fill: parent
        anchors.margins: root.padding
    }
}
