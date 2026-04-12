import QtQuick
import Pier

Rectangle {
    id: root

    property alias text: input.text
    property alias readOnly: input.readOnly
    property string placeholder: ""
    property bool password: false

    implicitHeight: Theme.fieldHeight
    implicitWidth: 220
    color: Theme.bgSurface
    border.color: input.activeFocus ? Theme.borderFocus : fieldMouse.containsMouse ? Theme.borderDefault : Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        verticalAlignment: TextInput.AlignVCenter
        clip: true
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: Theme.textPrimary
        selectionColor: Theme.accentMuted
        selectedTextColor: Theme.textPrimary
        echoMode: root.password ? TextInput.Password : TextInput.Normal
        passwordCharacter: "\u2022"

        Text {
            anchors.fill: parent
            verticalAlignment: Text.AlignVCenter
            text: root.placeholder
            font: input.font
            color: Theme.textTertiary
            visible: input.text.length === 0
        }
    }

    MouseArea {
        id: fieldMouse
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
    }
}
