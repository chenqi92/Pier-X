import QtQuick
import QtQuick.Layouts
import Pier

// Top toolbar — sits below the native window title bar.
// Houses app brand, primary actions, and global controls (theme toggle).
Rectangle {
    id: root

    implicitHeight: 38
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    // On macOS with frameless chrome, leave space for the
    // native traffic light buttons. The inset is zero on all
    // other platforms.
    readonly property int trafficLightInset: Qt.platform.os === "osx" ? 78 : 0

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp4 + root.trafficLightInset
        anchors.rightMargin: Theme.sp3
        spacing: Theme.sp2

        // Brand
        Text {
            text: "Pier-X"
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: Theme.weightSemibold
            color: Theme.textPrimary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        IconButton {
            icon: "plus"
            tooltip: qsTr("New connection")
            onClicked: root.newSessionRequested()
        }

        IconButton {
            icon: "command"
            tooltip: qsTr("Command palette  (Ctrl+K)")
            onClicked: root.commandPaletteRequested()
        }

        Item { Layout.fillWidth: true }

        IconButton {
            icon: Theme.dark ? "moon" : "sun"
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

    signal newSessionRequested
    signal commandPaletteRequested
    signal settingsRequested

    // Allow window dragging from the TopBar area on macOS
    // (frameless chrome removes the native title bar drag).
    TapHandler {
        onTapped: (eventPoint, button) => {
            // no-op: TapHandler consumes clicks that should go to
            // children only when nothing else matches, so toolbar
            // buttons keep working. This is here to prevent the
            // DragHandler below from swallowing single-click events.
        }
    }
    DragHandler {
        target: null
        grabPermissions: PointerHandler.CanTakeOverFromAnything
        onActiveChanged: {
            if (active) window.startSystemMove()
        }
    }

    // Bottom 1px border
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
