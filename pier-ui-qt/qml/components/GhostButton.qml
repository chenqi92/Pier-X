import QtQuick
import Pier

Rectangle {
    id: root

    property string text: ""
    property bool compact: false
    property int minimumWidth: 72
    signal clicked

    readonly property int horizontalPadding: compact ? Theme.sp2 : Theme.sp3
    readonly property int buttonHeight: compact ? 26 : Theme.controlHeight

    implicitHeight: buttonHeight
    implicitWidth: Math.max(label.implicitWidth + horizontalPadding * 2, minimumWidth)
    radius: Theme.radiusSm
    color: mouseArea.pressed ? Theme.bgActive
         : mouseArea.containsMouse ? Theme.bgHover
         : "transparent"
    border.color: activeFocus ? Theme.borderFocus
                 : mouseArea.containsMouse ? Theme.borderDefault : "transparent"
    border.width: (activeFocus || mouseArea.containsMouse) ? 1 : 0
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
        color: mouseArea.pressed ? Theme.textPrimary
             : mouseArea.containsMouse ? Theme.textPrimary
             : Theme.textSecondary
        wrapMode: Text.NoWrap
        elide: Text.ElideRight
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
