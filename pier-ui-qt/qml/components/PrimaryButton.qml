import QtQuick
import Pier

Rectangle {
    id: root

    property string text: ""
    signal clicked

    implicitHeight: Theme.controlHeight
    implicitWidth: label.implicitWidth + Theme.sp3 * 2 + Theme.sp1
    radius: Theme.radiusMd
    color: !enabled ? Theme.accent
         : mouseArea.pressed ? Qt.darker(Theme.accent, 1.12)
         : mouseArea.containsMouse ? Theme.accentHover
         : Theme.accent
    border.color: activeFocus ? Theme.borderFocus : Qt.rgba(1, 1, 1, Theme.dark ? 0.12 : 0.0)
    border.width: activeFocus ? 2 : 1
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
