import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import QtCore
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
    readonly property int rowLabelWidth: 196

    signal closed

    // Persist terminal theme selection across restarts.
    Settings {
        id: terminalSettings
        category: "terminal"
        property int themeIndex: 0
        property string fontFamily: "JetBrains Mono"
        property int fontSize: 13
        property int cursorStyle: 0
        property bool cursorBlink: true
        property int scrollbackLines: 10000
        property bool visualBell: true
        property bool audioBell: false
    }
    Settings {
        id: appearanceSettings
        category: "appearance"
        property string uiFontFamily: "Inter"
        property real uiScale: 1.0
    }
    Component.onCompleted: {
        Theme.fontUi = appearanceSettings.uiFontFamily
        Theme.uiScale = appearanceSettings.uiScale
        Theme.terminalThemeIndex = terminalSettings.themeIndex
        Theme.fontMono = terminalSettings.fontFamily
        Theme.terminalFontSize = terminalSettings.fontSize
        Theme.cursorStyle = terminalSettings.cursorStyle
        Theme.cursorBlink = terminalSettings.cursorBlink
        Theme.scrollbackLines = terminalSettings.scrollbackLines
        Theme.visualBell = terminalSettings.visualBell
        Theme.audioBell = terminalSettings.audioBell
    }

    visible: open
    z: 9400
    anchors.fill: parent

    function show() {
        open = true
        sectionList.currentIndex = 0
    }

    function hide() {
        open = false
        closed()
    }

    Keys.onEscapePressed: hide()

    ModalDialogShell {
        open: root.open
        dialogWidth: 980
        dialogHeight: 700
        title: qsTr("Settings")
        subtitle: qsTr("Adjust appearance, terminal behavior, and saved connections.")
        bodyPadding: 0
        onRequestClose: root.hide()

        body: RowLayout {
            anchors.fill: parent
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

                    SettingsPageScroll {
                        id: generalScroll
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp5

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Theme") }

                                SettingGroupCard {
                                    SettingRow {
                                        Layout.fillWidth: true
                                        title: qsTr("Follow system theme")
                                        description: qsTr("Automatically mirror the operating system appearance.")

                                        ToggleSwitch {
                                            checked: Theme.followSystem
                                            onToggled: (checked) => Theme.followSystem = checked
                                        }
                                    }

                                    SettingDivider { }

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
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Language") }

                                SettingGroupCard {
                                    SettingRow {
                                        Layout.fillWidth: true
                                        title: qsTr("Interface language")
                                        description: qsTr("Change the display language. Takes effect immediately for most text.")

                                        PierComboBox {
                                            implicitWidth: 168
                                            options: PierI18n.displayNames
                                            currentIndex: PierI18n.currentIndex
                                            onActivated: (i) => {
                                                PierI18n.switchLanguage(PierI18n.codes[i])
                                            }
                                        }
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        visible: PierI18n.language !== "en"
                                        text: qsTr("Some labels may require restarting Pier-X to update fully.")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                        wrapMode: Text.WordWrap
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3
                                visible: PierUpdate.available

                                SectionLabel { text: qsTr("Updates") }

                                SettingGroupCard {
                                    SettingRow {
                                        Layout.fillWidth: true
                                        title: qsTr("Automatic updates")
                                        description: qsTr("Periodically check for new versions in the background.")

                                        ToggleSwitch {
                                            checked: PierUpdate.autoCheck
                                            onToggled: (checked) => PierUpdate.autoCheck = checked
                                        }
                                    }

                                    SettingDivider { }

                                    SettingRow {
                                        Layout.fillWidth: true
                                        title: qsTr("Check now")
                                        description: qsTr("Manually check for a newer version of Pier-X.")

                                        GhostButton {
                                            text: qsTr("Check for updates…")
                                            onClicked: PierUpdate.checkForUpdates()
                                        }
                                    }
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Developer") }

                                SettingGroupCard {
                                    SettingRow {
                                        Layout.fillWidth: true
                                        title: qsTr("Performance overlay")
                                        description: qsTr("Show FPS, memory usage, and startup time in the status bar.")

                                        ToggleSwitch {
                                            checked: PierProfiler.enabled
                                            onToggled: (checked) => PierProfiler.enabled = checked
                                        }
                                    }
                                }
                            }

                        }
                    }

                    SettingsPageScroll {
                        id: appearanceScroll
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp5

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Typography") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("UI font")
                                    description: qsTr("Primary interface font used for labels, buttons, and navigation.")

                                    PierComboBox {
                                        implicitWidth: 200
                                        options: Theme.uiFontFamilies
                                        currentIndex: {
                                            var idx = options.indexOf(Theme.fontUi)
                                            return idx >= 0 ? idx : 0
                                        }
                                        onActivated: (i) => {
                                            Theme.fontUi = options[i]
                                            appearanceSettings.uiFontFamily = options[i]
                                        }
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Interface text size")
                                    description: qsTr("Scales typography across the app without changing layout density too aggressively.")

                                    RowLayout {
                                        spacing: Theme.sp2

                                        PierSlider {
                                            implicitWidth: 160
                                            from: 0.9
                                            to: 1.2
                                            stepSize: 0.05
                                            value: Theme.uiScale
                                            onValueChanged: {
                                                var scaled = Math.round(value * 100) / 100
                                                Theme.uiScale = scaled
                                                appearanceSettings.uiScale = scaled
                                            }
                                        }

                                        Text {
                                            text: Math.round(Theme.uiScale * 100) + "%"
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textSecondary
                                            Layout.preferredWidth: 44
                                        }
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Code / mono font")
                                    description: qsTr("Used for terminal content, paths, ports, and code-like data.")

                                    PierComboBox {
                                        implicitWidth: 200
                                        options: Theme.monoFontFamilies
                                        currentIndex: {
                                            var idx = options.indexOf(Theme.fontMono)
                                            return idx >= 0 ? idx : 0
                                        }
                                        onActivated: (i) => {
                                            Theme.fontMono = options[i]
                                            terminalSettings.fontFamily = options[i]
                                        }
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
                                    implicitHeight: appearancePreviewColumn.implicitHeight + padding * 2

                                    ColumnLayout {
                                        id: appearancePreviewColumn
                                        width: parent.width
                                        spacing: Theme.sp2

                                        Text {
                                            Layout.fillWidth: true
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
                                    implicitHeight: monoPreviewColumn.implicitHeight + padding * 2

                                    ColumnLayout {
                                        id: monoPreviewColumn
                                        width: parent.width
                                        spacing: Theme.sp2

                                        Text {
                                            Layout.fillWidth: true
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

                        }
                    }

                    SettingsPageScroll {
                        id: terminalScroll
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp5

                            // ── Terminal Theme ───────────────────────
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Terminal Theme") }

                                Card {
                                    Layout.fillWidth: true
                                    padding: Theme.sp1
                                    implicitHeight: terminalThemeList.implicitHeight + padding * 2

                                    ColumnLayout {
                                        id: terminalThemeList
                                        width: parent.width
                                        spacing: 0

                                        Repeater {
                                            model: Theme.terminalThemes

                                            delegate: Rectangle {
                                                id: themeRow
                                                required property int index
                                                required property var modelData
                                                Layout.fillWidth: true
                                                implicitHeight: 40
                                                radius: Theme.radiusSm
                                                color: themeMouseArea.containsMouse
                                                       ? Theme.bgHover
                                                       : (themeRow.index === Theme.terminalThemeIndex
                                                          ? Theme.bgSelected : "transparent")

                                                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                                RowLayout {
                                                    anchors.fill: parent
                                                    anchors.leftMargin: Theme.sp3
                                                    anchors.rightMargin: Theme.sp3
                                                    spacing: Theme.sp2

                                                    // Color swatches — show first 8 ANSI colors
                                                    Row {
                                                        spacing: Theme.sp0_5

                                                        Repeater {
                                                            model: 8
                                                            delegate: Rectangle {
                                                                required property int index
                                                                width: 14
                                                                height: 14
                                                                radius: 7
                                                                color: themeRow.modelData.ansi[index]
                                                                border.color: Qt.rgba(0,0,0,0.15)
                                                                border.width: 1
                                                            }
                                                        }
                                                    }

                                                    Text {
                                                        text: Theme.terminalThemeName(themeRow.modelData)
                                                        font.family: Theme.fontUi
                                                        font.pixelSize: Theme.sizeBody
                                                        font.weight: themeRow.index === Theme.terminalThemeIndex
                                                                     ? Theme.weightMedium
                                                                     : Theme.weightRegular
                                                        color: Theme.textPrimary
                                                    }

                                                    Item { Layout.fillWidth: true }

                                                    // Checkmark for selected theme
                                                    Text {
                                                        visible: themeRow.index === Theme.terminalThemeIndex
                                                        text: "✓"
                                                        font.pixelSize: Theme.sizeBodyLg
                                                        font.weight: Theme.weightSemibold
                                                        color: Theme.statusSuccess
                                                    }
                                                }

                                                MouseArea {
                                                    id: themeMouseArea
                                                    anchors.fill: parent
                                                    hoverEnabled: true
                                                    cursorShape: Qt.PointingHandCursor
                                                    onClicked: {
                                                        Theme.terminalThemeIndex = themeRow.index
                                                        terminalSettings.themeIndex = themeRow.index
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Preview card — shows what the theme looks like.
                                Card {
                                    Layout.fillWidth: true
                                    padding: 0
                                    implicitHeight: terminalThemePreviewSurface.implicitHeight

                                    Rectangle {
                                        id: terminalThemePreviewSurface
                                        width: parent.width
                                        implicitHeight: terminalThemePreviewColumn.implicitHeight + Theme.sp3 * 2
                                        color: Theme.currentTerminalTheme.bg
                                        radius: Theme.radiusMd

                                        Column {
                                            id: terminalThemePreviewColumn
                                            x: Theme.sp3
                                            y: Theme.sp3
                                            width: parent.width - Theme.sp3 * 2
                                            spacing: 2

                                            Text {
                                                text: "$ ssh admin@prod-02"
                                                font.family: Theme.fontMono
                                                font.pixelSize: Theme.terminalFontSize
                                                color: Theme.currentTerminalTheme.fg
                                            }

                                            Row {
                                                spacing: 0
                                                Text { text: "admin"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[2] }
                                                Text { text: "@"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.fg }
                                                Text { text: "prod-02"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[4] }
                                                Text { text: ":"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.fg }
                                                Text { text: "~"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[5] }
                                                Text { text: "$ "; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.fg }
                                                Text { text: "ls"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[3] }
                                            }

                                            Row {
                                                spacing: Theme.sp3
                                                Text { text: "README.md"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.fg }
                                                Text { text: "src/"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[4] }
                                                Text { text: "docs/"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[4] }
                                                Text { text: "Makefile"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[2] }
                                                Text { text: ".env"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[8] }
                                            }

                                            Row {
                                                spacing: 0
                                                Text { text: "Error: "; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.ansi[1] }
                                                Text { text: "connection refused"; font.family: Theme.fontMono; font.pixelSize: Theme.terminalFontSize; color: Theme.currentTerminalTheme.fg }
                                            }
                                        }
                                    }
                                }
                            }

                            // ── Font ─────────────────────────────────
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                SectionLabel { text: qsTr("Font") }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Font family")
                                    description: qsTr("Monospace font used in the terminal.")

                                    PierComboBox {
                                        implicitWidth: 200
                                        options: Theme.monoFontFamilies
                                        currentIndex: {
                                            var idx = options.indexOf(Theme.fontMono)
                                            return idx >= 0 ? idx : 0
                                        }
                                        onActivated: (i) => {
                                            Theme.fontMono = options[i]
                                            terminalSettings.fontFamily = options[i]
                                        }
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Font size")
                                    description: qsTr("Pixel size for terminal text.")

                                    RowLayout {
                                        spacing: Theme.sp2

                                        PierSlider {
                                            id: fontSizeSlider
                                            implicitWidth: 160
                                            from: 9
                                            to: 24
                                            stepSize: 1
                                            value: Theme.terminalFontSize
                                            onValueChanged: {
                                                Theme.terminalFontSize = value
                                                terminalSettings.fontSize = value
                                            }
                                        }

                                        Text {
                                            text: Theme.terminalFontSize + "px"
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textSecondary
                                            Layout.preferredWidth: 32
                                        }
                                    }
                                }

                                // Font preview
                                Card {
                                    Layout.fillWidth: true
                                    padding: Theme.sp3

                                    Text {
                                        text: "天地玄黄宇宙洪荒 The quick brown fox"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.terminalFontSize
                                        color: Theme.textPrimary
                                    }
                                }
                            }

                            // ── Cursor ───────────────────────────────
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
                                        currentIndex: Theme.cursorStyle
                                        onActivated: (i) => {
                                            Theme.cursorStyle = i
                                            terminalSettings.cursorStyle = i
                                        }
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Cursor blink")
                                    description: qsTr("Animate the cursor when the terminal is focused.")

                                    ToggleSwitch {
                                        checked: Theme.cursorBlink
                                        onToggled: (checked) => {
                                            Theme.cursorBlink = checked
                                            terminalSettings.cursorBlink = checked
                                        }
                                    }
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
                                        text: Theme.scrollbackLines.toString()
                                        validator: IntValidator { bottom: 1000; top: 100000 }
                                        onEditingFinished: {
                                            var val = parseInt(text)
                                            if (!isNaN(val)) {
                                                val = Math.max(1000, Math.min(100000, val))
                                                Theme.scrollbackLines = val
                                                terminalSettings.scrollbackLines = val
                                                text = val.toString()
                                            }
                                        }
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

                                    ToggleSwitch {
                                        checked: Theme.visualBell
                                        onToggled: (checked) => {
                                            Theme.visualBell = checked
                                            terminalSettings.visualBell = checked
                                        }
                                    }
                                }

                                SettingRow {
                                    Layout.fillWidth: true
                                    title: qsTr("Audio bell")
                                    description: qsTr("Play the terminal bell sound when supported.")

                                    ToggleSwitch {
                                        checked: Theme.audioBell
                                        onToggled: (checked) => {
                                            Theme.audioBell = checked
                                            terminalSettings.audioBell = checked
                                        }
                                    }
                                }
                            }

                        }
                    }

                    SettingsPageScroll {
                        id: connectionsScroll
                        ColumnLayout {
                            Layout.fillWidth: true
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
                                implicitHeight: emptyConnectionsColumn.implicitHeight + padding * 2

                                ColumnLayout {
                                    id: emptyConnectionsColumn
                                    width: parent.width
                                    spacing: Theme.sp1

                                    Text {
                                        Layout.fillWidth: true
                                        text: qsTr("No connections saved yet.")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                    }

                                    Text {
                                        Layout.fillWidth: true
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
                                        implicitHeight: connectionRow.implicitHeight + padding * 2
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
                                            id: connectionRow
                                            width: parent.width
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

                        }
                    }
                }
        }
    }

    component SettingsPageScroll: PierScrollView {
        id: pageScroll
        default property alias content: pageColumn.data

        clip: true
        contentWidth: width

        Item {
            width: pageScroll.width
            implicitHeight: pageColumn.implicitHeight + root.pagePadding * 2

            ColumnLayout {
                id: pageColumn
                width: Math.min(parent.width - root.pagePadding * 2, 720)
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: parent.top
                anchors.topMargin: root.pagePadding
                spacing: Theme.sp5
            }
        }
    }

    component SettingRow: Item {
        id: settingRow

        property string title: ""
        property string description: ""
        default property alias trailing: trailingRow.data

        Layout.fillWidth: true
        Layout.minimumWidth: 0
        implicitHeight: Math.max(46, labelColumn.implicitHeight + Theme.sp3 * 2)

        RowLayout {
            anchors.fill: parent
            spacing: Theme.sp4

            ColumnLayout {
                id: labelColumn
                Layout.preferredWidth: root.rowLabelWidth
                Layout.maximumWidth: root.rowLabelWidth
                Layout.minimumWidth: root.rowLabelWidth
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
                Layout.minimumWidth: 0
                Layout.alignment: Qt.AlignTop
            }
        }
    }

    component SettingGroupCard: Card {
        Layout.fillWidth: true
        padding: Theme.sp4
        inset: true
        implicitHeight: groupColumn.implicitHeight + padding * 2

        default property alias content: groupColumn.data

        ColumnLayout {
            id: groupColumn
            anchors.fill: parent
            spacing: Theme.sp3
        }
    }

    component SettingDivider: Rectangle {
        Layout.fillWidth: true
        height: 1
        color: Theme.borderSubtle
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
