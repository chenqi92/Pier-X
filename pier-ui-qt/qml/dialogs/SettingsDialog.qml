import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Modal settings dialog — section nav on the left, content on the right.
// Pure UI for now; persistence will land alongside pier-core.
Item {
    id: root

    property bool open: false
    property var connectionsModel: null

    signal closed

    visible: open
    z: 9400
    anchors.fill: parent

    function show() {
        // Reset animation state
        dialog.scale = 0.96
        dialog.opacity = 0
        open = true
        // Trigger entry animation
        dialog.scale = 1.0
        dialog.opacity = 1.0
        sectionList.currentIndex = 0
    }

    function hide() {
        open = false
        closed()
    }

    Keys.onEscapePressed: hide()

    // Backdrop
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0
        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open
            onClicked: root.hide()
        }
    }

    // Dialog card
    Rectangle {
        id: dialog
        anchors.centerIn: parent
        width: 720
        height: 520

        // Entry animation — scale-up + fade-in
        scale: 0.96
        opacity: 0
        Behavior on scale   { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
        transformOrigin: Item.Center

        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.5
            shadowBlur: 1.0
            shadowVerticalOffset: 16
        }

        // Block clicks on the dialog from reaching the backdrop.
        MouseArea {
            anchors.fill: parent
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // Title bar
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 44
                color: Theme.bgPanel

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp4
                    anchors.rightMargin: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Settings")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeH3
                        font.weight: Theme.weightMedium
                        color: Theme.textPrimary
                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    }

                    IconButton {
                        icon: "x"
                        tooltip: qsTr("Close")
                        onClicked: root.hide()
                    }
                }

                Rectangle {
                    anchors.bottom: parent.bottom
                    anchors.left: parent.left
                    anchors.right: parent.right
                    height: 1
                    color: Theme.borderSubtle
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // Section nav
                Rectangle {
                    Layout.preferredWidth: 180
                    Layout.fillHeight: true
                    color: Theme.bgPanel

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                    ListView {
                        id: sectionList
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp0_5
                        interactive: false
                        currentIndex: 0

                        model: ListModel {
                            ListElement { title: qsTr("General") }
                            ListElement { title: qsTr("Appearance") }
                            ListElement { title: qsTr("Terminal") }
                            ListElement { title: qsTr("Connections") }
                        }

                        delegate: Rectangle {
                            width: ListView.view.width
                            implicitHeight: 28
                            color: ListView.isCurrentItem
                                 ? Theme.accentMuted
                                 : navArea.containsMouse ? Theme.bgHover : "transparent"
                            radius: Theme.radiusSm

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            Text {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                verticalAlignment: Text.AlignVCenter
                                text: model.title
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                font.weight: ListView.isCurrentItem
                                           ? Theme.weightMedium
                                           : Theme.weightRegular
                                color: ListView.isCurrentItem
                                     ? Theme.textPrimary
                                     : Theme.textSecondary

                                Behavior on color { ColorAnimation { duration: Theme.durFast } }
                            }

                            MouseArea {
                                id: navArea
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: sectionList.currentIndex = index
                            }
                        }
                    }

                    Rectangle {
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 1
                        color: Theme.borderSubtle
                    }
                }

                // Content area
                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: sectionList.currentIndex

                    // ─── General ─────────────────────────
                    ScrollView {
                        clip: true
                        ColumnLayout {
                            width: parent.width
                            spacing: Theme.sp4

                            Item { Layout.preferredHeight: Theme.sp4 }

                            SectionLabel { text: qsTr("Theme"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Follow system theme")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    GhostButton {
                                        text: Theme.followSystem ? qsTr("On") : qsTr("Off")
                                        onClicked: Theme.followSystem = !Theme.followSystem
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Color scheme")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    PierComboBox {
                                        id: schemeCombo
                                        Layout.preferredWidth: 160
                                        options: [qsTr("Dark"), qsTr("Light")]
                                        currentIndex: Theme.dark ? 0 : 1
                                        onActivated: (i) => {
                                            Theme.followSystem = false
                                            Theme.dark = (i === 0)
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ─── Appearance ──────────────────────
                    ScrollView {
                        clip: true
                        ColumnLayout {
                            width: parent.width
                            spacing: Theme.sp4

                            Item { Layout.preferredHeight: Theme.sp4 }

                            SectionLabel { text: qsTr("Typography"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("UI font")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    Text {
                                        text: Theme.fontUi
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textTertiary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Mono font")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    Text {
                                        text: Theme.fontMono
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textTertiary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                }
                            }

                            Separator { Layout.fillWidth: true; Layout.leftMargin: Theme.sp4; Layout.rightMargin: Theme.sp4 }

                            SectionLabel { text: qsTr("UI font size"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        text: qsTr("Size")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                    }
                                    Item { Layout.fillWidth: true }
                                    Text {
                                        text: Theme.sizeBody + " px"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textTertiary
                                    }
                                }

                                // Preview
                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: previewText.implicitHeight + Theme.sp3 * 2
                                    color: Theme.bgSurface
                                    border.color: Theme.borderSubtle
                                    border.width: 1
                                    radius: Theme.radiusSm

                                    Text {
                                        id: previewText
                                        anchors.fill: parent
                                        anchors.margins: Theme.sp3
                                        text: "The quick brown fox jumps over the lazy dog."
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        wrapMode: Text.Wrap
                                    }
                                }

                                // Mono preview
                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: monoPreview.implicitHeight + Theme.sp3 * 2
                                    color: Theme.bgSurface
                                    border.color: Theme.borderSubtle
                                    border.width: 1
                                    radius: Theme.radiusSm

                                    Text {
                                        id: monoPreview
                                        anchors.fill: parent
                                        anchors.margins: Theme.sp3
                                        text: "$ ssh root@prod-01 'tail -f /var/log/nginx/access.log'"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }
                        }
                    }

                    // ─── Terminal ────────────────────────
                    ScrollView {
                        clip: true
                        ColumnLayout {
                            width: parent.width
                            spacing: Theme.sp4

                            Item { Layout.preferredHeight: Theme.sp4 }

                            SectionLabel { text: qsTr("Cursor"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Cursor style")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    PierComboBox {
                                        Layout.preferredWidth: 120
                                        options: [qsTr("Block"), qsTr("Beam"), qsTr("Underline")]
                                        currentIndex: 0
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Cursor blink")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    GhostButton {
                                        text: qsTr("On")
                                    }
                                }
                            }

                            Separator { Layout.fillWidth: true; Layout.leftMargin: Theme.sp4; Layout.rightMargin: Theme.sp4 }

                            SectionLabel { text: qsTr("Scrollback"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Buffer lines")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    // Editable field for scrollback buffer size
                                    PierTextField {
                                        Layout.preferredWidth: 90
                                        text: "10000"
                                    }
                                }
                            }

                            Separator { Layout.fillWidth: true; Layout.leftMargin: Theme.sp4; Layout.rightMargin: Theme.sp4 }

                            SectionLabel { text: qsTr("Bell"); Layout.leftMargin: Theme.sp4 }

                            ColumnLayout {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Visual bell")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    GhostButton {
                                        text: qsTr("On")
                                    }
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("Audio bell")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    GhostButton {
                                        text: qsTr("Off")
                                    }
                                }
                            }
                        }
                    }

                    // ─── Connections ─────────────────────
                    ScrollView {
                        clip: true
                        ColumnLayout {
                            width: parent.width
                            spacing: Theme.sp4

                            Item { Layout.preferredHeight: Theme.sp4 }

                            SectionLabel { text: qsTr("Saved connections"); Layout.leftMargin: Theme.sp4 }

                            // Empty state
                            Text {
                                visible: !root.connectionsModel || root.connectionsModel.count === 0
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                text: qsTr("No connections saved yet.")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textTertiary
                                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                            }

                            ListView {
                                Layout.fillWidth: true
                                Layout.leftMargin: Theme.sp4
                                Layout.rightMargin: Theme.sp4
                                Layout.preferredHeight: contentHeight
                                interactive: false
                                model: root.connectionsModel
                                visible: root.connectionsModel && root.connectionsModel.count > 0
                                spacing: Theme.sp1

                                delegate: Rectangle {
                                    width: ListView.view.width
                                    implicitHeight: 44
                                    color: Theme.bgSurface
                                    border.color: Theme.borderSubtle
                                    border.width: 1
                                    radius: Theme.radiusSm

                                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                                    ColumnLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp3
                                        anchors.rightMargin: Theme.sp3
                                        spacing: 0

                                        Text {
                                            text: model.name
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            font.weight: Theme.weightMedium
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                            Layout.fillWidth: true
                                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                        }
                                        Text {
                                            text: (model.username || "") + "@" + model.host + ":" + model.port
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textTertiary
                                            elide: Text.ElideRight
                                            Layout.fillWidth: true
                                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
