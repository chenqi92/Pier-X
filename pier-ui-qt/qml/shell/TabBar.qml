import QtQuick
import QtQuick.Layouts
import Pier

// Horizontal tab bar — TerminalTab repeater + new tab button.
// `model` is a ListModel with at least { title } per row.
Rectangle {
    id: root

    property var model: null
    property int currentIndex: 0
    signal tabClicked(int index)
    signal tabClosed(int index)
    signal newTabClicked

    implicitHeight: 32
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        ListView {
            id: list
            Layout.fillHeight: true
            Layout.preferredWidth: contentWidth
            Layout.maximumWidth: parent.width - 40
            orientation: ListView.Horizontal
            model: root.model
            interactive: false
            clip: true

            delegate: TerminalTab {
                title: model.title
                active: index === root.currentIndex
                onClicked: root.tabClicked(index)
                onCloseRequested: root.tabClosed(index)
            }
        }

        IconButton {
            Layout.alignment: Qt.AlignVCenter
            Layout.leftMargin: Theme.sp1
            glyph: "+"
            tooltip: qsTr("New session")
            onClicked: root.newTabClicked()
        }

        Item { Layout.fillWidth: true }
    }

    // Bottom border
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
