import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import Pier

Item {
    id: root

    property bool open: false
    property string title: ""
    property string subtitle: ""
    property int dialogWidth: 720
    property int dialogHeight: 640
    property int edgePadding: 72
    property int bodyPadding: 0
    property bool closeOnBackdrop: true
    default property alias body: bodySlot.data
    property alias footer: footerSlot.data
    property alias headerActions: headerActions.data
    signal requestClose

    visible: root.open
    anchors.fill: parent
    z: 9500

    Keys.onEscapePressed: root.requestClose()

    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open && root.closeOnBackdrop
            onClicked: root.requestClose()
        }
    }

    Rectangle {
        id: card
        anchors.centerIn: parent
        width: Math.min(root.dialogWidth, parent.width - root.edgePadding)
        height: Math.min(root.dialogHeight, parent.height - root.edgePadding)
        scale: root.open ? 1.0 : 0.96
        opacity: root.open ? 1.0 : 0.0
        transformOrigin: Item.Center
        color: Theme.bgElevated
        radius: Theme.radiusLg

        Behavior on scale { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: Theme.dark ? 0.46 : 0.20
            shadowBlur: 1.0
            shadowVerticalOffset: 18
        }

        MouseArea {
            anchors.fill: parent
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // ── Header ──
            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.dialogHeaderHeight
                clip: true

                // Header background — fills top area with rounded top corners
                Rectangle {
                    anchors.fill: parent
                    color: Theme.bgChrome
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                    // Only round the top corners by extending below the clip
                    radius: Theme.radiusLg
                }
                // Mask the bottom rounded corners of the header bg
                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: Theme.radiusLg
                    color: Theme.bgChrome
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp5
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp3

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            text: root.title
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeH3
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                        }

                        Text {
                            visible: root.subtitle.length > 0
                            text: root.subtitle
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    RowLayout {
                        id: headerActions
                        spacing: Theme.sp1
                    }

                    IconButton {
                        icon: "x"
                        tooltip: qsTr("Close")
                        onClicked: root.requestClose()
                    }
                }

                // Header bottom divider
                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.borderSubtle
                }
            }

            // ── Body ──
            Item {
                id: bodyContainer
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true

                Item {
                    id: bodySlot
                    anchors.fill: parent
                    anchors.margins: root.bodyPadding
                }
            }

            // ── Footer ──
            Item {
                id: footerContainer
                visible: footerSlot.children.length > 0
                Layout.fillWidth: true
                implicitHeight: footerSlot.childrenRect.height + Theme.sp4 * 2
                clip: true

                // Footer background — fills bottom area with rounded bottom corners
                Rectangle {
                    anchors.fill: parent
                    color: Theme.bgChrome
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    radius: Theme.radiusLg
                }
                // Mask the top rounded corners of the footer bg
                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: Theme.radiusLg
                    color: Theme.bgChrome
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                // Footer top divider
                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: 1
                    color: Theme.borderSubtle
                }

                Item {
                    id: footerSlot
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp5
                    anchors.rightMargin: Theme.sp5
                    anchors.topMargin: Theme.sp3
                    anchors.bottomMargin: Theme.sp3
                }
            }
        }
    }
}
