import QtQuick
import Pier

// Secondary button — transparent fill with subtle border.
// Spec: SKILL.md §9.2
Rectangle {
    id: root

    property string text: ""
    signal clicked

    implicitHeight: 28
    implicitWidth: label.implicitWidth + Theme.sp4 * 2

    color: mouseArea.pressed       ? Theme.bgActive
         : mouseArea.containsMouse ? Theme.bgHover
         : "transparent"
    border.color: activeFocus ? Theme.borderFocus : Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm
    opacity: enabled ? 1.0 : 0.5

    Behavior on color        { ColorAnimation  { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation  { duration: Theme.durNormal } }
    Behavior on opacity      { NumberAnimation { duration: Theme.durFast } }

    focusPolicy: Qt.TabFocus

    Text {
        id: label
        anchors.centerIn: parent
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: Theme.weightMedium
        color: Theme.textPrimary

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
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
