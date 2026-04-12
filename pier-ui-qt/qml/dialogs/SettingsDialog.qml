import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Modal settings dialog — denser and more structured, closer to
// the original Pier settings flow: stable nav, readable rows,
// proper switches, and full-width saved-connection cards.
Item {
    id: root

    property bool open: false
    property var connectionsModel: null
    readonly property int navWidth: 208
    readonly property int pagePadding: 28
    readonly property int rowLabelWidth: 224

    signal closed

    visible: open
    z: 9400
    anchors.fill: parent

    function show() {
        dialog.scale = 0.96
        dialog.opacity = 0
        open = true
        dialog.scale = 1.0
        dialog.opacity = 1.0
        sectionList.currentIndex = 0
    }

    function hide() {
        open = false
        closed()
    }

    Keys.onEscapePressed: hide()

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

    Rectangle {
        id: dialog
        anchors.centerIn: parent
        width: Math.min(980, parent.width - 72)
        height: Math.min(700, parent.height - 72)
        scale: 0.96
        opacity: 0
        transformOrigin: Item.Center
        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on scale { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Theme.easingType } }
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.46
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

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.dialogHeaderHeight
                color: Theme.bgChrome

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp5
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp3

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            text: qsTr("Settings")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeH3
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                        }

                        Text {
                            text: qsTr("Adjust appearance, terminal behavior, and saved connections.")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                        }
                    }

                    IconButton {
                        icon: "x"
                        tooltip: qsTr("Close")
                        onClicked: root.hide()
                    }
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.borderSubtle
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                Rectangle {
                    Layout.preferredWidth: root.navWidth
                    Layout.fillHeight: true
                    color: Theme.bgChrome

                    ListView {
                        id: sectionList
                        anchors.fill: parent
                        anchors.margins: Theme.sp3
                        spacing: Theme.sp1
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
                            implicitHeight: 32
                            radius: Theme.radiusSm
                            color: ListView.isCurrentItem
                                   ? Theme.accentMuted
                                   : navArea.containsMouse ? Theme.bgHover : "transparent"

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            Text {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp3
                                anchors.rightMargin: Theme.sp2
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
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        anchors.right: parent.right
                        width: 1
                        color: Theme.borderSubtle
                    }
                }

                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: sectionList.currentIndex

                    ScrollView {
                        id: generalScroll
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            width: Math.max(0, generalScroll.availableWidth - root.pagePadding * 2)
                            x: root.pagePadding
                            y: root.pagePadding
                            spacing: Theme.sp5

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Theme") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Follow system theme")
                                    description: qsTr("Automatically mirror the operating system appearance.")

                                    ToggleSwitch {
                                        checked: Theme.followSystem
                                        onToggled: (checked) => Theme.followSystem = checked
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Color scheme")
                                    description: qsTr("Manual override when system sync is turned off.")

                                    SegmentedControl {
                                        implicitWidth: 168
                                        options: [qsTr("Dark"), qsTr("Light")]
                                        currentIndex: Theme.dark ? 0 : 1
                                        enabled: !Theme.followSystem
                                        onActivated: (i) => {
                                            Theme.followSystem = false
                                            Theme.dark = (i === 0)
                                        }
                                    }
                                }
                            }

                            Item { implicitHeight: root.pagePadding }
                        }
                    }

                    ScrollView {
                        id: appearanceScroll
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            width: Math.max(0, appearanceScroll.availableWidth - root.pagePadding * 2)
                            x: root.pagePadding
                            y: root.pagePadding
                            spacing: Theme.sp5

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Typography") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("UI font")
                                    description: qsTr("Primary interface font used for labels, buttons, and navigation.")

                                    Text {
                                        text: Theme.fontUi
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textSecondary
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Mono font")
                                    description: qsTr("Used for terminal content, paths, ports, and code-like data.")

                                    Text {
                                        text: Theme.fontMono
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textSecondary
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Preview") }

                                Card {
                                    Layout.fillWidth: true
                                    padding: Theme.sp4

                                    ColumnLayout {
                                        anchors.fill: parent
                                        spacing: Theme.sp2

                                        Text {
                                            text: qsTr("The quick brown fox jumps over the lazy dog.")
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textPrimary
                                        }

                                        Text {
                                            text: qsTr("Buttons, tabs, and list rows should stay compact while preserving hierarchy.")
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textSecondary
                                            wrapMode: Text.WordWrap
                                        }
                                    }
                                }

                                Card {
                                    Layout.fillWidth: true
                                    padding: Theme.sp4

                                    ColumnLayout {
                                        anchors.fill: parent
                                        spacing: Theme.sp2

                                        Text {
                                            text: "$ ssh root@prod-01 'tail -f /var/log/nginx/access.log'"
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textPrimary
                                            wrapMode: Text.WrapAnywhere
                                        }

                                        Text {
                                            text: qsTr("Machine-readable values stay monospaced so hosts, commands, and ports scan immediately.")
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textSecondary
                                            wrapMode: Text.WordWrap
                                        }
                                    }
                                }
                            }

                            Item { implicitHeight: root.pagePadding }
                        }
                    }

                    ScrollView {
                        id: terminalScroll
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            width: Math.max(0, terminalScroll.availableWidth - root.pagePadding * 2)
                            x: root.pagePadding
                            y: root.pagePadding
                            spacing: Theme.sp5

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Cursor") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Cursor style")
                                    description: qsTr("The visual shape used in the terminal.")

                                    SegmentedControl {
                                        implicitWidth: 228
                                        options: [qsTr("Block"), qsTr("Beam"), qsTr("Underline")]
                                        currentIndex: 0
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Cursor blink")
                                    description: qsTr("Animate the cursor when the terminal is focused.")

                                    ToggleSwitch { checked: true }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Scrollback") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Buffer lines")
                                    description: qsTr("Number of lines to keep in terminal history.")

                                    PierTextField {
                                        implicitWidth: 108
                                        text: "10000"
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Bell") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Visual bell")
                                    description: qsTr("Flash the terminal instead of playing a sound.")

                                    ToggleSwitch { checked: true }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Audio bell")
                                    description: qsTr("Play the terminal bell sound when supported.")

                                    ToggleSwitch { checked: false }
                                }
                            }

                            Item { implicitHeight: root.pagePadding }
                        }
                    }

                    ScrollView {
                        id: connectionsScroll
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            width: Math.max(0, connectionsScroll.availableWidth - root.pagePadding * 2)
                            x: root.pagePadding
                            y: root.pagePadding
                            spacing: Theme.sp4

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                SectionLabel {
                                    text: qsTr("Saved connections")
                                    Layout.fillWidth: true
                                }

                                Rectangle {
                                    visible: root.connectionsModel && root.connectionsModel.count > 0
                                    implicitHeight: 22
                                    implicitWidth: countText.implicitWidth + Theme.sp2 * 2
                                    radius: Theme.radiusPill
                                    color: Theme.accentSubtle
                                    border.color: Theme.accentMuted
                                    border.width: 1

                                    Text {
                                        id: countText
                                        anchors.centerIn: parent
                                        text: qsTr("%1 saved").arg(root.connectionsModel ? root.connectionsModel.count : 0)
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        font.weight: Theme.weightMedium
                                        color: Theme.accent
                                    }
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: qsTr("Profiles saved here are reused by the sidebar, SFTP browser, and remote service panels.")
                                wrapMode: Text.WordWrap
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textSecondary
                            }

                            Card {
                                Layout.fillWidth: true
                                visible: !root.connectionsModel || root.connectionsModel.count === 0
                                padding: Theme.sp4

                                ColumnLayout {
                                    anchors.fill: parent
                                    spacing: Theme.sp1

                                    Text {
                                        text: qsTr("No connections saved yet.")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                    }

                                    Text {
                                        text: qsTr("Use the New SSH connection dialog to create your first reusable host profile.")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textSecondary
                                        wrapMode: Text.WordWrap
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2
                                visible: root.connectionsModel && root.connectionsModel.count > 0

                                Repeater {
                                    model: root.connectionsModel

                                    delegate: Card {
                                        id: connectionCard

                                        required property string name
                                        required property string host
                                        required property int port
                                        required property string username
                                        required property string keyPath
                                        required property bool usesAgent
                                        required property string credentialId

                                        Layout.fillWidth: true
                                        padding: Theme.sp4
                                        border.color: Theme.borderDefault
                                        radius: Theme.radiusLg

                                        readonly property string authLabel: usesAgent
                                                ? qsTr("Agent")
                                                : keyPath.length > 0 ? qsTr("Key")
                                                : qsTr("Password")
                                        readonly property color authTint: usesAgent
                                                ? Theme.accent
                                                : keyPath.length > 0 ? Theme.statusSuccess
                                                : Theme.statusWarning
                                        readonly property string summary: usesAgent
                                                ? qsTr("Uses the system SSH agent")
                                                : keyPath.length > 0
                                                  ? qsTr("Private key: %1").arg(keyPath.split("/").pop())
                                                  : (credentialId.length > 0
                                                     ? qsTr("Password stored in keychain")
                                                     : qsTr("Password stored directly"))

                                        RowLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp3

                                            Rectangle {
                                                Layout.alignment: Qt.AlignTop
                                                Layout.topMargin: Theme.sp0_5
                                                width: 22
                                                height: 22
                                                radius: Theme.radiusMd
                                                color: Qt.rgba(connectionCard.authTint.r,
                                                               connectionCard.authTint.g,
                                                               connectionCard.authTint.b,
                                                               Theme.dark ? 0.18 : 0.10)
                                                border.color: Qt.rgba(connectionCard.authTint.r,
                                                                      connectionCard.authTint.g,
                                                                      connectionCard.authTint.b,
                                                                      Theme.dark ? 0.34 : 0.18)
                                                border.width: 1

                                                Image {
                                                    anchors.centerIn: parent
                                                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/server.svg"
                                                    sourceSize: Qt.size(14, 14)
                                                    layer.enabled: true
                                                    layer.effect: MultiEffect {
                                                        colorization: 1.0
                                                        colorizationColor: connectionCard.authTint
                                                    }
                                                }
                                            }

                                            ColumnLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp0_5

                                                RowLayout {
                                                    Layout.fillWidth: true
                                                    spacing: Theme.sp2

                                                    Text {
                                                        Layout.fillWidth: true
                                                        text: name.length > 0 ? name : host
                                                        font.family: Theme.fontUi
                                                        font.pixelSize: Theme.sizeBody
                                                        font.weight: Theme.weightMedium
                                                        color: Theme.textPrimary
                                                        elide: Text.ElideRight
                                                    }

                                                    ConnectionBadge {
                                                        label: connectionCard.authLabel
                                                        tint: connectionCard.authTint
                                                    }
                                                }

                                                Text {
                                                    Layout.fillWidth: true
                                                    text: username + "@" + host + ":" + port
                                                    font.family: Theme.fontMono
                                                    font.pixelSize: Theme.sizeCaption
                                                    color: Theme.textSecondary
                                                    elide: Text.ElideRight
                                                }

                                                Text {
                                                    Layout.fillWidth: true
                                                    text: connectionCard.summary
                                                    font.family: Theme.fontUi
                                                    font.pixelSize: Theme.sizeSmall
                                                    color: Theme.textTertiary
                                                    elide: Text.ElideRight
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Item { implicitHeight: root.pagePadding }
                        }
                    }
                }
            }
        }
    }

    component SettingRow: Item {
        id: settingRow

        property string title: ""
        property string description: ""
        default property alias trailing: trailingRow.data

        implicitHeight: Math.max(46, labelColumn.implicitHeight + Theme.sp3 * 2)

        RowLayout {
            anchors.fill: parent
            spacing: Theme.sp4

            ColumnLayout {
                id: labelColumn
                Layout.preferredWidth: root.rowLabelWidth
                Layout.maximumWidth: root.rowLabelWidth
                spacing: Theme.sp0_5

                Text {
                    text: settingRow.title
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    wrapMode: Text.WordWrap
                }

                Text {
                    visible: settingRow.description.length > 0
                    text: settingRow.description
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }

            Item { Layout.fillWidth: true }

            RowLayout {
                id: trailingRow
                spacing: Theme.sp2
                Layout.alignment: Qt.AlignVCenter
            }
        }
    }

    component ConnectionBadge: Rectangle {
        property string label: ""
        property color tint: Theme.accent

        implicitHeight: 22
        implicitWidth: badgeText.implicitWidth + Theme.sp2 * 2
        radius: Theme.radiusPill
        color: Qt.rgba(tint.r, tint.g, tint.b, Theme.dark ? 0.18 : 0.12)
        border.color: Qt.rgba(tint.r, tint.g, tint.b, Theme.dark ? 0.36 : 0.24)
        border.width: 1

        Text {
            id: badgeText
            anchors.centerIn: parent
            text: parent.label
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeSmall
            font.weight: Theme.weightMedium
            color: parent.tint
        }
    }
}
