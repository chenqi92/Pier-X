import QtQuick
import QtQuick.Layouts
import Pier

Rectangle {
    id: root

    property string title: ""
    property string valueText: "—"
    property string subtitle: ""
    property string footerText: ""
    property real progress: -1
    property color accentColor: Theme.accent

    implicitHeight: 104
    color: Theme.bgPanel
    border.color: Theme.borderSubtle
    border.width: 1
    radius: Theme.radiusSm
    clip: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp1_5

        Text {
            Layout.fillWidth: true
            text: root.title
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textSecondary
            elide: Text.ElideRight
        }

        Item { Layout.fillHeight: true }

        Text {
            Layout.fillWidth: true
            text: root.valueText
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeH3
            font.weight: Theme.weightMedium
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

        Rectangle {
            visible: root.progress >= 0
            Layout.fillWidth: true
            implicitHeight: 6
            radius: 3
            color: Theme.bgInset

            Rectangle {
                width: Math.max(6, parent.width * Math.max(0, Math.min(100, root.progress)) / 100)
                height: parent.height
                radius: parent.radius
                color: root.accentColor
            }
        }

        Text {
            visible: root.footerText.length > 0
            Layout.fillWidth: true
            text: root.footerText
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textSecondary
            elide: Text.ElideRight
        }
    }
}
