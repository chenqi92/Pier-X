import QtQuick
import QtQuick.Layouts
import Pier

Item {
    id: root

    property string title: ""
    property string subtitle: ""
    property bool prominent: false
    default property alias actions: actionRow.data

    implicitHeight: Math.max(root.prominent ? 40 : 34, textColumn.implicitHeight)

    RowLayout {
        anchors.fill: parent
        spacing: Theme.sp2

        ColumnLayout {
            id: textColumn
            Layout.fillWidth: true
            spacing: 0

            Text {
                Layout.fillWidth: true
                text: root.title
                font.family: Theme.fontUi
                font.pixelSize: root.prominent ? Theme.sizeBodyLg : Theme.sizeBody
                font.weight: Theme.weightSemibold
                color: Theme.textPrimary
                elide: Text.ElideRight
            }

            Text {
                visible: root.subtitle.length > 0
                Layout.fillWidth: true
                text: root.subtitle
                font.family: Theme.fontUi
                font.pixelSize: root.prominent ? Theme.sizeSmall : Theme.sizeCaption
                color: root.prominent ? Theme.textSecondary : Theme.textTertiary
                elide: Text.ElideRight
            }
        }

        RowLayout {
            id: actionRow
            spacing: Theme.sp1
        }
    }
}
