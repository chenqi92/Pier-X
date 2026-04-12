import QtQuick
import QtQuick.Effects
import QtQuick.Window
import QtQuick.Controls
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import Pier

ApplicationWindow {
    id: window
    width: 1400
    height: 900
    minimumWidth: Theme.windowMinWidth
    minimumHeight: Theme.windowMinHeight
    visible: true
    title: qsTr("Pier-X")

    color: Theme.bgCanvas
    Behavior on color {
        ColorAnimation { duration: Theme.durNormal; easing.type: Theme.easingType }
    }

    menuBar: MenuBar {
        Menu {
            title: qsTr("Pier-X")

            MenuItem { text: qsTr("About Pier-X"); onTriggered: aboutDialog.open() }
            MenuSeparator {}
            MenuItem {
                action: Action {
                    text: qsTr("Settings…")
                    shortcut: Qt.platform.os === "osx" ? "Meta+," : "Ctrl+,"
                    onTriggered: settingsDialog.show()
                }
            }
            MenuSeparator {}
            MenuItem {
                action: Action {
                    text: qsTr("Quit Pier-X")
                    shortcut: StandardKey.Quit
                    onTriggered: Qt.quit()
                }
            }
        }

        Menu {
            title: qsTr("File")

            MenuItem {
                action: Action {
                    text: qsTr("New local terminal")
                    shortcut: Qt.platform.os === "osx" ? "Meta+T" : "Ctrl+T"
                    onTriggered: window.openNewTab()
                }
            }
            MenuItem {
                action: Action {
                    text: qsTr("New SSH connection…")
                    shortcut: Qt.platform.os === "osx" ? "Meta+N" : "Ctrl+N"
                    onTriggered: newConnectionDialog.show()
                }
            }
            MenuItem { text: qsTr("Open Markdown preview…"); onTriggered: markdownFileDialog.open() }
            MenuSeparator {}
            MenuItem {
                action: Action {
                    text: qsTr("Close current tab")
                    shortcut: StandardKey.Close
                    enabled: tabModel.count > 0
                    onTriggered: window.closeTab(window.currentTabIndex)
                }
            }
        }

        Menu {
            title: qsTr("Edit")

            MenuItem { action: Action { text: qsTr("Undo"); shortcut: StandardKey.Undo; enabled: window._focusMethodAvailable("undo"); onTriggered: window._invokeFocusMethod("undo") } }
            MenuItem { action: Action { text: qsTr("Redo"); shortcut: StandardKey.Redo; enabled: window._focusMethodAvailable("redo"); onTriggered: window._invokeFocusMethod("redo") } }
            MenuSeparator {}
            MenuItem { action: Action { text: qsTr("Cut"); shortcut: StandardKey.Cut; enabled: window._focusMethodAvailable("cut"); onTriggered: window._invokeFocusMethod("cut") } }
            MenuItem { action: Action { text: qsTr("Copy"); shortcut: StandardKey.Copy; enabled: window._focusMethodAvailable("copy"); onTriggered: window._invokeFocusMethod("copy") } }
            MenuItem { action: Action { text: qsTr("Paste"); shortcut: StandardKey.Paste; enabled: window._focusMethodAvailable("paste"); onTriggered: window._invokeFocusMethod("paste") } }
            MenuItem { action: Action { text: qsTr("Select All"); shortcut: StandardKey.SelectAll; enabled: window._focusMethodAvailable("selectAll"); onTriggered: window._invokeFocusMethod("selectAll") } }
        }

        Menu {
            title: qsTr("View")

            MenuItem {
                action: Action {
                    text: qsTr("Command palette…")
                    shortcut: Qt.platform.os === "osx" ? "Meta+K" : "Ctrl+K"
                    onTriggered: commandPalette.show()
                }
            }
            MenuItem {
                text: Theme.dark ? qsTr("Switch to light theme") : qsTr("Switch to dark theme")
                onTriggered: { Theme.followSystem = false; Theme.dark = !Theme.dark }
            }
            MenuItem {
                text: qsTr("Follow system theme")
                checkable: true; checked: Theme.followSystem
                onTriggered: Theme.followSystem = checked
            }
            MenuSeparator {}
            MenuItem {
                action: Action {
                    text: window.gitPanelVisible ? qsTr("Hide right sidebar") : qsTr("Show right sidebar")
                    shortcut: Qt.platform.os === "osx" ? "Meta+Shift+G" : "Ctrl+Shift+G"
                    onTriggered: window.toggleGitPanel()
                }
            }
        }

        Menu {
            title: qsTr("Window")

            MenuItem {
                action: Action {
                    text: qsTr("Minimize")
                    shortcut: StandardKey.Minimize
                    onTriggered: window.showMinimized()
                }
            }
            MenuItem {
                text: window.visibility === Window.Maximized ? qsTr("Restore") : qsTr("Zoom")
                onTriggered: {
                    if (window.visibility === Window.Maximized) window.showNormal()
                    else window.showMaximized()
                }
            }
        }

        Menu {
            title: qsTr("Help")
            MenuItem { text: qsTr("About Pier-X"); onTriggered: aboutDialog.open() }
        }
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
    Shortcut {
        sequences: ["Ctrl+R", "Meta+R"]
        onActivated: commandHistoryDialog.open()
    }
    Shortcut {
        sequences: ["Ctrl+Shift+G", "Meta+Shift+G"]
        onActivated: window.toggleGitPanel()
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
    property bool gitPanelVisible: true

    // Live SSH session from the current tab's TerminalView.
    // Returns the PierTerminalSession if the active tab is a
    // terminal (local or SSH); null for SFTP/Markdown tabs.
    readonly property var activeSession: {
        if (typeof tabRepeater === "undefined" || !tabRepeater) return null
        if (currentTabIndex < 0 || currentTabIndex >= tabModel.count) return null
        var loader = tabRepeater.itemAt(currentTabIndex)
        if (!loader || !loader.item) return null
        // TerminalView exposes `terminalSession`; SFTP/Markdown don't
        return loader.item.terminalSession || null
    }

    // Shared SSH session from the active terminal tab.
    // Right-panel tools use this to avoid redundant SSH handshakes.
    readonly property var activeSharedSession: {
        if (typeof tabRepeater === "undefined" || !tabRepeater) return null
        if (currentTabIndex < 0 || currentTabIndex >= tabModel.count) return null
        var loader = tabRepeater.itemAt(currentTabIndex)
        if (!loader || !loader.item) return null
        return loader.item.sharedSshSession || null
    }
    property var pendingCloseIndexes: []
    property string pendingCloseTitle: ""
    property string pendingCloseMessage: ""
    property string pendingCloseDetail: ""
    signal writeToActiveTerminal(string text)

    // Every tabModel row carries the full schema, with unused
    // fields defaulted, so the Repeater delegate can bind to
    // model.<role> unconditionally without dealing with undefined
    // columns. ListModel doesn't grow its role set after the first
    // append, so we have to bake all of these in from row #1.
    function _makeLocalRow(title) {
        return {
            title: title,
            tabColor: -1,
            backend: "local",
            startupCommand: "",
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
            mysqlDatabase: "",
            pgHost: "",
            pgPort: 5432,
            pgUser: "",
            pgDatabase: "",
            rpTool: "",
            rightTool: "git"
        }
    }

    function _focusMethodAvailable(name) {
        const item = window.activeFocusItem
        return !!(item && typeof item[name] === "function")
    }

    function _invokeFocusMethod(name) {
        const item = window.activeFocusItem
        if (item && typeof item[name] === "function")
            item[name]()
    }

    function _makeSshRow(conn) {
        return {
            title: conn.name,
            tabColor: -1,
            backend: "ssh",
            startupCommand: "",
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
            mysqlDatabase: "",
            pgHost: "",
            pgPort: 5432,
            pgUser: "",
            pgDatabase: "",
            rpTool: "docker",
            rightTool: "monitor"
        }
    }

    // SFTP tab row — same SSH field shape as an ssh tab, just
    // with backend = "sftp" so the Repeater delegate knows to
    // load SftpBrowserView instead of TerminalView.
    function _makeSftpRow(conn) {
        return {
            title: conn.name,
            tabColor: -1,
            backend: "sftp",
            startupCommand: "",
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
            mysqlDatabase: "",
            pgHost: "",
            pgPort: 5432,
            pgUser: "",
            pgDatabase: "",
            rpTool: "",
            rightTool: "sftp"
        }
    }

    function toggleRightPanelTool(tool, context) {
        // Apply context to the tab model so the RightSidebar
        // bindings pick it up automatically.
        if (currentTabIndex >= 0 && currentTabIndex < tabModel.count && context) {
            if (context.redisHost) tabModel.setProperty(currentTabIndex, "redisHost", context.redisHost)
            if (context.redisPort) tabModel.setProperty(currentTabIndex, "redisPort", context.redisPort)
            if (context.redisDb !== undefined) tabModel.setProperty(currentTabIndex, "redisDb", context.redisDb)
            if (context.mysqlHost) tabModel.setProperty(currentTabIndex, "mysqlHost", context.mysqlHost)
            if (context.mysqlPort) tabModel.setProperty(currentTabIndex, "mysqlPort", context.mysqlPort)
            if (context.mysqlUser) tabModel.setProperty(currentTabIndex, "mysqlUser", context.mysqlUser)
            if (context.mysqlPassword) tabModel.setProperty(currentTabIndex, "mysqlPassword", context.mysqlPassword)
            if (context.logCommand) tabModel.setProperty(currentTabIndex, "logCommand", context.logCommand)
            if (context.pgHost) tabModel.setProperty(currentTabIndex, "pgHost", context.pgHost)
            if (context.pgPort) tabModel.setProperty(currentTabIndex, "pgPort", context.pgPort)
            if (context.pgUser) tabModel.setProperty(currentTabIndex, "pgUser", context.pgUser)
            if (context.pgDatabase) tabModel.setProperty(currentTabIndex, "pgDatabase", context.pgDatabase)
        }
        // Switch tool in the unified sidebar and ensure content is expanded
        if (rightSidebar.activeTool === tool && rightSidebar.contentExpanded) {
            rightSidebar.contentExpanded = false
        } else {
            rightSidebar.activeTool = tool
            rightSidebar.contentExpanded = true
        }
    }

    function toggleGitPanel() {
        rightSidebar.contentExpanded = !rightSidebar.contentExpanded
    }

    function openMarkdownTab(filePath) {
        var row = _makeLocalRow(qsTr("Preview: %1").arg(filePath.split("/").pop()))
        row.backend = "markdown"
        row.markdownPath = filePath
        tabModel.append(row)
        currentTabIndex = tabModel.count - 1
    }

    function _quoteShellPath(path) {
        const value = String(path || "")
        if (Qt.platform.os === "windows")
            return "\"" + value.replace(/`/g, "``").replace(/"/g, "`\"") + "\""
        return "'" + value.replace(/'/g, "'\\''") + "'"
    }

    function _buildLocalStartupCommand(path) {
        const target = String(path || "").trim()
        if (target.length === 0)
            return ""
        if (Qt.platform.os === "windows")
            return "Set-Location -LiteralPath " + _quoteShellPath(target) + "\r"
        return "cd -- " + _quoteShellPath(target) + "\n"
    }

    function _pathLeaf(path) {
        const value = String(path || "").replace(/[\\\/]+$/, "")
        const parts = value.split(/[\\\/]+/).filter(function(part) { return part.length > 0 })
        return parts.length > 0 ? parts[parts.length - 1] : ""
    }

    function openNewTab(title, startupCommand) {
        const t = title || qsTr("Terminal")
        const row = _makeLocalRow(t)
        row.startupCommand = startupCommand || ""
        tabModel.append(row)
        currentTabIndex = tabModel.count - 1
    }

    function openNewSessionMenu() {
        newSessionPopup.open()
    }

    function openLocalTerminalAt(path) {
        const target = String(path || "").trim()
        if (target.length === 0) {
            openNewTab()
            return
        }
        const leaf = _pathLeaf(target)
        const title = leaf.length > 0 ? leaf : qsTr("Local %1").arg(tabModel.count + 1)
        openNewTab(title, _buildLocalStartupCommand(target))
    }

    function openSshTab(conn) {
        tabModel.append(_makeSshRow(conn))
        currentTabIndex = tabModel.count - 1
    }

    function _normalizeTabIndexes(indexes) {
        const seen = {}
        const normalized = []
        for (let i = 0; i < indexes.length; ++i) {
            const index = indexes[i]
            if (index < 0 || index >= tabModel.count || seen[index])
                continue
            seen[index] = true
            normalized.push(index)
        }
        normalized.sort(function(a, b) { return a - b })
        return normalized
    }

    function _isRemoteTabRow(row) {
        if (!row)
            return false
        return row.backend !== "local" && row.backend !== "markdown"
    }

    function _remoteTabLabels(indexes) {
        const labels = []
        let remoteCount = 0
        for (let i = 0; i < indexes.length; ++i) {
            const row = tabModel.get(indexes[i])
            if (!_isRemoteTabRow(row))
                continue
            remoteCount++
            let label = row.title || qsTr("Untitled tab")
            const endpoint = (row.sshUser ? row.sshUser + "@" : "")
                             + (row.sshHost || "")
                             + (row.sshHost ? ":" + row.sshPort : "")
            if (endpoint.length > 0)
                label += "  ·  " + endpoint
            if (labels.length < 3)
                labels.push(label)
        }
        return {
            count: remoteCount,
            preview: labels
        }
    }

    function _performCloseTabs(indexes) {
        const normalized = _normalizeTabIndexes(indexes)
        if (normalized.length === 0)
            return

        let nextCurrentIndex = currentTabIndex
        for (let i = normalized.length - 1; i >= 0; --i) {
            const index = normalized[i]
            if (index < nextCurrentIndex) {
                nextCurrentIndex--
            } else if (index === nextCurrentIndex && nextCurrentIndex === tabModel.count - 1) {
                nextCurrentIndex--
            }
            tabModel.remove(index)
        }

        if (tabModel.count === 0) {
            currentTabIndex = 0
            return
        }

        currentTabIndex = Math.max(0, Math.min(nextCurrentIndex, tabModel.count - 1))
    }

    function requestCloseTabs(indexes) {
        const normalized = _normalizeTabIndexes(indexes)
        if (normalized.length === 0)
            return

        const remoteInfo = _remoteTabLabels(normalized)
        if (remoteInfo.count === 0) {
            _performCloseTabs(normalized)
            return
        }

        pendingCloseIndexes = normalized
        if (normalized.length === 1) {
            pendingCloseTitle = qsTr("Close remote tab?")
            pendingCloseMessage = qsTr("This tab is connected to a remote host. Close it anyway?")
            pendingCloseDetail = remoteInfo.preview.length > 0 ? remoteInfo.preview[0] : ""
        } else {
            pendingCloseTitle = qsTr("Close %1 tabs?").arg(normalized.length)
            pendingCloseMessage = remoteInfo.count === normalized.length
                    ? qsTr("These tabs include active remote connections. Close them anyway?")
                    : qsTr("Some of these tabs include active remote connections. Close them anyway?")
            pendingCloseDetail = qsTr("Remote tabs: %1").arg(remoteInfo.preview.join("\n"))
            if (remoteInfo.count > remoteInfo.preview.length) {
                pendingCloseDetail += "\n" + qsTr("+%1 more").arg(remoteInfo.count - remoteInfo.preview.length)
            }
        }
        remoteCloseDialog.open()
    }

    function closeTab(index) {
        requestCloseTabs([index])
    }

    function closeOtherTabs(index) {
        const indexes = []
        for (let i = 0; i < tabModel.count; ++i) {
            if (i !== index)
                indexes.push(i)
        }
        requestCloseTabs(indexes)
    }

    function closeTabsToLeft(index) {
        const indexes = []
        for (let i = 0; i < index; ++i)
            indexes.push(i)
        requestCloseTabs(indexes)
    }

    function closeTabsToRight(index) {
        const indexes = []
        for (let i = index + 1; i < tabModel.count; ++i)
            indexes.push(i)
        requestCloseTabs(indexes)
    }

    function cancelPendingTabClose() {
        pendingCloseIndexes = []
        pendingCloseTitle = ""
        pendingCloseMessage = ""
        pendingCloseDetail = ""
        remoteCloseDialog.close()
    }

    function confirmPendingTabClose() {
        const indexes = pendingCloseIndexes
        pendingCloseIndexes = []
        pendingCloseTitle = ""
        pendingCloseMessage = ""
        pendingCloseDetail = ""
        remoteCloseDialog.close()
        _performCloseTabs(indexes)
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

        // Default = password auth. Store password directly in
        // the connection config — no OS keychain dependency.
        if (!connectionsModel.addWithPassword(conn.name, conn.host, conn.port,
                                              conn.username, conn.password)) {
            console.warn("Main: failed to persist connection")
            return
        }
        openSshTab({
            name: conn.name,
            host: conn.host,
            port: conn.port,
            username: conn.username,
            password: conn.password
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
        // Direct password (preferred — no keychain dependency)
        if (conn.password && conn.password.length > 0) {
            openSshTab({
                name: conn.name,
                host: conn.host,
                port: conn.port,
                username: conn.username,
                password: conn.password
            })
            return
        }
        // Legacy: credential id in OS keychain
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
        const name = conn ? conn.name : ""
        if (conn) {
            if (conn.credentialId && conn.credentialId.length > 0) {
                PierCredentials.deleteEntry(conn.credentialId)
            }
            if (conn.passphraseCredentialId && conn.passphraseCredentialId.length > 0) {
                PierCredentials.deleteEntry(conn.passphraseCredentialId)
            }
        }
        connectionsModel.removeAt(index)
        if (name.length > 0)
            toastManager.show(qsTr("Connection %1 deleted").arg(name), "info")
    }

    // ─────────────────────────────────────────────────────
    // IDE shell
    // ─────────────────────────────────────────────────────
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TopBar {
            Layout.fillWidth: true
            contextTitle: tabModel.count > 0 && currentTabIndex >= 0 && currentTabIndex < tabModel.count
                          ? (tabModel.get(currentTabIndex).title || qsTr("Workspace"))
                          : qsTr("Workspace")
            onNewSessionRequested: window.openNewSessionMenu()
            onCommandPaletteRequested: commandPalette.show()
            onSettingsRequested: settingsDialog.show()
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            Sidebar {
                id: sidebar
                SplitView.preferredWidth: Theme.sidebarWidth
                SplitView.minimumWidth: 190
                // Use a visible property mapped to a toggled state if needed
                connectionsModel: connectionsModel
                onAddConnectionRequested: newConnectionDialog.show()
                onConnectionActivated: (i) => window.activateConnection(i)
                onConnectionDeleted: (i) => window.removeConnection(i)
                onConnectionSftpRequested: (i) => window.openSftpForConnection(i)
                onConnectionDuplicated: (i) => {
                    const c = connectionsModel.get(i)
                    if (!c) return
                    if (c.usesAgent) {
                        connectionsModel.addAgent(
                            c.name + " (copy)", c.host, c.port, c.username)
                    } else if (c.keyPath && c.keyPath.length > 0) {
                        connectionsModel.addKeyAuth(
                            c.name + " (copy)", c.host, c.port,
                            c.username, c.keyPath, "")
                    } else {
                        connectionsModel.addPassword(
                            c.name + " (copy)", c.host, c.port,
                            c.username, "")
                    }
                    toastManager.show(qsTr("Connection duplicated"), "success")
                }
                onOpenLocalTerminalRequested: (path) => {
                    if (path && path.length > 0)
                        window.openLocalTerminalAt(path)
                    else
                        window.openNewTab()
                }
                onOpenMarkdownRequested: (filePath) => window.openMarkdownTab(filePath)
            }

            // Central Area + Right Panel wrapper
            // Needs to be wrapped in an Item so WelcomeView overlays correctly without breaking SplitView
            Item {
                SplitView.minimumWidth: 620
                SplitView.fillWidth: true

                WelcomeView {
                    anchors.fill: parent
                    visible: tabModel.count === 0
                    connectionsModel: window.connectionsModel
                    onOpenLocalTerminalRequested: window.openNewTab()
                    onNewSshRequested: newConnectionDialog.show()
                    onConnectToSaved: (index) => {
                        const conn = connectionsModel.get(index)
                        if (conn) window.openSshTab(conn)
                    }
                }

                SplitView {
                    anchors.fill: parent
                    visible: tabModel.count > 0
                    orientation: Qt.Horizontal

                    ColumnLayout {
                        SplitView.minimumWidth: 560
                        SplitView.fillWidth: true
                        spacing: 0

                        TabBar {
                            Layout.fillWidth: true
                            model: tabModel
                            currentIndex: window.currentTabIndex
                            onTabClicked: (i) => window.currentTabIndex = i
                            onTabClosed: (i) => window.closeTab(i)
                            onCloseOtherTabsRequested: (i) => window.closeOtherTabs(i)
                            onCloseTabsToLeftRequested: (i) => window.closeTabsToLeft(i)
                            onCloseTabsToRightRequested: (i) => window.closeTabsToRight(i)
                            onTabColorChanged: (i, colorTag) => {
                                if (i >= 0 && i < tabModel.count)
                                    tabModel.setProperty(i, "tabColor", colorTag)
                            }
                            onNewTabClicked: window.openNewSessionMenu()
                            onTabMoved: (from, to) => {
                                tabModel.move(from, 1, to)
                                if (window.currentTabIndex === from)
                                    window.currentTabIndex = to
                                else if (from < window.currentTabIndex && to >= window.currentTabIndex)
                                    window.currentTabIndex--
                                else if (from > window.currentTabIndex && to <= window.currentTabIndex)
                                    window.currentTabIndex++
                            }
                        }

                        // One TerminalView per tab, kept alive as the user
                        // switches between them so each tab owns its own
                        // PierTerminalSession (and its own child shell).
                        // StackLayout hides inactive tabs without destroying
                        // them — matches IDE terminal behavior.
                        StackLayout {
                            id: tabContentStack
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            currentIndex: window.currentTabIndex

                            Repeater {
                                id: tabRepeater
                                model: tabModel
                                delegate: Loader {
                                    // Main tab views
                                    required property string backend
                                    required property string startupCommand
                                    required property string sshHost
                                    required property int    sshPort
                                    required property string sshUser
                                    required property string sshPassword
                                    required property string sshCredentialId
                                    required property string sshKeyPath
                                    required property string sshPassphraseCredentialId
                                    required property bool   sshUsesAgent
                                    required property string markdownPath

                                    sourceComponent: backend === "sftp"
                                                     ? sftpComp
                                                     : (backend === "markdown"
                                                        ? markdownComp
                                                        : terminalComp)

                                    Component {
                                        id: terminalComp
                                        TerminalView {
                                            backend: parent.backend
                                            startupCommand: parent.startupCommand
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
                                        id: markdownComp
                                        MarkdownPreviewView {
                                            filePath: parent.markdownPath
                                        }
                                    }
                                }
                            }
                        }
                    }

                }
            }

            // Unified Right Sidebar — permanent, hosts all tools.
            // SSH context is read from the LIVE session (not static
            // tab model) so it tracks the actual connected host.
            // activeTool is per-tab via the rightTool field.
            RightSidebar {
                id: rightSidebar
                SplitView.preferredWidth: rightSidebar.contentExpanded ? Theme.rightSidebarWidth : Theme.toolRailWidth
                SplitView.minimumWidth: rightSidebar.contentExpanded ? 340 : Theme.toolRailWidth
                SplitView.maximumWidth: rightSidebar.contentExpanded ? 99999 : Theme.toolRailWidth

                // Per-tab tool memory
                activeTool: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return "git"
                    return tabModel.get(window.currentTabIndex).rightTool || "git"
                }
                onActiveToolChanged: {
                    if (window.currentTabIndex >= 0 && window.currentTabIndex < tabModel.count)
                        tabModel.setProperty(window.currentTabIndex, "rightTool", activeTool)
                }

                // Shared SSH session for right-panel tool reuse
                sharedSession: window.activeSharedSession

                // Live SSH context from the active terminal session
                activeBackend: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).backend || ""
                }
                sshHost:     window.activeSession ? window.activeSession.sshHost     : ""
                sshPort:     window.activeSession ? window.activeSession.sshPort     : 22
                sshUser:     window.activeSession ? window.activeSession.sshUser     : ""
                sshPassword: window.activeSession ? window.activeSession.sshPassword : ""
                sshCredentialId: window.activeSession ? window.activeSession.sshCredentialId : ""
                sshKeyPath:  window.activeSession ? window.activeSession.sshKeyPath  : ""
                sshPassphraseCredentialId: window.activeSession ? window.activeSession.sshPassphraseCredentialId : ""
                sshUsesAgent: window.activeSession ? window.activeSession.sshUsesAgent : false

                // Service context still comes from tab model (set by
                // service pill clicks in TerminalView)
                redisHost: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).redisHost || ""
                }
                redisPort: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return 0
                    return tabModel.get(window.currentTabIndex).redisPort || 0
                }
                redisDb: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return 0
                    return tabModel.get(window.currentTabIndex).redisDb || 0
                }
                logCommand: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).logCommand || ""
                }
                mysqlHost: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).mysqlHost || ""
                }
                mysqlPort: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return 3306
                    return tabModel.get(window.currentTabIndex).mysqlPort || 3306
                }
                mysqlUser: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).mysqlUser || ""
                }
                mysqlPassword: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).mysqlPassword || ""
                }
                mysqlDatabase: {
                    if (window.currentTabIndex < 0 || window.currentTabIndex >= tabModel.count) return ""
                    return tabModel.get(window.currentTabIndex).mysqlDatabase || ""
                }

                onClosePanelRequested: window.toggleGitPanel()
            }
        }

        StatusBar {
            Layout.fillWidth: true
        }
    }

    Popup {
        id: newSessionPopup
        width: 344
        modal: false
        focus: true
        padding: Theme.sp1
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        x: Math.round((window.width - width) / 2)
        y: Theme.topBarHeight + Theme.sp2

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg
        }

        contentItem: ColumnLayout {
            spacing: Theme.sp1

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 34
                color: "transparent"

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp2
                    spacing: Theme.sp2

                    Text {
                        text: qsTr("New session")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightSemibold
                        color: Theme.textPrimary
                    }

                    Item { Layout.fillWidth: true }

                    Item { Layout.preferredWidth: 26 }
                }
            }

            QuickSessionItem {
                title: qsTr("New local terminal")
                subtitle: qsTr("Open a fresh local shell tab.")
                icon: "terminal"
                onClicked: {
                    newSessionPopup.close()
                    window.openNewTab()
                }
            }

            QuickSessionItem {
                title: qsTr("New SSH connection…")
                subtitle: qsTr("Create or connect to a saved remote profile.")
                icon: "server"
                onClicked: {
                    newSessionPopup.close()
                    newConnectionDialog.show()
                }
            }

            Rectangle {
                Layout.fillWidth: true
                height: 1
                color: Theme.borderSubtle
                visible: connectionsModel.count > 0
            }

            Text {
                visible: connectionsModel.count > 0
                text: qsTr("Saved Connections")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeSmall
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
                leftPadding: Theme.sp3
                topPadding: Theme.sp1
                bottomPadding: Theme.sp0_5
            }

            Repeater {
                model: Math.min(connectionsModel.count, 6)

                delegate: QuickSessionItem {
                    required property int index

                    readonly property var conn: connectionsModel.get(index)
                    title: conn ? (conn.name || conn.host || qsTr("Connection")) : ""
                    subtitle: conn ? ((conn.username || "") + "@" + (conn.host || "") + ":" + (conn.port || 22)) : ""
                    icon: "server"
                    subtitleMono: true
                    onClicked: {
                        newSessionPopup.close()
                        window.activateConnection(index)
                    }
                }
            }
        }
    }

    component QuickSessionItem: Rectangle {
        property string title: ""
        property string subtitle: ""
        property string icon: "terminal"
        property bool subtitleMono: false
        signal clicked()

        Layout.fillWidth: true
        implicitHeight: 48
        radius: Theme.radiusMd
        color: quickArea.containsMouse ? Theme.bgHover : "transparent"

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp3
            anchors.rightMargin: Theme.sp3
            spacing: Theme.sp2

            Rectangle {
                Layout.preferredWidth: 20
                Layout.preferredHeight: 20
                radius: Theme.radiusSm
                color: Theme.accentSubtle

                Image {
                    anchors.centerIn: parent
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
                    sourceSize: Qt.size(12, 12)
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        colorization: 1.0
                        colorizationColor: Theme.accent
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Text {
                    Layout.fillWidth: true
                    text: title
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                Text {
                    Layout.fillWidth: true
                    visible: subtitle.length > 0
                    text: subtitle
                    font.family: subtitleMono ? Theme.fontMono : Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }
            }
        }

        MouseArea {
            id: quickArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }
    }

    // ─────────────────────────────────────────────────────
    // Floating overlays
    // ─────────────────────────────────────────────────────
    Popup {
        id: remoteCloseDialog
        parent: Overlay.overlay
        modal: true
        focus: true
        padding: Theme.sp4
        width: Math.min(420, window.width - Theme.sp6 * 2)
        x: Math.round((window.width - width) / 2)
        y: Math.round((window.height - implicitHeight) / 2)
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        onClosed: {
            if (window.pendingCloseIndexes.length === 0)
                return
            window.pendingCloseIndexes = []
            window.pendingCloseTitle = ""
            window.pendingCloseMessage = ""
            window.pendingCloseDetail = ""
        }

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }
        }

        contentItem: ColumnLayout {
            spacing: Theme.sp4

            Text {
                Layout.fillWidth: true
                text: window.pendingCloseTitle
                wrapMode: Text.WordWrap
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeH3
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
            }

            Text {
                Layout.fillWidth: true
                text: window.pendingCloseMessage
                wrapMode: Text.WordWrap
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textSecondary
            }

            Rectangle {
                Layout.fillWidth: true
                visible: window.pendingCloseDetail.length > 0
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusSm
                implicitHeight: detailText.implicitHeight + Theme.sp3 * 2

                Text {
                    id: detailText
                    anchors.fill: parent
                    anchors.margins: Theme.sp3
                    text: window.pendingCloseDetail
                    wrapMode: Text.WrapAnywhere
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Item { Layout.fillWidth: true }

                GhostButton {
                    id: remoteCloseCancelButton
                    text: qsTr("Cancel")
                    onClicked: window.cancelPendingTabClose()
                }

                PrimaryButton {
                    text: qsTr("Close")
                    onClicked: window.confirmPendingTabClose()
                }
            }
        }

        onOpened: remoteCloseCancelButton.forceActiveFocus()
    }

    NewConnectionDialog {
        id: newConnectionDialog
        onSaved: (conn) => {
            // M3c2: store password in OS keychain under a
            // generated id, persist the connection (without
            // secrets) to disk, then open a live SSH tab that
            // reconnects via the credential id.
            window.saveAndConnect(conn)
            toastManager.show(qsTr("Connection %1 saved").arg(conn.name), "success")
        }
    }

    // Global toast notification manager
    ToastManager { id: toastManager }

    SettingsDialog {
        id: settingsDialog
        connectionsModel: connectionsModel
    }

    Popup {
        id: aboutDialog
        parent: Overlay.overlay
        modal: true
        focus: true
        padding: Theme.sp4
        width: Math.min(380, window.width - Theme.sp6 * 2)
        x: Math.round((window.width - width) / 2)
        y: Math.round((window.height - implicitHeight) / 2)
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }
        }

        contentItem: ColumnLayout {
            spacing: Theme.sp3

            Text {
                text: qsTr("Pier-X")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeH3
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
            }

            Text {
                text: qsTr("A visual operations workspace for terminals, services, and remote infrastructure.")
                wrapMode: Text.WordWrap
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textSecondary
            }

            Rectangle {
                Layout.fillWidth: true
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusSm
                implicitHeight: aboutMeta.implicitHeight + Theme.sp3 * 2

                Text {
                    id: aboutMeta
                    anchors.fill: parent
                    anchors.margins: Theme.sp3
                    text: qsTr("Version %1\nQt %2\nCore %3")
                        .arg(Qt.application.version)
                        .arg(PierCore.qtVersion)
                        .arg(PierCore.buildInfo)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                }
            }

            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }

                PrimaryButton {
                    text: qsTr("Close")
                    onClicked: aboutDialog.close()
                }
            }
        }
    }

    CommandHistoryDialog {
        id: commandHistoryDialog
        onCommandSelected: (cmd) => {
            window.writeToActiveTerminal(cmd + "\n")
        }
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
                title: qsTr("Tail syslog"),
                shortcut: "",
                action: function() {
                    window.toggleRightPanelTool("log", { logCommand: "tail -f /var/log/syslog" })
                }
            },
            {
                title: qsTr("Docker containers"),
                shortcut: "",
                action: function() {
                    window.toggleRightPanelTool("docker")
                }
            },
            {
                title: qsTr("Git panel"),
                shortcut: "Ctrl+Shift+G",
                action: function() {
                    window.toggleGitPanel()
                }
            },
            {
                title: qsTr("Open Markdown preview…"),
                shortcut: "",
                action: function() {
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
                    window.openMysqlTab("127.0.0.1", 13306, "root", "", "")
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
