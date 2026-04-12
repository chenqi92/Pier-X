import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Window
import Pier

Rectangle {
    id: root

    property var options: []
    property int currentIndex: 0
    property string placeholder: ""
    signal activated(int index)

    readonly property bool hasSelection: root.currentIndex >= 0 && root.currentIndex < root.options.length
    readonly property Item windowContent: root.Window.window ? root.Window.window.contentItem : null

    function popupLabelAt(index) {
        if (index < 0 || index >= root.options.length)
            return ""
        const value = root.options[index]
        return typeof value === "string" ? value : String(value)
    }

    function openPopup() {
        if (!root.windowContent)
            return
        const pos = root.mapToItem(root.windowContent, 0, root.height + Theme.sp1)
        popup.x = Math.round(pos.x)
        popup.y = Math.round(pos.y)
        popup.width = Math.max(root.width, 160)
        popup.open()
    }

    implicitHeight: Theme.fieldHeight
    implicitWidth: 220
    color: Theme.bgSurface
    border.color: popup.visible
                 ? Theme.borderFocus
                 : comboMouse.containsMouse ? Theme.borderDefault : Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    Text {
        id: label
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp3
        anchors.right: chevron.left
        anchors.rightMargin: Theme.sp2
        anchors.verticalCenter: parent.verticalCenter
        elide: Text.ElideRight
        text: root.hasSelection
              ? root.popupLabelAt(root.currentIndex)
              : root.placeholder
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeBody
        color: root.hasSelection ? Theme.textPrimary : Theme.textTertiary
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
            if (popup.visible)
                popup.close()
            else
                root.openPopup()
        }
    }

    Keys.onEscapePressed: popup.close()

    Popup {
        id: popup
        parent: root.windowContent
        modal: false
        focus: true
        padding: Theme.sp1
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        width: root.width
        height: Math.min(listView.contentHeight + Theme.sp2, 280)

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg
            layer.enabled: true
            layer.effect: MultiEffect {
                shadowEnabled: true
                shadowColor: "#000000"
                shadowOpacity: Theme.dark ? 0.34 : 0.16
                shadowBlur: 1.0
                shadowVerticalOffset: 8
            }
        }

        onOpened: listView.positionViewAtIndex(Math.max(0, root.currentIndex), ListView.Contain)

        contentItem: ListView {
            id: listView
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: root.options
            currentIndex: root.currentIndex
            implicitHeight: contentHeight
            spacing: Theme.sp0_5

            delegate: Rectangle {
                required property int index
                required property var modelData

                width: listView.width
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
                    text: typeof modelData === "string" ? modelData : String(modelData)
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: index === root.currentIndex ? Theme.weightMedium : Theme.weightRegular
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                MouseArea {
                    id: optionArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        root.currentIndex = index
                        popup.close()
                        root.activated(index)
                    }
                }
            }

            ScrollBar.vertical: PierScrollBar {
                active: hovered || pressed
                visible: listView.contentHeight > listView.height
            }
        }
    }
}
