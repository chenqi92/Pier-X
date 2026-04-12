import QtQuick
import QtQuick.Layouts
import QtQuick.Effects
import Pier

Item {
    id: root

    property string icon: "database"
    property string title: ""
    property string description: ""

    implicitWidth: 240
    implicitHeight: 148

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(root.width, 280)
        spacing: Theme.sp2

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            width: 28
            height: 28
            radius: Theme.radiusMd
            color: Theme.bgInset
            border.color: Theme.borderSubtle
            border.width: 1

            Image {
                anchors.centerIn: parent
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
                sourceSize: Qt.size(Theme.iconSm, Theme.iconSm)
                layer.enabled: true
                layer.effect: MultiEffect {
                    colorization: 1.0
                    colorizationColor: Theme.textTertiary
                }
            }
        }

        Text {
            Layout.fillWidth: true
            text: root.title
            horizontalAlignment: Text.AlignHCenter
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            font.weight: Theme.weightMedium
            color: Theme.textPrimary
            wrapMode: Text.WordWrap
        }

        Text {
            Layout.fillWidth: true
            text: root.description
            horizontalAlignment: Text.AlignHCenter
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary
            wrapMode: Text.WordWrap
        }
    }
}
