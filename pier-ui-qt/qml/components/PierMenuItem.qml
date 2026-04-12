import QtQuick
import Pier

Rectangle {
    id: root

    property string text: ""
    property bool active: false
    property bool destructive: false
    signal clicked

    implicitWidth: 224
    implicitHeight: Theme.controlHeight
    radius: Theme.radiusSm
    color: active ? Theme.bgSelected : menuArea.containsMouse ? Theme.bgHover : "transparent"
    opacity: enabled ? 1.0 : 0.48

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    Text {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        font.weight: root.active ? Theme.weightMedium : Theme.weightRegular
        color: root.destructive ? Theme.statusError : (root.active ? Theme.textPrimary : Theme.textSecondary)
    }

    MouseArea {
        id: menuArea
        anchors.fill: parent
        hoverEnabled: true
        enabled: root.enabled
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: root.clicked()
    }
}
