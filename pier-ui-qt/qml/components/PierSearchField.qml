import QtQuick
import QtQuick.Effects
import Pier

Rectangle {
    id: root

    property alias text: input.text
    property alias readOnly: input.readOnly
    property alias input: input
    property string placeholder: ""
    property string icon: "search"
    property bool clearable: false
    property bool compact: false

    function forceActiveFocus() {
        input.forceActiveFocus()
    }

    implicitHeight: compact ? Theme.compactRowHeight : Theme.fieldHeight
    implicitWidth: 240
    color: Theme.bgSurface
    border.color: input.activeFocus ? Theme.borderFocus : fieldMouse.containsMouse ? Theme.borderDefault : Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    Image {
        id: leadingIcon
        anchors.left: parent.left
        anchors.leftMargin: compact ? Theme.sp2 : Theme.sp3
        anchors.verticalCenter: parent.verticalCenter
        source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
        sourceSize: Qt.size(compact ? 11 : Theme.iconSm,
                            compact ? 11 : Theme.iconSm)
        layer.enabled: true
        layer.effect: MultiEffect {
            colorization: 1.0
            colorizationColor: input.activeFocus ? Theme.textSecondary : Theme.textTertiary
        }
    }

    TextInput {
        id: input
        anchors.left: leadingIcon.right
        anchors.leftMargin: compact ? Theme.sp1_5 : Theme.sp2
        anchors.right: clearButton.visible ? clearButton.left : parent.right
        anchors.rightMargin: clearButton.visible ? Theme.sp1 : Theme.sp3
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        verticalAlignment: TextInput.AlignVCenter
        clip: true
        font.family: Theme.fontUi
        font.pixelSize: compact ? Theme.sizeCaption : Theme.sizeBody
        color: Theme.textPrimary
        selectionColor: Theme.accentMuted
        selectedTextColor: Theme.textPrimary

        Text {
            anchors.fill: parent
            verticalAlignment: Text.AlignVCenter
            text: root.placeholder
            font: input.font
            color: Theme.textTertiary
            visible: input.text.length === 0
        }
    }

    Rectangle {
        id: clearButton
        visible: root.clearable && input.text.length > 0
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp1
        anchors.verticalCenter: parent.verticalCenter
        width: (compact ? Theme.compactRowHeight : Theme.fieldHeight) - Theme.sp1
        height: (compact ? Theme.compactRowHeight : Theme.fieldHeight) - Theme.sp1
        radius: Theme.radiusSm
        color: clearArea.containsMouse ? Theme.bgHover : "transparent"

        Behavior on color { ColorAnimation { duration: Theme.durFast } }

        Image {
            anchors.centerIn: parent
            source: "qrc:/qt/qml/Pier/resources/icons/lucide/x.svg"
            sourceSize: Qt.size(Theme.iconXs, Theme.iconXs)
            layer.enabled: true
            layer.effect: MultiEffect {
                colorization: 1.0
                colorizationColor: clearArea.containsMouse ? Theme.textSecondary : Theme.textTertiary
            }
        }

        MouseArea {
            id: clearArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: input.text = ""
        }
    }

    MouseArea {
        id: fieldMouse
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
    }
}
