import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import Pier

Popup {
    id: root

    property int cornerRadius: Theme.radiusLg
    property int panelPadding: Theme.sp1
    property int itemSpacing: Theme.sp0_5
    property bool shadowEnabled: true
    default property alias body: contentColumn.data

    modal: false
    focus: true
    padding: panelPadding
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    background: Rectangle {
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: root.cornerRadius

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

        layer.enabled: root.shadowEnabled
        layer.effect: MultiEffect {
            shadowEnabled: root.shadowEnabled
            shadowColor: "#000000"
            shadowOpacity: Theme.dark ? 0.34 : 0.14
            shadowBlur: 1.0
            shadowVerticalOffset: 10
        }
    }

    contentItem: Column {
        id: contentColumn
        spacing: root.itemSpacing
    }
}
