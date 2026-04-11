import QtQuick
import QtQuick.Layouts
import Pier

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

    // Default shell is system-dependent. The caller can override this
    // via the `shell` property before the first layout; we only spawn
    // the PTY once `grid.cellWidth` is known (see startWhenSized).
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

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierTerminalSession {
        id: session

        // When the shell exits we don't tear down the view — let the
        // user see the final state. A future iteration can surface
        // an "exited (code N)" banner and offer a Restart button.
        onExited: {
            // no-op for now; grid stays visible frozen.
        }

        // M4: once the SSH handshake completes, fire off service
        // detection in the background. Local shells skip this
        // entirely (there's no remote to probe). The pill strip
        // below binds to `detector.state` and `detector.count`
        // so the QML flows naturally from there.
        onStatusChanged: {
            if (root.backend === "ssh"
                && session.status === PierTerminalSession.Connected
                && detector.state === PierServiceDetector.Idle) {
                root._kickServiceDetection()
            }
        }
    }

    // M4: remote service discovery. Runs after the SSH
    // handshake lands (see session.onStatusChanged above) and
    // populates the pill strip at the top of the view.
    PierServiceDetector {
        id: detector
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

    // Service pill strip — shown only when the SSH session is
    // Connected and the detector has at least one entry. Sits
    // above the grid, pushes the grid down by (pill height +
    // padding). Empty / loading state is invisible; we don't
    // want to eat vertical space while detection is in flight.
    Rectangle {
        id: serviceStrip

        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.topMargin: Theme.sp3
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3

        implicitHeight: detector.count > 0 ? 28 : 0
        visible: detector.count > 0
                 && root.backend === "ssh"
                 && session.status === PierTerminalSession.Connected
        color: "transparent"

        Behavior on implicitHeight { NumberAnimation { duration: Theme.durNormal } }

        RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            Repeater {
                model: detector

                delegate: Rectangle {
                    required property int index
                    required property string name
                    required property string version
                    required property string status
                    required property int port

                    implicitHeight: 22
                    implicitWidth: pillRow.implicitWidth + Theme.sp3 * 2
                    color: Theme.bgSurface
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusPill

                    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
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
                                if (status === "running") return Theme.statusSuccess
                                if (status === "stopped") return Theme.statusWarning
                                return Theme.textTertiary
                            }

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: name + (version.length > 0
                                          ? " " + version
                                          : "")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            font.weight: Theme.weightMedium
                            color: Theme.textSecondary

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        Text {
                            visible: port > 0
                            anchors.verticalCenter: parent.verticalCenter
                            text: ":" + port
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }
                    }
                }
            }
        }
    }

    PierTerminalGrid {
        id: grid
        // Anchor below the service strip so the grid shrinks
        // to make room when services are detected. When the
        // strip is hidden its implicitHeight is 0, so
        // `serviceStrip.bottom` sits at parent.top +
        // Theme.sp3 (the strip's own top margin), and the
        // grid fills the same space it always has.
        anchors.top: serviceStrip.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.topMargin: serviceStrip.visible ? Theme.sp2 : Theme.sp3
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        anchors.bottomMargin: Theme.sp3

        session: session
        font.family: Theme.fontMono
        font.pixelSize: Theme.sizeBody
        defaultForeground: Theme.textPrimary
        defaultBackground: "transparent"

        // Kick off the shell on first layout when we actually know
        // how many cell columns/rows fit. Doing it earlier would
        // spawn the shell at a bogus size and then immediately
        // resize it, which some TUI apps dislike.
        Component.onCompleted: startWhenSized()
        onWidthChanged: startWhenSized()
        onHeightChanged: startWhenSized()

        function startWhenSized() {
            if (session.running) return
            if (grid.cellWidth <= 0 || grid.cellHeight <= 0) return
            if (width <= 0 || height <= 0) return
            var cols = Math.max(1, Math.floor(width / grid.cellWidth))
            var rows = Math.max(1, Math.floor(height / grid.cellHeight))

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
            if (session.status === PierTerminalSession.Connecting) return
            var cols = Math.max(1, Math.floor(width / grid.cellWidth))
            var rows = Math.max(1, Math.floor(height / grid.cellHeight))
            if (cols <= 0 || rows <= 0) return
            grid._dispatchSshConnect(cols, rows)
        }

        // Pick the right startSsh* method given which auth
        // fields are populated. Priority:
        //   1. usesAgent       → startSshWithAgent
        //   2. key path        → startSshWithKey
        //   3. credential id   → startSshWithCredential
        //   4. plaintext password → startSsh
        // The four are mutually exclusive in practice — the
        // dialog / Main.qml only ever populates one set per tab.
        function _dispatchSshConnect(cols, rows) {
            if (root.sshUsesAgent) {
                session.startSshWithAgent(
                    root.sshHost, root.sshPort, root.sshUser,
                    cols, rows)
            } else if (root.sshKeyPath.length > 0) {
                session.startSshWithKey(
                    root.sshHost, root.sshPort, root.sshUser,
                    root.sshKeyPath,
                    root.sshPassphraseCredentialId,
                    cols, rows)
            } else if (root.sshCredentialId.length > 0) {
                session.startSshWithCredential(
                    root.sshHost, root.sshPort, root.sshUser,
                    root.sshCredentialId,
                    cols, rows)
            } else {
                session.startSsh(
                    root.sshHost, root.sshPort, root.sshUser,
                    root.sshPassword,
                    cols, rows)
            }
        }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            // Clicking the grid gives keyboard focus back to the
            // root so the Keys handler receives events again.
            onClicked: root.forceActiveFocus()
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

        anchors.fill: parent
        visible: session.status === PierTerminalSession.Connecting
              || session.status === PierTerminalSession.Failed

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
                    text: session.status === PierTerminalSession.Connecting
                          ? qsTr("Connecting")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                // Primary status line — the host we're dialing.
                Text {
                    text: session.sshTarget.length > 0
                          ? session.sshTarget
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

                    sourceComponent: session.status === PierTerminalSession.Connecting
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
                        visible: session.status === PierTerminalSession.Failed
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
                                 && session.status === PierTerminalSession.Connecting
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
                text: session.sshErrorMessage.length > 0
                      ? session.sshErrorMessage
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
            // ^? is what most terminals send on backspace
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
            session.write("\x1b[5~")
            break
        case Qt.Key_PageDown:
            session.write("\x1b[6~")
            break
        case Qt.Key_Delete:
            session.write("\x1b[3~")
            break
        default:
            // Ctrl+letter → corresponding control character.
            if ((event.modifiers & Qt.ControlModifier) && event.key >= Qt.Key_A && event.key <= Qt.Key_Z) {
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
