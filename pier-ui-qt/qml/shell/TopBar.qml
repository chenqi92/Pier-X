import QtQuick
import QtQuick.Layouts
import QtQuick.Window
import Pier
import "../components"

Rectangle {
    id: root

    implicitHeight: Theme.topBarHeight
    color: Theme.bgChrome
    property string contextTitle: qsTr("Workspace")
    readonly property var appWindow: root.Window.window

    // Keep the brand aligned with macOS traffic lights without leaving
    // an oversized dead zone before the Pier-X mark.
    readonly property int trafficLightReservedWidth: Qt.platform.os === "osx" ? 60 : 0
    readonly property int leadingContentMargin: trafficLightReservedWidth > 0
                                                ? trafficLightReservedWidth + Theme.sp2
                                                : Theme.sp3
    signal newSessionRequested
    signal settingsRequested

    function toggleWindowZoom() {
        if (!appWindow)
            return

        if (appWindow.visibility === Window.Maximized)
            appWindow.showNormal()
        else
            appWindow.showMaximized()
    }

    function handleTitleBarDoubleClick() {
        if (!appWindow || appWindow.visibility === Window.FullScreen)
            return

        const action = PierWindowChrome.titleBarDoubleClickAction()
        if (action === PierWindowChrome.MinimizeAction) {
            appWindow.showMinimized()
            return
        }
        if (action === PierWindowChrome.NoAction)
            return

        toggleWindowZoom()
    }

    function beginSystemMove(localX, localY) {
        if (!appWindow || appWindow.visibility === Window.FullScreen)
            return

        if (appWindow.visibility === Window.Maximized) {
            const globalPoint = root.mapToGlobal(Qt.point(localX, localY))
            const horizontalRatio = Math.max(0.12, Math.min(0.88, localX / Math.max(1, root.width)))
            const restoredWidth = Math.max(1, appWindow.windowedWidth || appWindow.width)

            appWindow.showNormal()

            const restoredX = globalPoint.x - restoredWidth * horizontalRatio
            const restoredY = globalPoint.y - Math.min(localY, Theme.topBarHeight / 2)

            appWindow.x = Math.round(restoredX)
            appWindow.y = Math.max(0, Math.round(restoredY))
        }

        appWindow.startSystemMove()
    }

    Item {
        id: dragRegion
        anchors.left: parent.left
        anchors.right: rightControls.left
        anchors.leftMargin: root.leadingContentMargin
        anchors.rightMargin: Theme.sp2
        anchors.top: parent.top
        anchors.bottom: parent.bottom

        DragHandler {
            id: moveHandler
            target: null
            grabPermissions: PointerHandler.CanTakeOverFromAnything

            onActiveChanged: {
                if (active)
                    root.beginSystemMove(centroid.position.x, centroid.position.y)
            }
        }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            hoverEnabled: true
            cursorShape: Qt.ArrowCursor
            onClicked: (mouse) => {
                if (mouse.button !== Qt.RightButton || !root.appWindow)
                    return

                const globalPoint = dragRegion.mapToGlobal(Qt.point(mouse.x, mouse.y))
                PierWindowChrome.showSystemMenu(root.appWindow, globalPoint.x, globalPoint.y)
            }
            onDoubleClicked: {
                mouse.accepted = true
                root.handleTitleBarDoubleClick()
            }
        }
    }

    RowLayout {
        anchors.left: parent.left
        anchors.leftMargin: root.leadingContentMargin
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp2

        Item {
            Layout.preferredWidth: 16
            Layout.preferredHeight: 16
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
            font.pixelSize: Theme.sizeSmall
            font.weight: Theme.weightMedium
            color: Theme.textTertiary
            elide: Text.ElideMiddle
            Layout.maximumWidth: 260
        }
    }

    RowLayout {
        id: rightControls
        anchors.right: parent.right
        anchors.rightMargin: Theme.sp3
        anchors.verticalCenter: parent.verticalCenter
        spacing: Theme.sp1

        IconButton {
            compact: true
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newSessionRequested()
        }

        IconButton {
            compact: true
            icon: Theme.dark ? "sun" : "moon"
            tooltip: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
            onClicked: {
                Theme.followSystem = false
                Theme.dark = !Theme.dark
            }
        }

        IconButton {
            compact: true
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
