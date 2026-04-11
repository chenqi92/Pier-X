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
    // App-wide models
    // ─────────────────────────────────────────────────────
    //
    // Tab model is a transient in-process list — the open tabs
    // never persist across launches. Connections model, on the
    // other hand, is backed by pier-core's on-disk JSON store
    // (M3c2) via PierConnectionStore, with the password living
    // in the OS keychain via PierCredentials. The Sidebar still
    // takes a `connectionsModel` property; we just point it at
    // the backed store instead of an inline ListModel.
    ListModel { id: tabModel }

    PierConnectionStore {
        id: connectionsModel
        // reload() is called automatically in the C++ ctor, so by
        // the time this QML root is being instantiated the model
        // already reflects whatever's on disk.
    }

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
            sshPassword: "",
            sshCredentialId: "",
            sshKeyPath: "",
            sshPassphraseCredentialId: "",
            sshUsesAgent: false
        }
    }

    function _makeSshRow(conn) {
        return {
            title: conn.name,
            backend: "ssh",
            sshHost: conn.host,
            sshPort: conn.port,
            sshUser: conn.username,
            sshPassword: conn.password || "",
            sshCredentialId: conn.credentialId || "",
            sshKeyPath: conn.keyPath || "",
            sshPassphraseCredentialId: conn.passphraseCredentialId || "",
            sshUsesAgent: conn.usesAgent === true
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

    // Take a freshly-collected connection from the dialog and
    // dispatch by auth method:
    //
    //   "password"    → store password in keychain under a
    //                   generated id, persist a no-secrets
    //                   entry, open a tab via credential id.
    //   "private_key" → if there's a passphrase, store it in
    //                   keychain under a generated id; persist
    //                   the key path + (optional) passphrase
    //                   credential id; open a tab via key auth.
    //
    // The plaintext password / passphrase lives only in this
    // single function call. Nothing on the JS heap retains it
    // after we return — the keychain owns the secret from then
    // on, and the Rust SSH layer reads it back at handshake
    // time.
    function saveAndConnect(conn) {
        if (conn.authKind === "agent") {
            // Agent auth: no secrets to collect at all. Just
            // persist the connection with usesAgent=true and
            // open the tab — the Rust side will talk to the
            // OS agent at connect time.
            if (!connectionsModel.addAgent(conn.name, conn.host, conn.port, conn.username)) {
                console.warn("Main: failed to persist agent connection")
                return
            }
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                usesAgent: true
            })
            return
        }
        if (conn.authKind === "private_key") {
            if (!conn.privateKeyPath || conn.privateKeyPath.length === 0) {
                console.warn("Main: private_key path missing")
                return
            }
            // Store passphrase in keychain only if provided.
            // Empty passphrase = unencrypted key.
            var passphraseCredentialId = ""
            if (conn.passphrase && conn.passphrase.length > 0) {
                passphraseCredentialId = PierCredentials.freshId()
                if (!PierCredentials.setEntry(passphraseCredentialId, conn.passphrase)) {
                    console.warn("Main: failed to save passphrase to keychain")
                    return
                }
            }
            if (!connectionsModel.addKey(conn.name, conn.host, conn.port,
                                         conn.username, conn.privateKeyPath,
                                         passphraseCredentialId)) {
                if (passphraseCredentialId.length > 0) {
                    PierCredentials.deleteEntry(passphraseCredentialId)
                }
                console.warn("Main: failed to persist key connection")
                return
            }
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                keyPath: conn.privateKeyPath,
                passphraseCredentialId: passphraseCredentialId
            })
            return
        }

        // Default = password auth.
        const credentialId = PierCredentials.freshId()
        if (!PierCredentials.setEntry(credentialId, conn.password)) {
            console.warn("Main: failed to save credential to keychain")
            return
        }
        if (!connectionsModel.add(conn.name, conn.host, conn.port,
                                  conn.username, credentialId)) {
            // Persist failed — clean up the orphan keychain entry
            // so we don't accumulate dead secrets across runs.
            PierCredentials.deleteEntry(credentialId)
            console.warn("Main: failed to persist connection")
            return
        }
        openSshTab({
            name: conn.name,
            host: conn.host,
            port: conn.port,
            username: conn.username,
            credentialId: credentialId
        })
    }

    function activateConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        if (!conn) return
        // Dispatch on whichever auth field is populated.
        // Priority: agent > key > credential id. The four
        // never coexist in practice — the add* methods
        // enforce exclusivity.
        if (conn.usesAgent === true) {
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                usesAgent: true
            })
            return
        }
        if (conn.keyPath && conn.keyPath.length > 0) {
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                keyPath: conn.keyPath,
                passphraseCredentialId: conn.passphraseCredentialId || ""
            })
            return
        }
        if (conn.credentialId && conn.credentialId.length > 0) {
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                credentialId: conn.credentialId
            })
            return
        }
        // Unknown auth shape — fall back to a local tab so the
        // user at least gets some feedback.
        openNewTab(conn.name)
    }

    // Delete a saved connection AND any keychain entries it
    // owns. Both auth kinds may have at most one keychain entry
    // (password id for password auth; passphrase id for an
    // encrypted key). Wired to the sidebar's hover-revealed
    // Delete button (M3c3).
    function removeConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        if (conn) {
            if (conn.credentialId && conn.credentialId.length > 0) {
                PierCredentials.deleteEntry(conn.credentialId)
            }
            if (conn.passphraseCredentialId && conn.passphraseCredentialId.length > 0) {
                PierCredentials.deleteEntry(conn.passphraseCredentialId)
            }
        }
        connectionsModel.removeAt(index)
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
                onConnectionDeleted: (i) => window.removeConnection(i)
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
                                // model row. TerminalView's
                                // _dispatchSshConnect picks the
                                // right startSsh* path based on
                                // which fields are populated:
                                //   usesAgent true    → agent auth
                                //   keyPath set       → key auth
                                //   credentialId set  → keychain pwd
                                //   else              → plaintext pwd
                                backend: model.backend
                                sshHost: model.sshHost
                                sshPort: model.sshPort
                                sshUser: model.sshUser
                                sshPassword: model.sshPassword
                                sshCredentialId: model.sshCredentialId
                                sshKeyPath: model.sshKeyPath
                                sshPassphraseCredentialId: model.sshPassphraseCredentialId
                                sshUsesAgent: model.sshUsesAgent
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
            // M3c2: store password in OS keychain under a
            // generated id, persist the connection (without
            // secrets) to disk, then open a live SSH tab that
            // reconnects via the credential id.
            window.saveAndConnect(conn)
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
