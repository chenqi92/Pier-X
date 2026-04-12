import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

Rectangle {
    id: root

    implicitHeight: Theme.topBarHeight
    color: Theme.bgChrome
    property string contextTitle: qsTr("Workspace")

    readonly property int trafficLightInset: Qt.platform.os === "osx" ? 78 : 0
    readonly property string shortcutLabel: Qt.platform.os === "osx" ? "Cmd+K" : "Ctrl+K"

    signal newSessionRequested
    signal commandPaletteRequested
    signal settingsRequested

    MouseArea {
        anchors.left: parent.left
        anchors.right: commandPaletteButton.left
        anchors.leftMargin: root.trafficLightInset
        anchors.rightMargin: Theme.sp3
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.ArrowCursor
        onPressed: (mouse) => {
            if (mouse.button === Qt.LeftButton)
                window.startSystemMove()
        }
    }

    MouseArea {
        anchors.left: commandPaletteButton.right
        anchors.right: rightControls.left
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp2
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        acceptedButtons: Qt.LeftButton
        cursorShape: Qt.ArrowCursor
        onPressed: (mouse) => {
            if (mouse.button === Qt.LeftButton)
                window.startSystemMove()
        }
    }

    RowLayout {
        anchors.left: parent.left
        anchors.leftMargin: Theme.sp4 + root.trafficLightInset
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp2

        Rectangle {
            width: 16
            height: 16
            radius: 5
            color: Theme.accentMuted
            border.color: Theme.borderDefault
            border.width: 1

            Rectangle {
                anchors.centerIn: parent
                width: 6
                height: 6
                radius: 3
                color: Theme.accent
            }
        }

        Text {
            text: "Pier-X"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: Theme.weightSemibold
            color: Theme.textPrimary
        }

        Rectangle {
            width: 1
            height: 14
            color: Theme.borderSubtle
        }

        Text {
            text: root.contextTitle
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textTertiary
            elide: Text.ElideMiddle
            Layout.maximumWidth: 260
        }
    }

    Rectangle {
        id: commandPaletteButton
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        width: Math.min(360,
                        Math.max(250,
                                 paletteLabel.implicitWidth
                                 + shortcutText.implicitWidth
                                 + Theme.sp10))
        height: Theme.controlHeight
        radius: Theme.radiusPill
        color: paletteMouse.pressed ? Theme.bgActive
             : paletteMouse.containsMouse ? Theme.bgHover
             : Theme.bgInset
        border.color: paletteMouse.containsMouse ? Theme.borderStrong : Theme.borderDefault
        border.width: 1

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp3
            anchors.rightMargin: Theme.sp3
            spacing: Theme.sp2

            Image {
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/command.svg"
                sourceSize: Qt.size(Theme.iconSm, Theme.iconSm)
                layer.enabled: true
                layer.effect: MultiEffect {
                    colorization: 1.0
                    colorizationColor: Theme.textTertiary
                }
            }

            Text {
                id: paletteLabel
                Layout.fillWidth: true
                text: qsTr("Command Palette")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textSecondary
                elide: Text.ElideRight
            }

            Text {
                id: shortcutText
                text: root.shortcutLabel
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeSmall
                color: Theme.textTertiary
            }
        }

        MouseArea {
            id: paletteMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.commandPaletteRequested()
        }
    }

    RowLayout {
        id: rightControls
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp3
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp1

        IconButton {
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newSessionRequested()
        }

        IconButton {
            icon: Theme.dark ? "sun" : "moon"
            tooltip: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
            onClicked: {
                Theme.followSystem = false
                Theme.dark = !Theme.dark
            }
        }

        IconButton {
            icon: "settings"
            tooltip: qsTr("Settings")
            onClicked: root.settingsRequested()
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: Theme.borderSubtle
    }
}
