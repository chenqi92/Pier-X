import QtQuick
import Pier

// Text input — semi-transparent surface, accent focus border.
// Spec: SKILL.md §9.3
Rectangle {
    id: root

    property alias text: input.text
    property alias readOnly: input.readOnly
    property string placeholder: ""
    // When true, echo as bullets instead of the raw characters.
    // Used by the connection dialog's password field. The
    // TextInput.PasswordEchoOnEdit mode would leak the first
    // character momentarily — we use Password which is stricter.
    property bool password: false

    implicitHeight: 28
    implicitWidth: 200

    color: Theme.bgSurface
    border.color: input.activeFocus ? Theme.borderFocus : Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    TextInput {
        id: input
        anchors.fill: parent
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2
        verticalAlignment: TextInput.AlignVCenter
        clip: true
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: Theme.textPrimary
        selectionColor: Theme.accentMuted
        selectedTextColor: Theme.textPrimary
        echoMode: root.password ? TextInput.Password : TextInput.Normal
        passwordCharacter: "\u2022"

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }

        Text {
            anchors.fill: parent
            verticalAlignment: Text.AlignVCenter
            text: root.placeholder
            font: input.font
            color: Theme.textTertiary
            visible: input.text.length === 0 && !input.activeFocus

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }
}
