import QtQuick
import Pier

// Lightweight in-scene tooltip.
//
// Qt Quick Controls ToolTip/Popup can flash transient black windows on
// Windows during rapid hover changes, especially in dense tool rails.
// Rendering the tooltip directly in the scene keeps it stable and avoids
// spawning extra popup surfaces.
Item {
    id: root

    property string text: ""
    property int delay: 600
    property int timeout: 4000
    property bool shown: false

    z: 10_000
    width: bubble.implicitWidth
    height: bubble.implicitHeight
    anchors.horizontalCenter: parent ? parent.horizontalCenter : undefined
    anchors.bottom: parent ? parent.top : undefined
    anchors.bottomMargin: Theme.sp2

    onVisibleChanged: {
        if (visible && text.length > 0) {
            hideTimer.stop()
            if (delay > 0) {
                shown = false
                showTimer.restart()
            } else {
                shown = true
                if (timeout > 0)
                    hideTimer.restart()
            }
        } else {
            showTimer.stop()
            hideTimer.stop()
            shown = false
        }
    }

    onTextChanged: {
        if (text.length === 0) {
            showTimer.stop()
            hideTimer.stop()
            shown = false
        } else if (visible && !showTimer.running && !shown) {
            showTimer.restart()
        }
    }

    Timer {
        id: showTimer
        interval: root.delay
        onTriggered: {
            if (!root.visible || root.text.length === 0)
                return
            root.shown = true
            if (root.timeout > 0)
                hideTimer.restart()
        }
    }

    Timer {
        id: hideTimer
        interval: root.timeout
        onTriggered: root.shown = false
    }

    Rectangle {
        id: bubble

        visible: root.shown && root.text.length > 0
        opacity: visible ? 1 : 0
        implicitWidth: Math.max(0, label.implicitWidth + Theme.sp2 * 2)
        implicitHeight: Math.max(0, label.implicitHeight + Theme.sp1)
        anchors.centerIn: parent
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusSm

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        Text {
            id: label
            anchors.centerIn: parent
            text: root.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textPrimary
        }
    }
}
