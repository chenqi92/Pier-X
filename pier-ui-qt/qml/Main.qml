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

    // Every tabModel row carries the full schema, with unused
    // fields defaulted, so the Repeater delegate can bind to
    // model.<role> unconditionally without dealing with undefined
    // columns. ListModel doesn't grow its role set after the first
    // append, so we have to bake all of these in from row #1.
    function _makeLocalRow(title) {
        return {
            title: title,
            backend: "local",
            sshHost: "",
            sshPort: 22,
            sshUser: "",
            sshPassword: ""
        }
    }

    function _makeSshRow(conn) {
        return {
            title: conn.name,
            backend: "ssh",
            sshHost: conn.host,
            sshPort: conn.port,
            sshUser: conn.username,
            sshPassword: conn.password || ""
        }
    }

    function openNewTab(title) {
        const t = title || qsTr("Local %1").arg(tabModel.count + 1)
        tabModel.append(_makeLocalRow(t))
        currentTabIndex = tabModel.count - 1
    }

    function openSshTab(conn) {
        tabModel.append(_makeSshRow(conn))
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
        // Persist a sanitized copy — the password is intentionally
        // NOT stored in connectionsModel. M3c will replace this
        // with keychain lookups; for now the password travels from
        // the dialog straight into the tab and then the SSH handshake.
        connectionsModel.append({
            name: conn.name,
            host: conn.host,
            port: conn.port,
            username: conn.username,
            authKind: conn.authKind
        })
    }

    function activateConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        // Without a stored password the sidebar activate path
        // can't connect on its own — it just opens a blank local
        // tab for now. M3c will re-prompt or pull from keychain.
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
                            delegate: TerminalView {
                                // Bind every backend field from the
                                // model row. For local tabs the ssh
                                // fields are empty strings / 22 and
                                // startWhenSized ignores them.
                                backend: model.backend
                                sshHost: model.sshHost
                                sshPort: model.sshPort
                                sshUser: model.sshUser
                                sshPassword: model.sshPassword
                            }
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
            // Remember the connection (without the password) in
            // the sidebar list, then open a live SSH tab that
            // does the actual handshake using the password we
            // just collected.
            window.addConnection(conn)
            window.openSshTab(conn)
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
