import QtQuick
import QtQuick.Layouts
import Pier

// Bottom status bar — short status text + version label.
Rectangle {
    id: root

    implicitHeight: 24
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        spacing: Theme.sp3

        Text {
            text: qsTr("Ready")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Item { Layout.fillWidth: true }

        Text {
            text: qsTr("Qt") + " " + qVersion
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            // Qt version isn't exposed as a QML built-in; injected from C++ would be cleaner,
            // for now we display the literal LTS we target.
            property string qVersion: "6.8 LTS"

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            text: "v" + Qt.application.version
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }

    // Top 1px border
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
