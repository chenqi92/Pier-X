import QtQuick
import QtQuick.Controls
import Pier

TextArea {
    id: root

    property bool mono: false
    property bool frameVisible: true
    property bool inset: false
    property int horizontalPadding: Theme.sp3
    property int verticalPadding: Theme.sp3
    property int cornerRadius: Theme.radiusSm
    property color fillColor: inset ? Theme.bgInset : Theme.bgSurface

    implicitWidth: 240
    implicitHeight: Math.max(Theme.fieldHeight * 2,
                             contentHeight + topPadding + bottomPadding)
    hoverEnabled: true
    color: Theme.textPrimary
    placeholderTextColor: Theme.textTertiary
    selectionColor: Theme.accentMuted
    selectedTextColor: Theme.textPrimary
    font.family: mono ? Theme.fontMono : Theme.fontUi
    font.pixelSize: mono ? Theme.sizeBody : Theme.sizeBody
    leftPadding: horizontalPadding
    rightPadding: horizontalPadding
    topPadding: verticalPadding
    bottomPadding: verticalPadding

    background: Rectangle {
        visible: root.frameVisible
        color: root.frameVisible ? root.fillColor : "transparent"
        border.color: root.activeFocus ? Theme.borderFocus
                                       : root.hovered ? Theme.borderDefault : Theme.borderSubtle
        border.width: root.frameVisible ? 1 : 0
        radius: root.cornerRadius

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    }

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
}
