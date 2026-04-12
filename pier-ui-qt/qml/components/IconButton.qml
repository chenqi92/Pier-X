import QtQuick
import QtQuick.Effects
import Pier

// Toolbar icon button — square, hover-revealing background.
// Supports Lucide SVG icons via the `icon` property (primary) and
// legacy Unicode glyphs via `glyph` (deprecated, kept for transition).
Rectangle {
    id: root

    property string icon: ""       // Lucide SVG name, e.g. "plus"
    property string glyph: ""      // Legacy Unicode glyph (deprecated)
    property real iconRotation: 0
    property string tooltip: ""
    property bool active: false
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

    // ── SVG icon (primary) ──────────────────────────────
    Image {
        id: iconImage
        anchors.centerIn: parent
        source: root.icon.length > 0
                ? "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
                : ""
        sourceSize: Qt.size(16, 16)
        visible: root.icon.length > 0
        transform: Rotation {
            origin.x: iconImage.width / 2
            origin.y: iconImage.height / 2
            angle: root.iconRotation
        }
        // The SVG is rendered with the default stroke color (currentColor
        // in the file is black). We overlay a tint so the icon follows
        // the theme color. MultiEffect's colorization replaces the
        // source color completely at 1.0.
        layer.enabled: true
        layer.effect: MultiEffect {
            colorization: 1.0
            colorizationColor: root.active ? Theme.accent : Theme.textSecondary

            Behavior on colorizationColor { ColorAnimation { duration: Theme.durNormal } }
        }
    }

    // ── Legacy glyph fallback ───────────────────────────
    Text {
        anchors.centerIn: parent
        text: root.glyph
        visible: root.icon.length === 0 && root.glyph.length > 0
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
