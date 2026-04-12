import QtQuick
import QtQuick.Window
import QtQuick.Controls.Basic
import QtQuick.Dialogs
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
            sshUsesAgent: false,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
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
            sshUsesAgent: conn.usesAgent === true,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    // SFTP tab row — same SSH field shape as an ssh tab, just
    // with backend = "sftp" so the Repeater delegate knows to
    // load SftpBrowserView instead of TerminalView.
    function _makeSftpRow(conn) {
        return {
            title: qsTr("📁 %1").arg(conn.name),
            backend: "sftp",
            sshHost: conn.host,
            sshPort: conn.port,
            sshUser: conn.username,
            sshPassword: conn.password || "",
            sshCredentialId: conn.credentialId || "",
            sshKeyPath: conn.keyPath || "",
            sshPassphraseCredentialId: conn.passphraseCredentialId || "",
            sshUsesAgent: conn.usesAgent === true,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    // Redis tab row — M5a per-service panel. Connects to a
    // plain TCP endpoint (typically the local side of an SSH
    // tunnel, e.g. 127.0.0.1:16379). No SSH auth — the
    // encryption is already provided by the tunnel that opened
    // the port.
    function _makeRedisRow(host, port, db, label) {
        return {
            title: qsTr("⧉ %1").arg(label),
            backend: "redis",
            sshHost: "",
            sshPort: 22,
            sshUser: "",
            sshPassword: "",
            sshCredentialId: "",
            sshKeyPath: "",
            sshPassphraseCredentialId: "",
            sshUsesAgent: false,
            redisHost: host,
            redisPort: port,
            redisDb: db,
            logCommand: "",
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    function openRedisTab(host, port, db, label) {
        tabModel.append(_makeRedisRow(host, port, db, label || (host + ":" + port)))
        currentTabIndex = tabModel.count - 1
    }

    // Log viewer tab row — M5b per-service panel. Uses the
    // same SSH field shape as an ssh/sftp tab (the log stream
    // connects via its own SshSession under the hood) and
    // carries the remote `command` to exec in the logCommand
    // field.
    function _makeLogRow(conn, command, label) {
        return {
            title: qsTr("Log: %1").arg(label || conn.name),
            backend: "log",
            sshHost: conn.host,
            sshPort: conn.port,
            sshUser: conn.username,
            sshPassword: conn.password || "",
            sshCredentialId: conn.credentialId || "",
            sshKeyPath: conn.keyPath || "",
            sshPassphraseCredentialId: conn.passphraseCredentialId || "",
            sshUsesAgent: conn.usesAgent === true,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: command,
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    function openLogTab(conn, command, label) {
        tabModel.append(_makeLogRow(conn, command, label))
        currentTabIndex = tabModel.count - 1
    }

    // Quick entry: tail syslog on the saved connection at
    // `index`. Used by the command palette; a sidebar context
    // menu will grow more options later.
    function openLogForConnection(index, command) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        if (!conn) return
        openLogTab(conn, command || "tail -f /var/log/syslog",
                   command ? conn.name : "syslog")
    }

    // Docker panel tab row — M5c per-service panel. Uses the
    // same SSH field shape as an ssh/sftp tab; the panel runs
    // `docker ps` / `start` / `stop` / `rm` via one-shot
    // exec_command and links out to the Log viewer for live
    // `docker logs -f`.
    function _makeDockerRow(conn) {
        return {
            title: qsTr("Docker: %1").arg(conn.name),
            backend: "docker",
            sshHost: conn.host,
            sshPort: conn.port,
            sshUser: conn.username,
            sshPassword: conn.password || "",
            sshCredentialId: conn.credentialId || "",
            sshKeyPath: conn.keyPath || "",
            sshPassphraseCredentialId: conn.passphraseCredentialId || "",
            sshUsesAgent: conn.usesAgent === true,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: "",
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    function openDockerTab(conn) {
        tabModel.append(_makeDockerRow(conn))
        currentTabIndex = tabModel.count - 1
    }

    // Quick entry: Docker panel on the saved connection at
    // `index`. Used by the command palette.
    function openDockerForConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        if (!conn) return
        openDockerTab(conn)
    }

    // Markdown preview tab row — M5e per-service panel. Pure
    // local: no SSH, no sessions, no services. Carries only a
    // filesystem path; the view loads + renders on mount.
    function _makeMarkdownRow(filePath) {
        // Strip path to basename for the tab title.
        var slash = filePath.lastIndexOf("/")
        if (slash < 0) slash = filePath.lastIndexOf("\\")
        var name = slash >= 0 ? filePath.slice(slash + 1) : filePath
        return {
            title: qsTr("MD: %1").arg(name),
            backend: "markdown",
            sshHost: "",
            sshPort: 22,
            sshUser: "",
            sshPassword: "",
            sshCredentialId: "",
            sshKeyPath: "",
            sshPassphraseCredentialId: "",
            sshUsesAgent: false,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: filePath,
            mysqlHost: "",
            mysqlPort: 3306,
            mysqlUser: "",
            mysqlPassword: "",
            mysqlDatabase: ""
        }
    }

    function openMarkdownTab(filePath) {
        if (!filePath || filePath.length === 0) return
        tabModel.append(_makeMarkdownRow(filePath))
        currentTabIndex = tabModel.count - 1
    }

    // MySQL client tab row — M5d per-service panel. Connects
    // to a plain TCP endpoint (typically the local side of an
    // SSH tunnel, e.g. 127.0.0.1:13306). The panel itself
    // shows a connect form up front; the fields below are
    // prefill hints only — the user can override them in the
    // form before clicking Connect.
    function _makeMysqlRow(host, port, user, password, database, label) {
        return {
            title: qsTr("MySQL: %1").arg(label || (user + "@" + host + ":" + port)),
            backend: "mysql",
            sshHost: "",
            sshPort: 22,
            sshUser: "",
            sshPassword: "",
            sshCredentialId: "",
            sshKeyPath: "",
            sshPassphraseCredentialId: "",
            sshUsesAgent: false,
            redisHost: "",
            redisPort: 0,
            redisDb: 0,
            logCommand: "",
            markdownPath: "",
            mysqlHost: host,
            mysqlPort: port,
            mysqlUser: user,
            mysqlPassword: password || "",
            mysqlDatabase: database || ""
        }
    }

    function openMysqlTab(host, port, user, password, database, label) {
        tabModel.append(_makeMysqlRow(host, port, user, password, database, label))
        currentTabIndex = tabModel.count - 1
    }

    function openSftpTab(conn) {
        tabModel.append(_makeSftpRow(conn))
        currentTabIndex = tabModel.count - 1
    }

    // Open an SFTP file browser for whatever saved connection
    // the given index points at. Called from the command
    // palette and from a future sidebar context-menu entry.
    function openSftpForConnection(index) {
        if (index < 0 || index >= connectionsModel.count)
            return
        const conn = connectionsModel.get(index)
        if (!conn) return
        openSftpTab(conn)
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
                            delegate: Loader {
                                // Pick the view class by
                                // backend. TerminalView /
                                // SftpBrowserView / RedisBrowserView
                                // each bind a disjoint set of
                                // model fields; the delegate
                                // copies every field through
                                // regardless of which class
                                // the Loader ends up
                                // instantiating.
                                required property string backend
                                required property string sshHost
                                required property int    sshPort
                                required property string sshUser
                                required property string sshPassword
                                required property string sshCredentialId
                                required property string sshKeyPath
                                required property string sshPassphraseCredentialId
                                required property bool   sshUsesAgent
                                required property string redisHost
                                required property int    redisPort
                                required property int    redisDb
                                required property string logCommand
                                required property string markdownPath
                                required property string mysqlHost
                                required property int    mysqlPort
                                required property string mysqlUser
                                required property string mysqlPassword
                                required property string mysqlDatabase

                                sourceComponent: backend === "sftp"
                                                 ? sftpComp
                                                 : (backend === "redis"
                                                    ? redisComp
                                                    : (backend === "log"
                                                       ? logComp
                                                       : (backend === "docker"
                                                          ? dockerComp
                                                          : (backend === "markdown"
                                                             ? markdownComp
                                                             : (backend === "mysql"
                                                                ? mysqlComp
                                                                : terminalComp)))))

                                Component {
                                    id: terminalComp
                                    TerminalView {
                                        backend: parent.backend
                                        sshHost: parent.sshHost
                                        sshPort: parent.sshPort
                                        sshUser: parent.sshUser
                                        sshPassword: parent.sshPassword
                                        sshCredentialId: parent.sshCredentialId
                                        sshKeyPath: parent.sshKeyPath
                                        sshPassphraseCredentialId: parent.sshPassphraseCredentialId
                                        sshUsesAgent: parent.sshUsesAgent
                                    }
                                }
                                Component {
                                    id: sftpComp
                                    SftpBrowserView {
                                        sshHost: parent.sshHost
                                        sshPort: parent.sshPort
                                        sshUser: parent.sshUser
                                        sshPassword: parent.sshPassword
                                        sshCredentialId: parent.sshCredentialId
                                        sshKeyPath: parent.sshKeyPath
                                        sshPassphraseCredentialId: parent.sshPassphraseCredentialId
                                        sshUsesAgent: parent.sshUsesAgent
                                    }
                                }
                                Component {
                                    id: redisComp
                                    RedisBrowserView {
                                        redisHost: parent.redisHost
                                        redisPort: parent.redisPort
                                        redisDb: parent.redisDb
                                    }
                                }
                                Component {
                                    id: logComp
                                    LogViewerView {
                                        sshHost: parent.sshHost
                                        sshPort: parent.sshPort
                                        sshUser: parent.sshUser
                                        sshPassword: parent.sshPassword
                                        sshCredentialId: parent.sshCredentialId
                                        sshKeyPath: parent.sshKeyPath
                                        sshPassphraseCredentialId: parent.sshPassphraseCredentialId
                                        sshUsesAgent: parent.sshUsesAgent
                                        logCommand: parent.logCommand
                                    }
                                }
                                Component {
                                    id: dockerComp
                                    DockerPanelView {
                                        sshHost: parent.sshHost
                                        sshPort: parent.sshPort
                                        sshUser: parent.sshUser
                                        sshPassword: parent.sshPassword
                                        sshCredentialId: parent.sshCredentialId
                                        sshKeyPath: parent.sshKeyPath
                                        sshPassphraseCredentialId: parent.sshPassphraseCredentialId
                                        sshUsesAgent: parent.sshUsesAgent
                                    }
                                }
                                Component {
                                    id: markdownComp
                                    MarkdownPreviewView {
                                        filePath: parent.markdownPath
                                    }
                                }
                                Component {
                                    id: mysqlComp
                                    MySqlPanelView {
                                        mysqlHost: parent.mysqlHost
                                        mysqlPort: parent.mysqlPort
                                        mysqlUser: parent.mysqlUser
                                        mysqlDatabase: parent.mysqlDatabase
                                    }
                                }
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

    // M5e: Native file picker for the "Open Markdown preview"
    // palette entry. Accepts .md / .markdown / .txt — the
    // renderer doesn't care about extension, but filtering
    // keeps the picker clean.
    FileDialog {
        id: markdownFileDialog
        title: qsTr("Open Markdown file")
        nameFilters: [
            qsTr("Markdown files (*.md *.markdown *.mdx)"),
            qsTr("Text files (*.txt)"),
            qsTr("All files (*)")
        ]
        fileMode: FileDialog.OpenFile
        onAccepted: {
            // `selectedFile` is a URL; convert to a filesystem
            // path before handing it to the Rust FFI.
            const path = markdownFileDialog.selectedFile.toString()
                .replace(/^file:\/\//, "")
            window.openMarkdownTab(path)
        }
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
                title: qsTr("Browse remote files (first saved connection)"),
                shortcut: "",
                action: function() {
                    // Quick entry point: open an SFTP browser
                    // for the first saved connection. A richer
                    // picker (list all saved + type to filter)
                    // lands when the command palette grows
                    // sub-lists; for M3d2 this is the smoke-
                    // test hook.
                    if (connectionsModel.count > 0) {
                        window.openSftpForConnection(0)
                    } else {
                        console.warn("No saved connections to browse.")
                    }
                }
            },
            {
                title: qsTr("Tail syslog (first saved connection)"),
                shortcut: "",
                action: function() {
                    // M5b smoke-test hook: opens a Log viewer
                    // tab that runs `tail -f /var/log/syslog`
                    // on the first saved SSH connection. A
                    // richer picker + custom-command form lands
                    // when the palette grows sub-lists.
                    if (connectionsModel.count > 0) {
                        window.openLogForConnection(0, "")
                    } else {
                        console.warn("No saved connections to tail.")
                    }
                }
            },
            {
                title: qsTr("Docker containers (first saved connection)"),
                shortcut: "",
                action: function() {
                    // M5c smoke-test hook: opens a Docker panel
                    // tab on the first saved SSH connection.
                    if (connectionsModel.count > 0) {
                        window.openDockerForConnection(0)
                    } else {
                        console.warn("No saved connections for Docker panel.")
                    }
                }
            },
            {
                title: qsTr("Open Markdown preview…"),
                shortcut: "",
                action: function() {
                    // M5e: native file picker → markdown tab.
                    markdownFileDialog.open()
                }
            },
            {
                title: qsTr("MySQL client (127.0.0.1:13306)"),
                shortcut: "",
                action: function() {
                    // M5d: open a MySQL panel tab pointing at
                    // the Pier-X tunnel convention port. The
                    // panel itself shows a connect form up
                    // front, so the user still fills in the
                    // user / password / database fields.
                    window.openMysqlTab("127.0.0.1", 13306, "root", "", "", "")
                }
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
