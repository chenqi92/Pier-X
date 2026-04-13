import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property string text: ""
    property string trailingText: ""
    property bool active: false
    property bool checkable: false
    property bool checked: false
    property bool destructive: false
    signal clicked

    implicitWidth: 224
    implicitHeight: Theme.controlHeight
    radius: Theme.radiusSm
    color: active ? Theme.bgSelected : menuArea.containsMouse ? Theme.bgHover : "transparent"
    opacity: enabled ? 1.0 : 0.48

    Behavior on color { ColorAnimation { duration: Theme.durFast } }
    Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        spacing: Theme.sp3

        Item {
            Layout.preferredWidth: root.checkable ? 16 : 0
            Layout.preferredHeight: 16
            visible: root.checkable

            Text {
                anchors.centerIn: parent
                text: "✓"
                visible: root.checked
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.accent
            }
        }

        Text {
            Layout.fillWidth: true
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            text: root.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: root.active ? Theme.weightMedium : Theme.weightRegular
            color: root.destructive ? Theme.statusError : (root.active ? Theme.textPrimary : Theme.textSecondary)
        }

        Text {
            text: root.trailingText
            visible: root.trailingText.length > 0
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary
        }
    }

    MouseArea {
        id: menuArea
        anchors.fill: parent
        hoverEnabled: true
        enabled: root.enabled
        cursorShape: root.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: root.clicked()
    }
}
