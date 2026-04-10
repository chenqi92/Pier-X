import QtQuick
import QtQuick.Effects
import Pier

// Themed combo box — opens an inline popup of options.
// Lightweight by design (no Qt Quick Controls dependency in markup).
Rectangle {
    id: root

    property var options: []          // array of strings
    property int currentIndex: 0
    property string placeholder: ""
    signal activated(int index)

    implicitHeight: 28
    implicitWidth: 200
    color: Theme.bgSurface
    border.color: popup.visible ? Theme.borderFocus : Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    Text {
        id: label
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp2
        anchors.right: chevron.left
        anchors.verticalCenter: parent.verticalCenter
        elide: Text.ElideRight
        text: root.currentIndex >= 0 && root.currentIndex < root.options.length
              ? root.options[root.currentIndex]
              : root.placeholder
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: (root.currentIndex >= 0 && root.currentIndex < root.options.length)
               ? Theme.textPrimary
               : Theme.textTertiary

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }

    Text {
        id: chevron
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp2
        anchors.verticalCenter: parent.verticalCenter
        text: "▾"
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: Theme.textTertiary

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }

    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: popup.visible = !popup.visible
    }

    // Inline popup
    Rectangle {
        id: popup
        visible: false
        z: 100
        anchors.top: parent.bottom
        anchors.topMargin: Theme.sp1
        anchors.left: parent.left
        width: parent.width
        height: optionsCol.implicitHeight + Theme.sp1 * 2
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusMd

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.32
            shadowBlur: 1.0
            shadowVerticalOffset: 6
        }

        Column {
            id: optionsCol
            anchors.fill: parent
            anchors.margins: Theme.sp1

            Repeater {
                model: root.options
                delegate: Rectangle {
                    width: optionsCol.width
                    implicitHeight: 26
                    color: optArea.containsMouse
                         ? Theme.bgHover
                         : (index === root.currentIndex ? Theme.accentMuted : "transparent")
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        verticalAlignment: Text.AlignVCenter
                        text: modelData
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textPrimary
                    }

                    MouseArea {
                        id: optArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.currentIndex = index
                            popup.visible = false
                            root.activated(index)
                        }
                    }
                }
            }
        }
    }
}
