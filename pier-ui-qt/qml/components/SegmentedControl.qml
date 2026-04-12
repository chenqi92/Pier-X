import QtQuick
import QtQuick.Layouts
import Pier

// Small segmented control — used where the original Pier UI
// prefers direct mode switching over a dropdown.
Rectangle {
    id: root

    property var options: []
    property int currentIndex: 0
    signal activated(int index)

    implicitHeight: 32
    implicitWidth: segRow.implicitWidth + 6
    color: Theme.dark ? Theme.bgSurface : Theme.bgPanel
    border.color: Theme.borderDefault
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        id: segRow
        anchors.fill: parent
        anchors.margins: 3
        spacing: 3

        Repeater {
            model: root.options

            delegate: Rectangle {
                required property var modelData

                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: Theme.radiusSm - 1
                color: index === root.currentIndex
                       ? Theme.bgElevated
                       : segArea.containsMouse ? Theme.bgHover : "transparent"

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                Text {
                    anchors.centerIn: parent
                    text: modelData
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: index === root.currentIndex
                                 ? Theme.weightMedium
                                 : Theme.weightRegular
                    color: index === root.currentIndex
                           ? Theme.textPrimary
                           : Theme.textSecondary

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }
                }

                MouseArea {
                    id: segArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                    enabled: root.enabled
                    onClicked: {
                        if (root.currentIndex === index)
                            return
                        root.currentIndex = index
                        root.activated(index)
                    }
                }
            }
        }
    }
}
