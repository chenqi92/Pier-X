import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property var options: []
    property int currentIndex: 0
    signal activated(int index)

    implicitHeight: Theme.controlHeight + Theme.sp0_5
    implicitWidth: segRow.implicitWidth + Theme.sp1
    color: Theme.bgInset
    border.color: Theme.borderDefault
    border.width: 1
    radius: Theme.radiusMd

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        id: segRow
        anchors.fill: parent
        anchors.margins: Theme.sp0_5
        spacing: Theme.sp0_5

        Repeater {
            model: root.options

            delegate: Rectangle {
                required property var modelData
                required property int index

                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: Theme.radiusSm
                color: index === root.currentIndex
                       ? Theme.bgSurface
                       : segArea.containsMouse ? Theme.bgHover : "transparent"
                border.color: index === root.currentIndex ? Theme.borderSubtle : "transparent"
                border.width: index === root.currentIndex ? 1 : 0

                Text {
                    anchors.centerIn: parent
                    text: modelData
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: index === root.currentIndex ? Theme.weightMedium : Theme.weightRegular
                    color: index === root.currentIndex ? Theme.textPrimary : Theme.textSecondary
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
