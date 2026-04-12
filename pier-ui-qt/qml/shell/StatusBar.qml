import QtQuick
import QtQuick.Layouts
import Pier

// Bottom status bar — short status text + version label.
// When the performance overlay is enabled (Settings → Developer),
// shows live FPS and memory usage between the status and version.
Rectangle {
    id: root

    implicitHeight: 24
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp3
        spacing: Theme.sp3

        Text {
            text: qsTr("Ready")
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        // Startup time — shown once, only when profiling is on.
        Text {
            visible: PierProfiler.enabled && PierProfiler.startupMs > 0
            text: qsTr("startup %1 ms").arg(PierProfiler.startupMs)
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary
            opacity: 0.7

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Item { Layout.fillWidth: true }

        // ── Performance overlay ──────────────────────
        Text {
            visible: PierProfiler.enabled
            text: PierProfiler.fps + " FPS"
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            font.weight: Theme.weightMedium
            color: PierProfiler.fps < 30
                   ? Theme.statusWarning
                   : (PierProfiler.fps < 50
                      ? Theme.statusInfo
                      : Theme.statusSuccess)

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            visible: PierProfiler.enabled
            text: PierProfiler.memoryUsage
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        // ── Version info ─────────────────────────────
        Text {
            text: qsTr("Qt") + " " + PierCore.qtVersion
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }

        Text {
            // App version (from CMake) · pier-core build info (from Rust FFI).
            // The "·" separator groups them visually without a second Text.
            text: "v" + Qt.application.version + " · core " + PierCore.buildInfo
            font.family: Theme.fontMono
            font.pixelSize: Theme.sizeCaption
            color: Theme.textTertiary

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
        }
    }

    // Top 1px border
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }
}
