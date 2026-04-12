import QtQuick
import QtQuick.Layouts
import Pier

Item {
    id: root

    property string title: ""
    property string subtitle: ""
    default property alias actions: actionRow.data

    implicitHeight: Math.max(32, textColumn.implicitHeight)

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
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightSemibold
                color: Theme.textPrimary
                elide: Text.ElideRight
            }

            Text {
                visible: root.subtitle.length > 0
                Layout.fillWidth: true
                text: root.subtitle
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeSmall
                color: Theme.textTertiary
                elide: Text.ElideRight
            }
        }

        RowLayout {
            id: actionRow
            spacing: Theme.sp1
        }
    }
}
