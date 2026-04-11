import QtQuick
import Pier

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
    // These are consumed exactly once at startup and never echoed
    // or stored on the QML object after the handshake — the C++
    // layer zeroes the QByteArray it passed into the FFI as soon
    // as pier_terminal_new_ssh returns.
    property string sshHost: ""
    property int sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""

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
    }

    PierTerminalGrid {
        id: grid
        anchors.fill: parent
        anchors.margins: Theme.sp3

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
                // Blocks the main thread for the full handshake —
                // see PierTerminalSession::startSsh for the caveats.
                // M3c moves this to a worker thread with a
                // "Connecting..." overlay.
                session.startSsh(
                    root.sshHost,
                    root.sshPort,
                    root.sshUser,
                    root.sshPassword,
                    cols, rows)
            } else {
                session.start(root.shell, cols, rows)
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
