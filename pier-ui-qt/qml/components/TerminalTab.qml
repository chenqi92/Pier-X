import QtQuick
import QtQuick.Effects
import Pier

// A single tab in the TabBar — title + close button + active accent line.
Rectangle {
    id: root

    property string title: ""
    property bool active: false
    property bool menuOpen: false
    signal clicked
    signal closeRequested
    signal contextMenuRequested(real x, real y)

    implicitHeight: 32
    implicitWidth: row.implicitWidth + Theme.sp4 * 2

    color: active                  ? Theme.bgCanvas
         : root.menuOpen           ? Theme.bgActive
         : tabArea.containsMouse   ? Theme.bgHover
         : "transparent"

    Behavior on color { ColorAnimation { duration: Theme.durFast } }

    // Right border (subtle separator between tabs)
    Rectangle {
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: 1
        color: Theme.borderSubtle
    }

    // Active indicator (top accent line)
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 2
        color: Theme.accent
        visible: root.active

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }

    // Click area for the whole tab (sits behind the close button)
    MouseArea {
        id: tabArea
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        cursorShape: Qt.PointingHandCursor
        onClicked: (mouse) => {
            if (mouse.button === Qt.RightButton) {
                root.contextMenuRequested(mouse.x, mouse.y)
                return
            }
            root.clicked()
        }
    }

    Row {
        id: row
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp4
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp2

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: root.title
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: root.active ? Theme.weightMedium : Theme.weightRegular
            color: root.active ? Theme.textPrimary : Theme.textSecondary

            Behavior on color { ColorAnimation { duration: Theme.durFast } }
        }

        // Close button
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: Theme.radiusXs
            color: closeArea.containsMouse ? Theme.bgActive : "transparent"

            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            Image {
                anchors.centerIn: parent
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/x.svg"
                sourceSize: Qt.size(12, 12)
                layer.enabled: true
                layer.effect: MultiEffect {
                    colorization: 1.0
                    colorizationColor: closeArea.containsMouse ? Theme.textPrimary : Theme.textTertiary
                }
            }

            MouseArea {
                id: closeArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: root.closeRequested()
            }
        }
    }
}
