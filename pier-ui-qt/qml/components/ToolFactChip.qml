import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property string label: ""
    property string value: ""
    property bool monoValue: false
    property color valueColor: Theme.textPrimary

    visible: root.value.length > 0
    implicitHeight: 26
    implicitWidth: factRow.implicitWidth + Theme.sp2 * 2
    radius: Theme.radiusSm
    color: Theme.bgInset
    border.color: Theme.borderSubtle
    border.width: 1

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        id: factRow
        anchors.centerIn: parent
        spacing: Theme.sp1_5

        Text {
            visible: root.label.length > 0
            text: root.label
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            font.weight: Theme.weightMedium
            color: Theme.textTertiary
        }

        Text {
            text: root.value
            font.family: root.monoValue ? Theme.fontMono : Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: root.valueColor
            elide: Text.ElideRight
        }
    }
}
