import QtQuick
import QtQuick.Layouts
import QtQuick.Effects
import Pier

Item {
    id: root

    property string icon: "database"
    property string title: ""
    property string description: ""
    property bool compact: false

    implicitWidth: root.compact ? 180 : 220
    implicitHeight: root.compact ? 96 : 120

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(root.width, root.compact ? 220 : 248)
        spacing: root.compact ? Theme.sp1 : Theme.sp1_5

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            width: root.compact ? 20 : 24
            height: root.compact ? 20 : 24
            radius: Theme.radiusMd
            color: Theme.bgInset
            border.color: Theme.borderSubtle
            border.width: 1

            Image {
                anchors.centerIn: parent
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
                sourceSize: Qt.size(root.compact ? Theme.iconXs : Theme.iconSm,
                                    root.compact ? Theme.iconXs : Theme.iconSm)
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
            font.pixelSize: root.compact ? Theme.sizeSmall : Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: Theme.textPrimary
            wrapMode: Text.WordWrap
        }

        Text {
            Layout.fillWidth: true
            text: root.description
            horizontalAlignment: Text.AlignHCenter
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            color: Theme.textTertiary
            wrapMode: Text.WordWrap
        }
    }
}
