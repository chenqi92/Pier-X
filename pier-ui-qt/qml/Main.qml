import QtQuick
import QtQuick.Window
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier

ApplicationWindow {
    id: window
    width: 1280
    height: 800
    minimumWidth: 800
    minimumHeight: 500
    visible: true
    title: qsTr("Pier-X")

    color: Theme.bgCanvas
    Behavior on color {
        ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType }
    }

    // ─────────────────────────────────────────────────────
    // Global keyboard shortcuts
    // ─────────────────────────────────────────────────────
    Shortcut {
        sequences: ["Ctrl+K", "Meta+K"]
        onActivated: commandPalette.toggle()
    }
    Shortcut {
        sequences: ["Ctrl+T", "Meta+T"]
        onActivated: window.openNewTab()
    }
    Shortcut {
        sequences: ["Ctrl+W", "Meta+W"]
        onActivated: window.closeTab(window.currentTabIndex)
    }
    Shortcut {
        sequences: ["Ctrl+N", "Meta+N"]
        onActivated: newConnectionDialog.show()
    }
    Shortcut {
        sequences: ["Ctrl+,", "Meta+,"]
        onActivated: settingsDialog.show()
    }

    // ─────────────────────────────────────────────────────
    // App-wide models — will become C++/Rust models once
    // pier-core lands. Plain QML ListModel for now.
    // ─────────────────────────────────────────────────────
    ListModel { id: tabModel }
    ListModel { id: connectionsModel }

    property int currentTabIndex: 0

    function openNewTab(title) {
        const t = title || qsTr("Local %1").arg(tabModel.count + 1)
        tabModel.append({ title: t })
        currentTabIndex = tabModel.count - 1
    }

    function closeTab(index) {
        if (index < 0 || index >= tabModel.count)
            return
        tabModel.remove(index)
        if (currentTabIndex >= tabModel.count) {
            currentTabIndex = Math.max(0, tabModel.count - 1)
        }
    }

    function addConnection(conn) {
        connectionsModel.append(conn)
    }

    function activateConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        openNewTab(conn.name)
    }

    // ─────────────────────────────────────────────────────
    // IDE shell
    // ─────────────────────────────────────────────────────
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TopBar {
            Layout.fillWidth: true
            onNewSessionRequested: newConnectionDialog.show()
            onCommandPaletteRequested: commandPalette.show()
            onSettingsRequested: settingsDialog.show()
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            Sidebar {
                Layout.fillHeight: true
                connectionsModel: connectionsModel
                onAddConnectionRequested: newConnectionDialog.show()
                onConnectionActivated: (i) => window.activateConnection(i)
                onOpenLocalTerminalRequested: window.openNewTab()
            }

            // Main content area
            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                WelcomeView {
                    anchors.fill: parent
                    visible: tabModel.count === 0
                    onOpenLocalTerminalRequested: window.openNewTab()
                    onNewSshRequested: newConnectionDialog.show()
                }

                ColumnLayout {
                    anchors.fill: parent
                    visible: tabModel.count > 0
                    spacing: 0

                    TabBar {
                        Layout.fillWidth: true
                        model: tabModel
                        currentIndex: window.currentTabIndex
                        onTabClicked: (i) => window.currentTabIndex = i
                        onTabClosed: (i) => window.closeTab(i)
                        onNewTabClicked: window.openNewTab()
                    }

                    // One TerminalView per tab, kept alive as the user
                    // switches between them so each tab owns its own
                    // PierTerminalSession (and its own child shell).
                    // StackLayout hides inactive tabs without destroying
                    // them — matches IDE terminal behavior.
                    StackLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        currentIndex: window.currentTabIndex

                        Repeater {
                            model: tabModel
                            delegate: TerminalView {}
                        }
                    }
                }
            }
        }

        StatusBar {
            Layout.fillWidth: true
        }
    }

    // ─────────────────────────────────────────────────────
    // Floating overlays
    // ─────────────────────────────────────────────────────
    NewConnectionDialog {
        id: newConnectionDialog
        onSaved: (conn) => {
            window.addConnection(conn)
            window.activateConnection(connectionsModel.count - 1)
        }
    }

    SettingsDialog {
        id: settingsDialog
        connectionsModel: connectionsModel
    }

    CommandPalette {
        id: commandPalette
        commands: [
            {
                title: qsTr("New local terminal"),
                shortcut: "Ctrl+T",
                action: function() { window.openNewTab() }
            },
            {
                title: qsTr("New SSH connection…"),
                shortcut: "Ctrl+N",
                action: function() { newConnectionDialog.show() }
            },
            {
                title: qsTr("Close current tab"),
                shortcut: "Ctrl+W",
                action: function() { window.closeTab(window.currentTabIndex) }
            },
            {
                title: qsTr("Toggle theme"),
                shortcut: "",
                action: function() {
                    Theme.followSystem = false
                    Theme.dark = !Theme.dark
                }
            },
            {
                title: qsTr("Follow system theme"),
                shortcut: "",
                action: function() { Theme.followSystem = true }
            },
            {
                title: qsTr("Settings…"),
                shortcut: "Ctrl+,",
                action: function() { settingsDialog.show() }
            },
            {
                title: qsTr("Quit Pier-X"),
                shortcut: "Ctrl+Q",
                action: function() { Qt.quit() }
            }
        ]
    }
}
