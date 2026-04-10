import QtQuick
import Pier

// Toolbar icon button — square, hover-revealing background.
// Glyph is a placeholder for SVG icons (Lucide / Phosphor) coming later.
Rectangle {
    id: root

    property string glyph: ""
    property string tooltip: ""
    property bool active: false
    property bool enabled: true
    property alias hovered: mouseArea.containsMouse
    property alias pressed: mouseArea.pressed
    signal clicked

    implicitHeight: 28
    implicitWidth: 28

    color: mouseArea.pressed       ? Theme.bgActive
         : mouseArea.containsMouse ? Theme.bgHover
         : active                  ? Theme.bgSelected
         : "transparent"
    radius: Theme.radiusSm
    opacity: enabled ? 1.0 : 0.5

    Behavior on color   { ColorAnimation  { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    Text {
        anchors.centerIn: parent
        text: root.glyph
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBodyLg
        color: root.active ? Theme.accent : Theme.textSecondary

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
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
