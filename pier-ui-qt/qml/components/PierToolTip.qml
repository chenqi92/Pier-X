import QtQuick
import QtQuick.Controls.Basic
import Pier

// Themed tooltip — wraps Qt Quick Controls ToolTip with the design system.
// Usage:
//   IconButton {
//       glyph: "+"
//       tooltip: qsTr("New session")
//   }
// (IconButton already mounts a PierToolTip internally when `tooltip` is set.)
ToolTip {
    id: root

    delay: 600
    timeout: 4000

    background: Rectangle {
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusSm

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }
    }

    contentItem: Text {
        text: root.text
        font.family: Theme.fontUi
        font.pixelSize: Theme.sizeCaption
        color: Theme.textPrimary
        leftPadding: Theme.sp1
        rightPadding: Theme.sp1
        topPadding: Theme.sp0_5
        bottomPadding: Theme.sp0_5

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
