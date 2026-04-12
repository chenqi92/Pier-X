import QtQuick
import Pier

// Primary CTA — accent-filled button.
// Spec: SKILL.md §9.1
Rectangle {
    id: root

    property string text: ""
    signal clicked

    implicitHeight: 28
    implicitWidth: label.implicitWidth + Theme.sp4 * 2

    color: !enabled            ? Theme.accent
         : mouseArea.pressed   ? Qt.darker(Theme.accent, 1.15)
         : mouseArea.containsMouse ? Theme.accentHover
         : Theme.accent
    opacity: enabled ? 1.0 : 0.5
    radius: Theme.radiusSm

    // Keyboard focus ring
    border.color: activeFocus ? Theme.borderFocus : "transparent"
    border.width: activeFocus ? 2 : 0

    Behavior on color        { ColorAnimation  { duration: Theme.durFast } }
    Behavior on opacity      { NumberAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation  { duration: Theme.durFast } }

    focusPolicy: Qt.TabFocus

    Text {
        id: label
        anchors.centerIn: parent
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: Theme.weightMedium
        color: Theme.textInverse
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
