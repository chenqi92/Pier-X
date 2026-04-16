import QtQuick
import QtQuick.Effects
import Pier

Rectangle {
    id: root

    default property alias content: contentItem.data
    property int padding: Theme.sp4
    property bool inset: false
    implicitWidth: contentItem.childrenRect.width + root.padding * 2
    implicitHeight: contentItem.childrenRect.height + root.padding * 2

    color: inset ? Theme.bgInset : Theme.bgSurface
    border.color: inset ? Theme.borderSubtle : Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusMd
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    layer.enabled: !Theme.dark && !inset
    layer.effect: MultiEffect {
        shadowEnabled: true
        shadowColor: "#000000"
        shadowOpacity: 0.06
        shadowBlur: 0.34
        shadowVerticalOffset: 4
    }

    Item {
        id: contentItem
        anchors.fill: parent
        anchors.margins: root.padding
    }
}
