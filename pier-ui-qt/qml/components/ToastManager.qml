import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// ToastManager — shows transient notification banners at the top-right
// of the window. The toast auto-dismisses after `duration` ms.
//
// Usage from Main.qml:
//   toastManager.show("Connection saved", "success")
//   toastManager.show("Network error", "error")
//   toastManager.show("Copied to clipboard", "info")
Item {
    id: root
    anchors.fill: parent
    z: 9999

    // Show a toast notification.
    //   message: text to display
    //   type:    "success" | "error" | "warning" | "info" (default: "info")
    //   duration: auto-dismiss time in ms (default: 3000)
    function show(message, type, duration) {
        toastModel.append({
            message: message,
            type: type || "info",
            duration: duration || 3000
        })
    }

    ListModel { id: toastModel }

    // Toast stack — top-right corner, newest at top
    Column {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: Theme.sp5
        anchors.rightMargin: Theme.sp4
        spacing: Theme.sp2

        Repeater {
            model: toastModel
            delegate: Rectangle {
                id: toast
                required property int index
                required property string message
                required property string type
                required property int duration

                width: 320
                height: toastRow.implicitHeight + Theme.sp3 * 2
                radius: Theme.radiusMd
                color: Theme.bgElevated
                border.color: Theme.borderDefault
                border.width: 1

                // Entry animation
                opacity: 0
                scale: 0.96
                Component.onCompleted: {
                    opacity = 1.0
                    scale = 1.0
                }
                Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
                Behavior on scale   { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
                transformOrigin: Item.TopRight

                layer.enabled: true
                layer.effect: MultiEffect {
                    shadowEnabled: true
                    shadowColor: "#000000"
                    shadowOpacity: 0.25
                    shadowBlur: 0.6
                    shadowVerticalOffset: 4
                }

                // Status accent bar on the left edge
                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 3
                    radius: Theme.radiusMd
                    color: {
                        switch (toast.type) {
                            case "success": return Theme.statusSuccess
                            case "error":   return Theme.statusError
                            case "warning": return Theme.statusWarning
                            default:        return Theme.statusInfo
                        }
                    }
                }

                RowLayout {
                    id: toastRow
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3 + 3  // past accent bar
                    anchors.rightMargin: Theme.sp2
                    anchors.topMargin: Theme.sp3
                    anchors.bottomMargin: Theme.sp3
                    spacing: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: toast.message
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightMedium
                        color: Theme.textPrimary
                        wrapMode: Text.Wrap
                        maximumLineCount: 3
                        elide: Text.ElideRight

                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    }

                    // Close button
                    IconButton {
                        icon: "x"
                        implicitWidth: 20
                        implicitHeight: 20
                        onClicked: {
                            dismissTimer.stop()
                            toastModel.remove(toast.index)
                        }
                    }
                }

                // Auto-dismiss timer
                Timer {
                    id: dismissTimer
                    interval: toast.duration
                    running: true
                    onTriggered: {
                        // Fade out, then remove
                        toast.opacity = 0
                        toast.scale = 0.96
                        removeTimer.start()
                    }
                }

                // Delay removal until fade-out animation completes
                Timer {
                    id: removeTimer
                    interval: Theme.durNormal + 50
                    onTriggered: {
                        if (toast.index >= 0 && toast.index < toastModel.count)
                            toastModel.remove(toast.index)
                    }
                }

                // Pause auto-dismiss on hover
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    propagateComposedEvents: true
                    // Block clicks from reaching elements below
                    onClicked: (mouse) => mouse.accepted = true
                    onEntered: dismissTimer.stop()
                    onExited: dismissTimer.restart()
                }
            }
        }
    }
}
