import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

// NOTE for M4: the service pill strip shows at the top of
// this view (above the terminal grid) when the session is
// Connected AND we're running against an ssh backend. The
// grid is re-anchored below the strip so cell sizing picks
// up the new height.

// Live terminal view. Owns a PierTerminalSession (spawned lazily on
// first layout) and paints its grid via PierTerminalGrid.
//
// Keyboard routing:
//   * Every printable key press translates to UTF-8 bytes which we
//     forward to session.write().
//   * Control keys are translated to their VT100 equivalents (^C
//     → 0x03 etc.). This is a minimal set today — Arrow keys, Home,
//     End, Page Up/Down, Delete, and Tab are explicitly handled.
//     Everything else falls through to Keys.onPressed with its
//     event.text value.
Rectangle {
    id: root

    // Backend selector. "local" spawns a local shell via the
    // default Unix/Win PTY. "ssh" dials a remote via pier-core's
    // SSH layer and uses the remote shell as the PTY. Both paths
    // produce identical PierTerminalSession handles above the
    // M2 `Pty` trait — everything below is backend-agnostic.
    property string backend: "local"
    property string startupCommand: ""
    property bool startupCommandSent: false
    property string contextMenuUrl: ""
    property real contextMenuX: 0
    property real contextMenuY: 0
    readonly property string copyShortcut: Qt.platform.os === "osx" ? "Meta+C" : "Ctrl+Shift+C"
    readonly property string pasteShortcut: Qt.platform.os === "osx" ? "Meta+V" : "Ctrl+Shift+V"
    readonly property string selectAllShortcut: Qt.platform.os === "osx" ? "Meta+A" : "Ctrl+Shift+A"

    // Expose the live C++ handles to parent views. Keep these as `var`
    // rather than strongly-typed QML properties: Qt 6.8 on Windows can
    // assert in qqml.cpp while building property caches for these custom
    // QObject-backed types during startup, which aborts the app before
    // the first window is shown.
    readonly property var terminalSession: session

    // Shared SSH session handle — right-panel tools reuse this
    // instead of opening their own SSH connections.
    readonly property var sharedSshSession: _sharedSession
    readonly property var controlMaster: _controlMaster
    readonly property int sshStatusIdle: 0
    readonly property int sshStatusConnecting: 1
    readonly property int sshStatusConnected: 2
    readonly property int sshStatusFailed: 3
    readonly property int serviceDetectorStateIdle: 0
    readonly property int tunnelStateIdle: 0
    readonly property int tunnelStateOpening: 1
    readonly property int tunnelStateOpen: 2

    // Guard startup bindings until custom QObject ids are fully available.
    // Qt 6.8 release qmlcache on Windows can eagerly initialize lookups
    // against these ids during construction and crash if the target is
    // still null for the first evaluation pass.
    readonly property int sessionStatus: session ? session.status : root.sshStatusIdle
    readonly property bool sessionRunning: session ? session.running : false
    readonly property bool sessionCursorVisible: session ? session.cursorVisible : false
    readonly property int sessionScrollOffset: session ? session.scrollOffset : 0
    readonly property string sessionSshTarget: session ? session.sshTarget : ""
    readonly property string sessionSshErrorMessage: session ? session.sshErrorMessage : ""
    readonly property int detectorState: detector ? detector.state : root.serviceDetectorStateIdle
    readonly property int detectorCount: detector ? detector.count : 0

    // Emitted when the shared SSH session connects or disconnects.
    // Main.qml's Loader delegate listens to update the sidebar bindings.
    signal sshContextChanged()

    // Default shell is system-dependent. The caller can override this
    // via the `shell` property before the first layout; we only spawn
    // the PTY once `grid.cellWidth` is known (see startWhenSized).
    //
    // Windows keeps PowerShell as the default shell so local tabs
    // match the rest of the product, but the backend runs it without
    // loading the user's profile so embedded sessions stay on a
    // deterministic VT input path that matches Pier-X's transport.
    property string shell: Qt.platform.os === "windows"
                           ? "powershell.exe"
                           : (Qt.platform.os === "osx" ? "/bin/zsh" : "/bin/bash")

    // SSH backend parameters. Only read when `backend === "ssh"`.
    // The plaintext password (sshPassword) is consumed exactly
    // once at startup and never echoed or stored on the QML
    // object after the handshake — the C++ layer copies it into
    // a std::string for the worker closure as soon as
    // startSsh returns.
    //
    // For saved connections, the dialog sets sshCredentialId
    // instead of sshPassword. When set, TerminalView calls
    // startSshWithCredential, which goes through the
    // pier_terminal_new_ssh_credential FFI — no plaintext
    // crosses the C ABI boundary at all, the Rust SSH layer
    // pulls the password from the OS keychain by id.
    property string sshHost: ""
    property int sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""

    // M3c3: private-key auth. When sshKeyPath is non-empty
    // TerminalView dispatches startSshWithKey, which goes
    // through pier_terminal_new_ssh_key — the Rust SSH layer
    // reads the key file from disk and (if needed) pulls the
    // passphrase from the OS keychain by id at handshake time.
    // Empty sshPassphraseCredentialId means "unencrypted key".
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""

    // M3c4: SSH agent auth. When sshUsesAgent is true, the
    // session uses the system SSH agent ($SSH_AUTH_SOCK on
    // Unix / Pageant on Windows) for auth. No credentials
    // cross the FFI at all.
    property bool sshUsesAgent: false

    color: Theme.currentTerminalTheme.bg
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierSshSessionHandle {
        id: _sharedSession

        onConnectedChanged: {
            if (_sharedSession.connected && root.backend === "ssh")
                root._startTerminalOnSharedSession()
            root.sshContextChanged()

            // If SSH session open failed for a local-terminal-detected ssh,
            // fall back to ControlMaster which piggybacks on the terminal's
            // own SSH connection (no password needed).
            if (!_sharedSession.connected && !_sharedSession.busy
                && root.backend === "local"
                && session.detectedSshHost.length > 0
                && !_controlMaster.connected && !_controlMaster.busy) {
                _controlMaster.connectTo(
                    session.detectedSshHost,
                    session.detectedSshPort,
                    session.detectedSshUser)
            }
        }
    }

    PierControlMasterHandle {
        id: _controlMaster

        onConnectedChanged: root.sshContextChanged()
    }

    PierTerminalSession {
        id: session
        scrollbackLimit: Theme.scrollbackLines

        // SSH command detected in terminal output — auto-create or
        // replace the shared SSH session for right-panel tools.
        // Supports multi-hop: ssh gateway → ssh internal → ...
        // Each new ssh command replaces the previous session.
        onSshCommandDetected: {
            if (_sharedSession.busy) return

            var newHost = session.detectedSshHost
            var newPort = session.detectedSshPort
            var newUser = session.detectedSshUser
            var newTarget = newUser + "@" + newHost + ":" + newPort

            // Skip if same target as current session
            if (_sharedSession.connected && _sharedSession.target === newTarget)
                return

            // Close previous session before opening new one (multi-hop)
            if (_sharedSession.connected)
                _sharedSession.close()

            // Determine credentials — priority:
            // 1. This tab's own SSH credentials (SSH-backend tab)
            // 2. Saved connection matching host+user
            // 3. Agent auth (fallback)
            var kind = 3
            var secret = ""
            var extra = ""

            if (root.sshPassword.length > 0) {
                kind = 0; secret = root.sshPassword
            } else if (root.sshKeyPath.length > 0) {
                kind = 2; secret = root.sshKeyPath; extra = root.sshPassphraseCredentialId
            } else if (root.sshCredentialId.length > 0) {
                kind = 1; secret = root.sshCredentialId
            } else {
                // Look up saved connections for matching credentials
                var saved = window.findSavedConnection(newHost, newUser)
                if (saved) {
                    if (saved.password && saved.password.length > 0) {
                        kind = 0; secret = saved.password
                    } else if (saved.keyPath && saved.keyPath.length > 0) {
                        kind = 2; secret = saved.keyPath
                        extra = saved.passphraseCredentialId || ""
                    } else if (saved.usesAgent) {
                        kind = 3
                    }
                }
            }

            _sharedSession.open(newHost, newPort, newUser, kind, secret, extra)
        }

        // SSH exit/logout detected — close the shared session so
        // right-panel tools revert to local mode (or previous hop).
        onSshExitDetected: {
            if (_sharedSession.connected) {
                _sharedSession.close()
                root.sshContextChanged()
            }
        }

        // When the shell exits we don't tear down the view.
        onExited: {
            if (_sharedSession.connected) {
                _sharedSession.close()
                root.sshContextChanged()
            }
        }

        // M4: once the SSH handshake completes, fire off service
        // detection in the background. Local shells skip this
        // entirely (there's no remote to probe). The pill strip
        // below binds to `detector.state` and `detector.count`
        // so the QML flows naturally from there.
        onStatusChanged: {
            if (root.backend === "ssh"
                && root.sessionStatus === root.sshStatusConnected
                && root.detectorState === root.serviceDetectorStateIdle) {
                root._kickServiceDetection()
            }
            if (root.backend === "local"
                && root.sessionStatus === root.sshStatusConnected
                && !root.startupCommandSent
                && root.startupCommand.length > 0) {
                root.startupCommandSent = true
                session.write(root.startupCommand)
            }
        }
    }

    // M4: remote service discovery. Runs after the SSH
    // handshake lands (see session.onStatusChanged above) and
    // populates the pill strip at the top of the view.
    PierServiceDetector {
        id: detector
    }

    Connections {
        target: window
        function onWriteToActiveTerminal(text) {
            if (root.visible && root.sessionRunning) {
                session.write(text)
            }
        }
    }

    function _kickServiceDetection() {
        if (root.sshHost.length === 0 || root.sshUser.length === 0) return
        var kind = 0
        var secret = ""
        var extra = ""
        if (root.sshUsesAgent) {
            kind = 3
        } else if (root.sshKeyPath.length > 0) {
            kind = 2
            secret = root.sshKeyPath
            extra = root.sshPassphraseCredentialId
        } else if (root.sshCredentialId.length > 0) {
            kind = 1
            secret = root.sshCredentialId
        } else {
            kind = 0
            secret = root.sshPassword
        }
        detector.detect(root.sshHost, root.sshPort, root.sshUser,
                        kind, secret, extra)
    }

    function _startTerminalOnSharedSession() {
        if (!_sharedSession.connected
                || root.backend !== "ssh"
                || root.sessionRunning
                || grid.cellWidth <= 0
                || grid.cellHeight <= 0
                || grid.width <= 24
                || grid.height <= 24) {
            return
        }

        Qt.callLater(function() {
            if (!_sharedSession.connected
                    || root.sessionRunning
                    || grid.cellWidth <= 0
                    || grid.cellHeight <= 0
                    || grid.width <= 24
                    || grid.height <= 24) {
                return
            }
            var cols = Math.max(40, Math.floor(grid.width / grid.cellWidth))
            var rows = Math.max(12, Math.floor(grid.height / grid.cellHeight))
            session.startSshOnSession(_sharedSession, cols, rows)
        })
    }

    function _friendlySshError(message) {
        var text = String(message || "")
        if (text.indexOf("no keychain entry for credential_id=") >= 0) {
            return qsTr("Saved credentials were not found in the system keychain. Re-enter the password or update this connection profile.")
        }
        if (text.indexOf("invalid ssh config:") === 0) {
            return text.replace(/^invalid ssh config:\s*/, "")
        }
        return text
    }

    function copy() {
        var text = grid.selectedText()
        if (text.length > 0)
            PierLocalSystem.copyText(text)
    }

    function paste() {
        var text = PierLocalSystem.readText()
        if (text.length === 0 || !root.sessionRunning)
            return
        if (root.sessionScrollOffset > 0)
            session.scrollToBottom()
        session.write(text)
    }

    function selectAll() {
        grid.selectAll()
    }

    function _openExternalUrl(url) {
        if (url.length > 0)
            Qt.openUrlExternally(url)
    }

    function _showTerminalContextMenu(localX, localY, url) {
        contextMenuUrl = url
        var pos = pointerArea.mapToItem(root, localX, localY)
        contextMenuX = Math.max(Theme.sp2,
                                Math.min(root.width - terminalContextMenu.width - Theme.sp2,
                                         pos.x + Theme.sp1))
        contextMenuY = Math.max(Theme.sp2,
                                Math.min(root.height - terminalContextMenu.implicitHeight - Theme.sp2,
                                         pos.y + Theme.sp1))
        terminalContextMenu.x = contextMenuX
        terminalContextMenu.y = contextMenuY
        terminalContextMenu.open()
    }

    // Service pill strip — shown only when the SSH session is
    // Connected and the detector has at least one entry. Sits
    // above the grid and scrolls horizontally instead of letting
    // the center view collapse into unusable widths.
    Item {
        id: serviceStrip

        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: Theme.sp2
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2

        implicitHeight: root.detectorCount > 0 ? 34 : 0
        visible: root.detectorCount > 0
                 && root.backend === "ssh"
                 && root.sessionStatus === root.sshStatusConnected
        clip: true

        Behavior on implicitHeight { NumberAnimation { duration: Theme.durNormal } }

        ToolPanelSurface {
            anchors.fill: parent
            inset: true
            padding: Theme.sp1

            RowLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                Text {
                    text: qsTr("Services")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightSemibold
                    color: Theme.textSecondary
                    Layout.alignment: Qt.AlignVCenter
                }

                Rectangle {
                    width: 1
                    height: 14
                    color: Theme.borderSubtle
                    Layout.alignment: Qt.AlignVCenter
                }

                Flickable {
                    id: serviceScroller
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    contentWidth: serviceRow.width
                    contentHeight: height
                    flickableDirection: Flickable.HorizontalFlick
                    boundsBehavior: Flickable.StopAtBounds
                    interactive: contentWidth > width
                    ScrollBar.horizontal: PierScrollBar { height: 0; active: false; visible: false }

                    Row {
                        id: serviceRow
                        height: serviceScroller.height
                        spacing: Theme.sp1_5

                        Repeater {
                            model: detector

                            delegate: Rectangle {
                                id: pill
                                required property int index
                                required property string name
                                required property string version
                                required property string status
                                required property int port

                                PierTunnel {
                                    id: tunnel
                                }

                                readonly property bool tunnelable: pill.port > 0
                                                                   && pill.status === "running"
                                readonly property bool directOpenable: pill.name === "docker"
                                                                       && pill.status === "running"
                                readonly property int tunnelStatus: tunnel ? tunnel.state : root.tunnelStateIdle
                                readonly property bool tunneled: pill.tunnelStatus === root.tunnelStateOpen
                                readonly property bool opening: pill.tunnelStatus === root.tunnelStateOpening

                                implicitHeight: 22
                                implicitWidth: pillRow.implicitWidth + Theme.sp2 * 2
                                color: {
                                    if (pill.tunneled) return Theme.accentSubtle
                                    if (pillMouse.containsMouse
                                        && (pill.tunnelable || pill.directOpenable)) {
                                        return Theme.bgHover
                                    }
                                    return Theme.bgSurface
                                }
                                border.color: pill.tunneled ? Theme.borderFocus : Theme.borderSubtle
                                border.width: 1
                                radius: Theme.radiusPill

                                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                                Row {
                                    id: pillRow
                                    anchors.centerIn: parent
                                    spacing: Theme.sp1

                                    Rectangle {
                                        width: 6
                                        height: 6
                                        radius: 3
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: {
                                            if (pill.tunneled) return Theme.accent
                                            if (pill.status === "running") return Theme.statusSuccess
                                            if (pill.status === "stopped") return Theme.statusWarning
                                            return Theme.textTertiary
                                        }

                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }

                                    Text {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: pill.name + (pill.version.length > 0 ? " " + pill.version : "")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeCaption
                                        font.weight: Theme.weightMedium
                                        color: pill.tunneled ? Theme.textPrimary : Theme.textSecondary
                                        elide: Text.ElideRight
                                        width: Math.min(120, implicitWidth)

                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }

                                    Text {
                                        visible: pill.port > 0
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: pill.tunneled ? "→ :" + tunnel.localPort : ":" + pill.port
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeCaption
                                        color: pill.tunneled ? Theme.accent : Theme.textTertiary

                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }

                                    Rectangle {
                                        id: launcherChip
                                        visible: pill.directOpenable
                                                 || (pill.tunneled
                                                     && (pill.name === "redis"
                                                         || pill.name === "mysql"))
                                        anchors.verticalCenter: parent.verticalCenter
                                        implicitWidth: launcherText.implicitWidth + Theme.sp2 * 2
                                        implicitHeight: 16
                                        radius: Theme.radiusSm
                                        color: launcherMouse.containsMouse ? Theme.accent : "transparent"
                                        border.color: Theme.accent
                                        border.width: 1

                                        Behavior on color { ColorAnimation { duration: Theme.durFast } }
                                        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                                        Text {
                                            id: launcherText
                                            anchors.centerIn: parent
                                            text: qsTr("Open")
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeCaption
                                            font.weight: Theme.weightMedium
                                            color: launcherMouse.containsMouse ? Theme.bgCanvas : Theme.accent

                                            Behavior on color { ColorAnimation { duration: Theme.durFast } }
                                        }

                                        MouseArea {
                                            id: launcherMouse
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: {
                                                if (pill.name === "redis") {
                                                    window.toggleRightPanelTool("redis", {
                                                        redisHost: "127.0.0.1",
                                                        redisPort: tunnel.localPort,
                                                        redisDb: 0
                                                    })
                                                } else if (pill.name === "mysql") {
                                                    window.toggleRightPanelTool("mysql", {
                                                        mysqlHost: "127.0.0.1",
                                                        mysqlPort: tunnel.localPort,
                                                        mysqlUser: "",
                                                        mysqlPassword: "",
                                                        mysqlDatabase: ""
                                                    })
                                                } else if (pill.name === "docker") {
                                                    window.toggleRightPanelTool("docker")
                                                }
                                            }
                                        }
                                    }

                                    Image {
                                        id: tunnelLoader
                                        visible: pill.opening
                                        anchors.verticalCenter: parent.verticalCenter
                                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/loader.svg"
                                        sourceSize: Qt.size(12, 12)
                                        layer.enabled: true
                                        layer.effect: MultiEffect {
                                            colorization: 1.0
                                            colorizationColor: Theme.accent
                                        }

                                        RotationAnimation on rotation {
                                            from: 0
                                            to: 360
                                            duration: 1500
                                            loops: Animation.Infinite
                                            running: pill.opening
                                        }
                                    }
                                }

                                MouseArea {
                                    id: pillMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: pill.tunnelable ? Qt.PointingHandCursor : Qt.ArrowCursor
                                    enabled: pill.tunnelable && !pill.opening
                                    onClicked: {
                                        if (pill.tunneled) {
                                            tunnel.close()
                                        } else {
                                            var kind = 0
                                            var secret = ""
                                            var extra = ""
                                            if (root.sshUsesAgent) {
                                                kind = 3
                                            } else if (root.sshKeyPath.length > 0) {
                                                kind = 2
                                                secret = root.sshKeyPath
                                                extra = root.sshPassphraseCredentialId
                                            } else if (root.sshCredentialId.length > 0) {
                                                kind = 1
                                                secret = root.sshCredentialId
                                            } else {
                                                kind = 0
                                                secret = root.sshPassword
                                            }
                                            var localPort = 10000 + pill.port
                                            tunnel.open(root.sshHost, root.sshPort, root.sshUser,
                                                        kind, secret, extra,
                                                        localPort, "127.0.0.1", pill.port)
                                        }
                                    }
                                }

                                PierToolTip {
                                    visible: pillMouse.containsMouse && pill.tunnelable && !pill.tunneled
                                    text: qsTr("Click to open a local tunnel to :%1").arg(pill.port)
                                }
                                PierToolTip {
                                    visible: pillMouse.containsMouse && pill.directOpenable
                                    text: qsTr("Open the Docker panel for this host")
                                }
                                PierToolTip {
                                    visible: pillMouse.containsMouse && pill.tunneled
                                    text: qsTr("Forwarding localhost:%1 → :%2. Click to close.")
                                          .arg(tunnel.localPort).arg(pill.port)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Item {
        id: terminalSurface

        anchors.top: serviceStrip.visible ? serviceStrip.bottom : parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.topMargin: serviceStrip.visible ? Theme.sp2 : 0
        clip: true

        PierTerminalGrid {
            id: grid
            anchors.fill: parent

            session: session
            font: Qt.font({
                family: Theme.fontMono,
                pixelSize: Theme.terminalFontSize
            })
            defaultForeground: Theme.currentTerminalTheme.fg
            defaultBackground: Theme.currentTerminalTheme.bg
            isDarkTheme: Theme.dark
            selectionBackground: Theme.bgSelected
            linkForeground: Theme.accent
            linkHoverForeground: Theme.accentHover
            cursorStyle: Theme.cursorStyle
            cursorBlink: Theme.cursorBlink
            cursorVisible: root.sessionCursorVisible

            // Feed the 16-color ANSI palette from the selected terminal theme.
            paletteColors: {
                var ansi = Theme.currentTerminalTheme.ansi
                if (!ansi || ansi.length < 16) return []
                var colors = []
                for (var i = 0; i < 16; i++)
                    colors.push(ansi[i])
                return colors
            }

            Component.onCompleted: startWhenSized()
            onWidthChanged: startWhenSized()
            onHeightChanged: startWhenSized()

            function startWhenSized() {
                if (root.sessionRunning) return
                if (grid.cellWidth <= 0 || grid.cellHeight <= 0) return
                if (width <= 0 || height <= 0) return
                var cols = Math.max(40, Math.floor(width / grid.cellWidth))
                var rows = Math.max(12, Math.floor(height / grid.cellHeight))

                if (root.backend === "ssh") {
                    if (root.sshHost.length === 0 || root.sshUser.length === 0) {
                        console.warn("TerminalView: ssh backend needs sshHost + sshUser")
                        return
                    }
                    grid._dispatchSshConnect(cols, rows)
                } else {
                    session.start(root.shell, cols, rows)
                }
            }

            function retryIfSsh() {
                if (root.backend !== "ssh") return
                if (root.sessionStatus === root.sshStatusConnecting) return
                var cols = Math.max(40, Math.floor(width / grid.cellWidth))
                var rows = Math.max(12, Math.floor(height / grid.cellHeight))
                if (cols <= 0 || rows <= 0) return
                grid._dispatchSshConnect(cols, rows)
            }

            function _dispatchSshConnect(cols, rows) {
                var kind = 0
                var secret = ""
                var extra = ""
                if (_sharedSession.connected) {
                    root._startTerminalOnSharedSession()
                    return
                }
                if (_sharedSession.busy)
                    return
                if (root.sshUsesAgent) {
                    kind = 3
                } else if (root.sshKeyPath.length > 0) {
                    kind = 2
                    secret = root.sshKeyPath
                    extra = root.sshPassphraseCredentialId
                } else if (root.sshPassword.length > 0) {
                    kind = 0
                    secret = root.sshPassword
                } else if (root.sshCredentialId.length > 0) {
                    kind = 1
                    secret = root.sshCredentialId
                }
                _sharedSession.open(root.sshHost, root.sshPort,
                                    root.sshUser, kind, secret, extra)
            }

            MouseArea {
                id: pointerArea
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton
                hoverEnabled: true
                preventStealing: true

                property real pressX: 0
                property real pressY: 0
                property bool dragSelecting: false
                property string pressedUrl: ""
                readonly property real dragThreshold: 4

                cursorShape: dragSelecting ? Qt.IBeamCursor
                                          : (grid.hoveredUrl.length > 0 ? Qt.PointingHandCursor : Qt.IBeamCursor)

                onPressed: (mouse) => {
                    root.forceActiveFocus()
                    terminalContextMenu.close()
                    if (mouse.button === Qt.LeftButton) {
                        pressX = mouse.x
                        pressY = mouse.y
                        pressedUrl = grid.urlAt(mouse.x, mouse.y)
                        dragSelecting = false
                    } else if (mouse.button === Qt.RightButton) {
                        pressedUrl = ""
                        dragSelecting = false
                        root._showTerminalContextMenu(mouse.x, mouse.y, grid.urlAt(mouse.x, mouse.y))
                    }
                }

                onPositionChanged: (mouse) => {
                    if (mouse.buttons & Qt.LeftButton) {
                        var dx = mouse.x - pressX
                        var dy = mouse.y - pressY
                        if (!dragSelecting && Math.sqrt(dx * dx + dy * dy) >= dragThreshold) {
                            dragSelecting = true
                            grid.beginSelection(pressX, pressY)
                        }
                        if (dragSelecting) {
                            grid.updateSelection(mouse.x, mouse.y)
                            return
                        }
                    }
                    grid.updateHoveredLink(mouse.x, mouse.y)
                }

                onReleased: (mouse) => {
                    if (mouse.button === Qt.LeftButton) {
                        if (dragSelecting) {
                            grid.endSelection()
                        } else {
                            var releasedUrl = grid.urlAt(mouse.x, mouse.y)
                            if (pressedUrl.length > 0 && releasedUrl === pressedUrl) {
                                root._openExternalUrl(releasedUrl)
                            } else {
                                grid.clearSelection()
                            }
                            grid.updateHoveredLink(mouse.x, mouse.y)
                        }
                        dragSelecting = false
                        pressedUrl = ""
                    } else if (mouse.button === Qt.RightButton) {
                        grid.updateHoveredLink(mouse.x, mouse.y)
                    }
                }

                onDoubleClicked: (mouse) => {
                    if (mouse.button === Qt.LeftButton) {
                        root.forceActiveFocus()
                        grid.selectWordAt(mouse.x, mouse.y)
                        dragSelecting = false
                        pressedUrl = ""
                        mouse.accepted = true
                    }
                }

                onCanceled: {
                    if (dragSelecting)
                        grid.endSelection()
                    dragSelecting = false
                    pressedUrl = ""
                }

                onExited: {
                    if (!containsMouse)
                        grid.clearHoveredUrl()
                }

                onWheel: (wheel) => {
                    var rawDelta = wheel.angleDelta.y
                    if (rawDelta === 0 && wheel.pixelDelta.y !== 0)
                        rawDelta = wheel.pixelDelta.y * 3
                    if (rawDelta === 0)
                        return
                    var steps = Math.max(1, Math.round(Math.abs(rawDelta) / 120))
                    session.scrollBy((rawDelta > 0 ? 1 : -1) * steps * 3)
                    wheel.accepted = true
                }
            }
        }

        ToolPanelSurface {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.rightMargin: Theme.sp2
            anchors.bottomMargin: Theme.sp2
            visible: root.sessionScrollOffset > 0
            inset: true
            padding: Theme.sp1
            z: 2

            RowLayout {
                spacing: Theme.sp2

                Text {
                    text: qsTr("History +%1").arg(root.sessionScrollOffset)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textSecondary
                }

                GhostButton {
                    text: qsTr("Back to Live")
                    onClicked: session.scrollToBottom()
                }
            }
        }

        PopoverPanel {
            id: terminalContextMenu
            width: 220
            cornerRadius: Theme.radiusMd

            PierMenuItem {
                width: parent.width
                text: qsTr("Open Link")
                enabled: root.contextMenuUrl.length > 0
                onClicked: {
                    terminalContextMenu.close()
                    root._openExternalUrl(root.contextMenuUrl)
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Copy Link")
                enabled: root.contextMenuUrl.length > 0
                onClicked: {
                    terminalContextMenu.close()
                    PierLocalSystem.copyText(root.contextMenuUrl)
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
                visible: root.contextMenuUrl.length > 0
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Copy")
                enabled: grid.hasSelection
                onClicked: {
                    terminalContextMenu.close()
                    root.copy()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Paste")
                enabled: root.sessionRunning && PierLocalSystem.readText().length > 0
                onClicked: {
                    terminalContextMenu.close()
                    root.paste()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Select All")
                enabled: root.sessionRunning
                onClicked: {
                    terminalContextMenu.close()
                    root.selectAll()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Clear Selection")
                enabled: grid.hasSelection
                onClicked: {
                    terminalContextMenu.close()
                    grid.clearSelection()
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────
    // SSH Connecting / Failed overlay
    // ─────────────────────────────────────────────────────
    // Shown whenever the session is in the middle of dialing an
    // SSH host (Connecting) or has just failed to (Failed). Sits
    // on top of the grid — the grid is still there underneath,
    // blank or frozen on a previous session, which is fine
    // because we intercept all mouse + keyboard events while the
    // overlay is visible.
    //
    // Design-system compliance: every color / size / font / radius
    // goes through Theme.*. No raw hex, no magic numbers.
    Rectangle {
        id: overlay

        anchors.fill: terminalSurface
        visible: root.sessionStatus === root.sshStatusConnecting
              || root.sessionStatus === root.sshStatusFailed

        // Scrim that darkens the underlying grid slightly. Using
        // bgCanvas at ~85% opacity gives us the "modal over a
        // live view" feel without swallowing the grid colors
        // completely. `Qt.rgba` on theme tokens keeps the
        // light/dark theme handoff clean.
        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.88)

        Behavior on opacity { NumberAnimation { duration: Theme.durNormal } }

        // Block every mouse event from reaching the grid
        // underneath while the overlay is up.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            preventStealing: true
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        // Centered card — L3 elevated surface per the design skill.
        Rectangle {
            id: card

            anchors.centerIn: parent
            width: Math.min(420, parent.width - Theme.sp8 * 2)
            implicitHeight: cardColumn.implicitHeight + Theme.sp5 * 2

            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                id: cardColumn
                anchors.fill: parent
                anchors.margins: Theme.sp5
                spacing: Theme.sp3

                // Tiny section label — "CONNECTING" or "FAILED".
                SectionLabel {
                    text: root.sessionStatus === root.sshStatusConnecting
                          ? qsTr("Connecting")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                // Primary status line — the host we're dialing.
                Text {
                    text: root.sessionSshTarget.length > 0
                          ? root.sessionSshTarget
                          : qsTr("SSH session")
                    Layout.alignment: Qt.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeH3
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: card.width - Theme.sp5 * 2

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                // Spinner (Connecting) or error message (Failed).
                Loader {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignHCenter
                    Layout.topMargin: Theme.sp2

                    sourceComponent: root.sessionStatus === root.sshStatusConnecting
                                     ? spinnerComponent
                                     : errorComponent
                }

                // Buttons row — Cancel (always) + Retry (only Failed).
                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp3
                    spacing: Theme.sp2

                    Item { Layout.fillWidth: true }

                    GhostButton {
                        text: qsTr("Cancel")
                        onClicked: {
                            session.cancelSsh()
                            session.stop()
                        }
                    }

                    PrimaryButton {
                        text: qsTr("Retry")
                        visible: root.sessionStatus === root.sshStatusFailed
                        onClicked: grid.retryIfSsh()
                    }
                }
            }
        }

        // Connecting: a single rotating arc drawn with Canvas. No
        // bundled spinner glyph yet (M6 ships real icons), so the
        // Canvas gives us something that's clearly animated
        // without introducing a font dependency. IntelliJ blue
        // arc on subtle circular track.
        Component {
            id: spinnerComponent
            Item {
                implicitHeight: 40
                Canvas {
                    id: spinCanvas
                    anchors.centerIn: parent
                    width: 28
                    height: 28

                    // Named `arcAngle` rather than `rotation` to
                    // avoid shadowing QQuickItem.rotation (which
                    // Canvas inherits). The warning surfaced on
                    // first launch — renaming is the clean fix.
                    property real arcAngle: 0

                    onPaint: {
                        const ctx = getContext("2d")
                        ctx.reset()
                        const cx = width / 2
                        const cy = height / 2
                        const r = Math.min(cx, cy) - 2

                        // Track: subtle full circle.
                        ctx.beginPath()
                        ctx.arc(cx, cy, r, 0, Math.PI * 2)
                        ctx.lineWidth = 2
                        ctx.strokeStyle = Theme.borderSubtle
                        ctx.stroke()

                        // Moving arc: 270° of accent color.
                        ctx.beginPath()
                        const start = arcAngle
                        const end = arcAngle + Math.PI * 1.5
                        ctx.arc(cx, cy, r, start, end)
                        ctx.lineWidth = 2
                        ctx.lineCap = "round"
                        ctx.strokeStyle = Theme.accent
                        ctx.stroke()
                    }

                    NumberAnimation on arcAngle {
                        running: overlay.visible
                                 && root.sessionStatus === root.sshStatusConnecting
                        from: 0
                        to: Math.PI * 2
                        duration: 900
                        loops: Animation.Infinite
                    }
                    onArcAngleChanged: requestPaint()
                }
            }
        }

        // Failed: multi-line error text. Mono font because most
        // of these messages contain hostnames, ports, or russh
        // diagnostic strings.
        Component {
            id: errorComponent
            Text {
                width: card.width - Theme.sp5 * 2
                text: root.sessionSshErrorMessage.length > 0
                      ? root._friendlySshError(root.sessionSshErrorMessage)
                      : qsTr("Unknown error")
                wrapMode: Text.Wrap
                horizontalAlignment: Text.AlignHCenter
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.statusError

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }
    }

    // Keyboard handling. Forwarded to session.write() as raw UTF-8
    // bytes (or their VT100 escape sequence equivalents).
    Keys.onPressed: function (event) {
        if (Qt.platform.os === "osx") {
            if (event.matches(StandardKey.Copy)) {
                root.copy()
                event.accepted = true
                return
            }
            if (event.matches(StandardKey.Paste)) {
                root.paste()
                event.accepted = true
                return
            }
            if (event.matches(StandardKey.SelectAll)) {
                root.selectAll()
                event.accepted = true
                return
            }
        } else {
            if ((event.modifiers & Qt.ControlModifier) && (event.modifiers & Qt.ShiftModifier)) {
                if (event.key === Qt.Key_C) {
                    root.copy()
                    event.accepted = true
                    return
                }
                if (event.key === Qt.Key_V || event.key === Qt.Key_Insert) {
                    root.paste()
                    event.accepted = true
                    return
                }
                if (event.key === Qt.Key_A) {
                    root.selectAll()
                    event.accepted = true
                    return
                }
            }
            if ((event.modifiers & Qt.ShiftModifier) && event.key === Qt.Key_Insert) {
                root.paste()
                event.accepted = true
                return
            }
        }

        if (!session.running) {
            event.accepted = false
            return
        }

        var handled = true
        switch (event.key) {
        case Qt.Key_Return:
        case Qt.Key_Enter:
            session.write("\r")
            break
        case Qt.Key_Backspace:
            // ConPTY-backed Windows shells expect the terminal-style
            // DEL byte here, same as Unix PTYs. The old pipe-backed
            // fallback rendered DEL visibly, which is why Windows had
            // a temporary BS workaround before the ConPTY transport landed.
            session.write("\x7f")
            break
        case Qt.Key_Tab:
            session.write("\t")
            break
        case Qt.Key_Escape:
            session.write("\x1b")
            break
        case Qt.Key_Up:
            session.write("\x1b[A")
            break
        case Qt.Key_Down:
            session.write("\x1b[B")
            break
        case Qt.Key_Right:
            session.write("\x1b[C")
            break
        case Qt.Key_Left:
            session.write("\x1b[D")
            break
        case Qt.Key_Home:
            session.write("\x1b[H")
            break
        case Qt.Key_End:
            session.write("\x1b[F")
            break
        case Qt.Key_PageUp:
            if (event.modifiers & Qt.ShiftModifier) {
                session.scrollBy(Math.max(1, session.rows - 1))
            } else {
                session.write("\x1b[5~")
            }
            break
        case Qt.Key_PageDown:
            if (event.modifiers & Qt.ShiftModifier) {
                session.scrollBy(-Math.max(1, session.rows - 1))
            } else {
                session.write("\x1b[6~")
            }
            break
        case Qt.Key_Delete:
            session.write("\x1b[3~")
            break
        default:
            // Let Ctrl+R / Meta+R bubble up to global shortcuts for CommandHistoryDialog
            if ((event.modifiers & (Qt.ControlModifier | Qt.MetaModifier)) && event.key === Qt.Key_R) {
                handled = false
            }
            // Ctrl+letter → corresponding control character.
            else if ((event.modifiers & Qt.ControlModifier) && event.key >= Qt.Key_A && event.key <= Qt.Key_Z) {
                var code = (event.key - Qt.Key_A) + 1
                session.write(String.fromCharCode(code))
            } else if (event.text.length > 0) {
                session.write(event.text)
            } else {
                handled = false
            }
            break
        }
        event.accepted = handled
    }
}
