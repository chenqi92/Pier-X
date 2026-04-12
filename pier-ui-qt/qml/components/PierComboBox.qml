import QtQuick
import QtQuick.Effects
import QtQuick.Window
import Pier

Rectangle {
    id: root

    property var options: []
    property int currentIndex: 0
    property string placeholder: ""
    signal activated(int index)

    implicitHeight: Theme.fieldHeight
    implicitWidth: 220
    color: Theme.bgSurface
    border.color: popup.visible ? Theme.borderFocus : comboMouse.containsMouse ? Theme.borderStrong : Theme.borderDefault
    border.width: 1
    radius: Theme.radiusMd

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    Text {
        id: label
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp3
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
    }

    Image {
        id: chevron
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp3
        anchors.verticalCenter: parent.verticalCenter
        source: "qrc:/qt/qml/Pier/resources/icons/lucide/chevron-down.svg"
        sourceSize: Qt.size(Theme.iconSm, Theme.iconSm)
        layer.enabled: true
        layer.effect: MultiEffect {
            colorization: 1.0
            colorizationColor: popup.visible ? Theme.textPrimary : Theme.textTertiary
        }
    }

    MouseArea {
        id: comboMouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            popup.visible = !popup.visible
            if (popup.visible)
                root.forceActiveFocus()
        }
    }

    Keys.onEscapePressed: popup.visible = false

    MouseArea {
        id: dismissOverlay
        parent: root.Window.contentItem || root
        anchors.fill: parent
        visible: popup.visible
        z: 99
        onClicked: popup.visible = false
    }

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
        radius: Theme.radiusLg

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: Theme.dark ? 0.34 : 0.16
            shadowBlur: 1.0
            shadowVerticalOffset: 8
        }

        Column {
            id: optionsCol
            anchors.fill: parent
            anchors.margins: Theme.sp1
            spacing: Theme.sp0_5

            Repeater {
                model: root.options
                delegate: Rectangle {
                    width: optionsCol.width
                    implicitHeight: Theme.controlHeight
                    radius: Theme.radiusSm
                    color: optionArea.containsMouse
                         ? Theme.bgHover
                         : (index === root.currentIndex ? Theme.bgSelected : "transparent")

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp3
                        anchors.rightMargin: Theme.sp3
                        verticalAlignment: Text.AlignVCenter
                        text: modelData
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: index === root.currentIndex ? Theme.weightMedium : Theme.weightRegular
                        color: Theme.textPrimary
                    }

                    MouseArea {
                        id: optionArea
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
