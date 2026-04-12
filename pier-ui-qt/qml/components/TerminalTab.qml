import QtQuick
import QtQuick.Effects
import Pier

Rectangle {
    id: root

    property string title: ""
    property string kind: ""
    property int colorTag: -1
    property bool active: false
    property bool menuOpen: false
    signal clicked
    signal closeRequested
    signal contextMenuRequested(real x, real y)

    readonly property var colorPalette: [
        "#e05555", "#f08d49", "#d9b44a", "#5fb865",
        "#3574f0", "#9b6df2", "#dc6ea8", "#33a6a6"
    ]
    readonly property string iconName: {
        if (root.kind === "sftp")
            return "folder"
        if (root.kind === "markdown")
            return "file-text"
        if (root.kind === "ssh")
            return "server"
        return "terminal"
    }

    implicitHeight: Theme.tabHeight
    implicitWidth: Math.max(108, row.implicitWidth + Theme.sp3 * 2)
    radius: Theme.radiusMd
    color: active ? Theme.accentSubtle
         : root.menuOpen ? Theme.bgActive
         : tabArea.containsMouse ? Theme.bgHover
         : "transparent"
    border.color: active ? Theme.borderFocus : "transparent"
    border.width: active ? 1 : 0

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 2
        color: root.active ? Theme.accent : "transparent"
    }

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
        anchors.leftMargin: Theme.sp3
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp2
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp2

        Rectangle {
            visible: root.colorTag >= 0 && root.colorTag < root.colorPalette.length
            anchors.verticalCenter: parent.verticalCenter
            width: 3
            height: 18
            radius: 1.5
            color: root.colorPalette[root.colorTag]
        }

        Image {
            anchors.verticalCenter: parent.verticalCenter
            source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.iconName + ".svg"
            sourceSize: Qt.size(12, 12)
            layer.enabled: true
            layer.effect: MultiEffect {
                colorization: 1.0
                colorizationColor: root.kind === "ssh"
                                   ? Theme.statusSuccess
                                   : (root.active ? Theme.accent : Theme.textTertiary)
            }
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(220, implicitWidth)
            text: root.title
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            font.weight: root.active ? Theme.weightMedium : Theme.weightRegular
            color: root.active ? Theme.textPrimary : Theme.textSecondary
            elide: Text.ElideRight
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 16
            height: 16
            radius: Theme.radiusSm
            visible: root.active || closeArea.containsMouse || tabArea.containsMouse
            color: closeArea.containsMouse ? Theme.bgActive : "transparent"
            opacity: root.active || tabArea.containsMouse ? 1.0 : 0.0

            Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
            Behavior on color { ColorAnimation { duration: Theme.durFast } }

            Image {
                anchors.centerIn: parent
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/x.svg"
                sourceSize: Qt.size(Theme.iconXs, Theme.iconXs)
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
