import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier
import "../components"

// Local Markdown preview — M5e per-service tool.
//
// Layout (preview-only):
//   ┌───────────────────────────────────────────────────┐
//   │ README.md                    [Source] [↻ Reload]  │  top bar
//   ├───────────────────────────────────────────────────┤
//   │                                                   │
//   │  # Heading                                        │
//   │  rendered markdown as rich text                   │
//   │  - bullet                                         │
//   │                                                   │
//   └───────────────────────────────────────────────────┘
//
// Layout (split):
//   ┌───────────────────────────────────────────────────┐
//   │ README.md                    [Source ✓] [↻]       │
//   ├────────────────────┬──────────────────────────────┤
//   │ # Heading          │ Heading                      │
//   │ rendered here      │ rendered here                │
//   │ - bullet           │ • bullet                     │
//   └────────────────────┴──────────────────────────────┘
//
// The view is stateless: it calls PierMarkdown.loadSource(path)
// and PierMarkdown.loadHtml(path) once on load + reload, and
// stores the results in local properties. No QObject model,
// no workers — markdown rendering is fast enough to run on
// the main thread.
Rectangle {
    id: root

    // Absolute path to the .md file to render. Set by
    // Main.qml when the tab is created.
    property string filePath: ""

    // Internal state: raw source + rendered HTML + error flag.
    property string markdownSource: ""
    property string markdownHtml: ""
    property bool   loadFailed: false
    property bool   showSource: false

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    Component.onCompleted: _reload()

    onFilePathChanged: _reload()

    function _reload() {
        if (root.filePath.length === 0) {
            root.markdownSource = ""
            root.markdownHtml = ""
            root.loadFailed = false
            return
        }
        const html = PierMarkdown.loadHtml(root.filePath)
        const src  = PierMarkdown.loadSource(root.filePath)
        root.loadFailed = (html.length === 0 && src.length === 0)
        root.markdownSource = src
        root.markdownHtml = html
    }

    function _basename(path) {
        var i = path.lastIndexOf("/")
        if (i < 0) i = path.lastIndexOf("\\")
        return i >= 0 ? path.slice(i + 1) : path
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // ─── Top bar ─────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Text {
                text: root.filePath.length > 0
                      ? _basename(root.filePath)
                      : qsTr("Markdown")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 120
                Layout.maximumWidth: 320

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            Text {
                Layout.fillWidth: true
                text: root.filePath
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary
                elide: Text.ElideLeft
                horizontalAlignment: Text.AlignRight

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            GhostButton {
                compact: true
                minimumWidth: 0
                text: root.showSource ? qsTr("Source ✓") : qsTr("Source")
                onClicked: root.showSource = !root.showSource
            }

            GhostButton {
                compact: true
                minimumWidth: 0
                text: qsTr("↻ Reload")
                onClicked: root._reload()
            }
        }

        // ─── Content area ────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.bgPanel
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            // Load error overlay.
            Text {
                anchors.centerIn: parent
                visible: root.loadFailed
                text: root.filePath.length > 0
                      ? qsTr("Failed to load %1").arg(_basename(root.filePath))
                      : qsTr("No file selected")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.statusError
            }

            // Split source + preview.
            RowLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp1
                spacing: Theme.sp2
                visible: root.showSource && !root.loadFailed

                // Source pane.
                Rectangle {
                    Layout.preferredWidth: parent.width / 2
                    Layout.fillHeight: true
                    color: "transparent"
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    PierScrollView {
                        anchors.fill: parent
                        clip: true

                        PierTextArea {
                            readOnly: true
                            frameVisible: false
                            mono: true
                            wrapMode: TextArea.NoWrap
                            text: root.markdownSource
                            font.pixelSize: Theme.sizeCaption
                            selectByMouse: true
                        }
                    }
                }

                // Preview pane (split).
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: "transparent"
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    PierScrollView {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        clip: true

                        PierTextArea {
                            readOnly: true
                            frameVisible: false
                            wrapMode: TextArea.Wrap
                            textFormat: TextEdit.RichText
                            text: root.markdownHtml
                            font.pixelSize: Theme.sizeBody
                            selectByMouse: true
                        }
                    }
                }
            }

            // Preview-only pane.
            PierScrollView {
                anchors.fill: parent
                anchors.margins: Theme.sp3
                clip: true
                visible: !root.showSource && !root.loadFailed

                PierTextArea {
                    readOnly: true
                    frameVisible: false
                    wrapMode: TextArea.Wrap
                    textFormat: TextEdit.RichText
                    text: root.markdownHtml
                    font.pixelSize: Theme.sizeBody
                    selectByMouse: true
                }
            }
        }

        // ─── Footer: char count ─────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 20
            color: "transparent"

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                text: root.markdownSource.length > 0
                      ? qsTr("%1 chars").arg(root.markdownSource.length)
                      : ""
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }
    }
}
