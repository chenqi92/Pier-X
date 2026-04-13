import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

Item {
    id: root

    property string title: ""
    property string subtitle: ""
    property string icon: ""
    property color iconColor: root.prominent ? Theme.accent : Theme.textTertiary
    property bool prominent: false
    property bool compact: false
    default property alias actions: actionRow.data

    implicitHeight: Math.max(root.prominent
                             ? (root.compact ? 34 : 40)
                             : (root.compact ? 28 : 34),
                             textColumn.implicitHeight)

    RowLayout {
        anchors.fill: parent
        spacing: Theme.sp2

        ColumnLayout {
            id: textColumn
            Layout.fillWidth: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1_5

                Item {
                    visible: root.icon.length > 0
                    Layout.preferredWidth: root.compact ? 14 : 16
                    Layout.preferredHeight: root.compact ? 14 : 16

                    Image {
                        id: iconImage
                        anchors.centerIn: parent
                        visible: root.icon.length > 0
                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + root.icon + ".svg"
                        sourceSize: Qt.size(parent.width, parent.height)
                        smooth: true
                        mipmap: true
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorization: 1.0
                            colorizationColor: root.iconColor
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    text: root.title
                    font.family: Theme.fontUi
                    font.pixelSize: root.prominent
                                    ? (root.compact ? Theme.sizeBody : Theme.sizeBodyLg)
                                    : (root.compact ? Theme.sizeCaption : Theme.sizeBody)
                    font.weight: Theme.weightSemibold
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }
            }

            Text {
                visible: root.subtitle.length > 0
                Layout.fillWidth: true
                text: root.subtitle
                font.family: Theme.fontUi
                font.pixelSize: root.prominent
                                ? Theme.sizeSmall
                                : (root.compact ? Theme.sizeSmall : Theme.sizeCaption)
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
