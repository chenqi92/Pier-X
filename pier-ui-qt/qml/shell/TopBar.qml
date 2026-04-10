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

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp4
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

        Item { Layout.preferredWidth: Theme.sp3 }

        IconButton {
            glyph: "+"
            onClicked: console.log("New session — TODO")
        }
        IconButton {
            glyph: "⌕"
            onClicked: console.log("Search — TODO")
        }

        Item { Layout.fillWidth: true }

        IconButton {
            glyph: Theme.dark ? "☾" : "☀"
            onClicked: Theme.dark = !Theme.dark
        }
        IconButton {
            glyph: "⚙"
            onClicked: console.log("Settings — TODO")
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
