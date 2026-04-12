import QtQuick
import Pier

Rectangle {
    id: root

    property string text: ""
    signal clicked

    implicitHeight: Theme.controlHeight
    implicitWidth: label.implicitWidth + Theme.sp3 * 2
    radius: Theme.radiusMd
    color: mouseArea.pressed ? Theme.bgActive
         : mouseArea.containsMouse ? Theme.bgHover
         : "transparent"
    border.color: activeFocus ? Theme.borderFocus : Theme.borderDefault
    border.width: 1
    opacity: enabled ? 1.0 : 0.45

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    focusPolicy: Qt.TabFocus

    Text {
        id: label
        anchors.centerIn: parent
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: Theme.weightMedium
        color: mouseArea.containsMouse ? Theme.textPrimary : Theme.textSecondary
        Behavior on color { ColorAnimation { duration: Theme.durFast } }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        enabled: root.enabled
        onClicked: root.clicked()
    }

    Keys.onReturnPressed: root.clicked()
    Keys.onEnterPressed: root.clicked()
    Keys.onSpacePressed: root.clicked()
}
