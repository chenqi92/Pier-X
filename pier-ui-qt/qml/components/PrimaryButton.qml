import QtQuick
import Pier

Rectangle {
    id: root

    property string text: ""
    property bool compact: false
    property bool destructive: false
    property int minimumWidth: 88
    signal clicked

    readonly property int horizontalPadding: compact ? Theme.sp2 : Theme.sp3
    readonly property int buttonHeight: compact ? 26 : Theme.controlHeight

    implicitHeight: buttonHeight
    implicitWidth: Math.max(label.implicitWidth + horizontalPadding * 2 + Theme.sp1, minimumWidth)
    radius: Theme.radiusSm
    readonly property color toneColor: destructive ? Theme.statusError : Theme.accent
    readonly property color hoverColor: destructive ? Qt.lighter(Theme.statusError, 1.08) : Theme.accentHover

    color: !enabled ? toneColor
         : mouseArea.pressed ? Qt.darker(toneColor, 1.12)
         : mouseArea.containsMouse ? hoverColor
         : toneColor
    border.color: activeFocus ? Theme.borderFocus : Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.0)
    border.width: activeFocus ? 2 : 1
    opacity: enabled ? 1.0 : 0.45

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    focusPolicy: Qt.TabFocus

    Text {
        id: label
        anchors.fill: parent
        anchors.leftMargin: root.horizontalPadding
        anchors.rightMargin: root.horizontalPadding
        verticalAlignment: Text.AlignVCenter
        horizontalAlignment: Text.AlignHCenter
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: compact ? Theme.sizeCaption : Theme.sizeBody
        font.weight: Theme.weightMedium
        color: Theme.textInverse
        wrapMode: Text.NoWrap
        elide: Text.ElideRight
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
