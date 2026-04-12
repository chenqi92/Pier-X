import QtQuick
import QtQuick.Effects
import Pier

Rectangle {
    id: root

    property string icon: ""
    property string glyph: ""
    property real iconRotation: 0
    property string tooltip: ""
    property bool active: false
    property bool compact: false
    property int iconSize: Theme.iconMd
    property alias hovered: mouseArea.containsMouse
    property alias pressed: mouseArea.pressed
    signal clicked

    readonly property int buttonSize: compact ? 26 : Theme.controlHeight

    implicitWidth: buttonSize
    implicitHeight: buttonSize
    radius: Theme.radiusSm
    color: mouseArea.pressed ? Theme.bgActive
         : active ? Theme.bgSelected
         : mouseArea.containsMouse ? Theme.bgHover
         : "transparent"
    border.color: active ? Theme.borderFocus
                 : mouseArea.containsMouse ? Theme.borderSubtle
                 : "transparent"
    border.width: (active || mouseArea.containsMouse) ? 1 : 0
    opacity: enabled ? 1.0 : 0.42

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    Image {
        id: iconImage
        anchors.centerIn: parent
        source: root.icon.length > 0
                ? "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
                : ""
        sourceSize: Qt.size(root.iconSize, root.iconSize)
        visible: root.icon.length > 0
        transform: Rotation {
            origin.x: iconImage.width / 2
            origin.y: iconImage.height / 2
            angle: root.iconRotation
        }
        layer.enabled: true
        layer.effect: MultiEffect {
            colorization: 1.0
            colorizationColor: root.active
                               ? Theme.accent
                               : mouseArea.containsMouse
                                 ? Theme.textPrimary
                                 : Theme.textSecondary
            Behavior on colorizationColor { ColorAnimation { duration: Theme.durFast } }
        }
    }

    Text {
        anchors.centerIn: parent
        text: root.glyph
        visible: root.icon.length === 0 && root.glyph.length > 0
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBodyLg
        color: root.active ? Theme.accent : Theme.textSecondary
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

    PierToolTip {
        text: root.tooltip
        visible: root.tooltip.length > 0 && root.hovered
    }
}
