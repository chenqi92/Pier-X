import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier
import "../components"

// Git panel — standalone panel at the same level as RightPanel.
// Shows repository status, staged/unstaged files, commit form,
// and a diff viewer for the selected file.
Rectangle {
    id: root

    property string repoPath: ""

    signal closePanelRequested()

    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierGitClient {
        id: client
    }

    // Auto-open when repoPath changes
    onRepoPathChanged: {
        if (repoPath.length > 0) {
            client.open(repoPath)
        }
    }

    // Refresh when panel becomes visible
    onVisibleChanged: {
        if (visible && client.isGitRepo) {
            client.refresh()
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ── Header ──────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 36
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                // Branch name
                Text {
                    text: client.isGitRepo
                          ? ("\u2387 " + client.currentBranch)
                          : qsTr("Git")
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                // Ahead/behind badges
                Text {
                    visible: client.aheadCount > 0
                    text: "\u2191" + client.aheadCount
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.statusSuccess
                }
                Text {
                    visible: client.behindCount > 0
                    text: "\u2193" + client.behindCount
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.statusWarning
                }

                IconButton {
                    icon: "refresh-cw"
                    onClicked: client.refresh()
                    Layout.preferredWidth: 24
                    Layout.preferredHeight: 24
                    enabled: !client.busy
                }
            }
        }

        // ── Not a repo message ──────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: "transparent"
            visible: !client.isGitRepo && client.status !== PierGitClient.Loading

            Text {
                anchors.centerIn: parent
                text: qsTr("Not a Git repository")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textTertiary
            }
        }

        // ── Loading indicator ───────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: "transparent"
            visible: client.status === PierGitClient.Loading && !client.isGitRepo

            Text {
                anchors.centerIn: parent
                text: qsTr("Loading...")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textTertiary
            }
        }

        // ── Main content (visible when repo is open) ────
        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Vertical
            visible: client.isGitRepo

            // ── Top: File lists + commit form ────────────
            ColumnLayout {
                SplitView.fillWidth: true
                SplitView.preferredHeight: parent.height * 0.55
                SplitView.minimumHeight: 120
                spacing: 0

                // ── Staged files ────────────────────────
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: stagedHeader.height + stagedList.contentHeight + Theme.sp1
                    Layout.maximumHeight: 200
                    color: "transparent"
                    visible: client.stagedFiles.length > 0

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 0

                        // Section header
                        Rectangle {
                            id: stagedHeader
                            Layout.fillWidth: true
                            implicitHeight: 24
                            color: Theme.bgHover

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2

                                Text {
                                    text: qsTr("Staged Changes (%1)").arg(client.stagedFiles.length)
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    font.weight: Theme.weightSemibold
                                    color: Theme.textSecondary
                                    Layout.fillWidth: true
                                }

                                Text {
                                    text: "\u2212"
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textTertiary
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: client.unstageAll()
                                    }
                                }
                            }
                        }

                        // Staged file list
                        ListView {
                            id: stagedList
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: client.stagedFiles

                            delegate: Rectangle {
                                required property var modelData
                                required property int index
                                width: stagedList.width
                                height: 24
                                color: fileMouseStaged.containsMouse ? Theme.bgHover : "transparent"

                                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                MouseArea {
                                    id: fileMouseStaged
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: client.loadDiff(modelData.path, true)
                                }

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp3
                                    anchors.rightMargin: Theme.sp2
                                    spacing: Theme.sp2

                                    // Status badge
                                    Rectangle {
                                        width: 16; height: 16
                                        radius: Theme.radiusXs
                                        color: statusColor(modelData.status)
                                        Text {
                                            anchors.centerIn: parent
                                            text: modelData.status
                                            font.family: Theme.fontMono
                                            font.pixelSize: 9
                                            font.weight: Theme.weightSemibold
                                            color: "#ffffff"
                                        }
                                    }

                                    Text {
                                        text: modelData.path
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textPrimary
                                        elide: Text.ElideMiddle
                                        Layout.fillWidth: true
                                    }

                                    // Unstage button
                                    Text {
                                        text: "\u2212"
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textTertiary
                                        visible: fileMouseStaged.containsMouse
                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: client.unstageFile(modelData.path)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Unstaged files ──────────────────────
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumHeight: 60
                    color: "transparent"

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 0

                        // Section header
                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 24
                            color: Theme.bgHover

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2

                                Text {
                                    text: qsTr("Changes (%1)").arg(client.unstagedFiles.length)
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    font.weight: Theme.weightSemibold
                                    color: Theme.textSecondary
                                    Layout.fillWidth: true
                                }

                                Text {
                                    text: "+"
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textTertiary
                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: client.stageAll()
                                    }
                                }
                            }
                        }

                        // Unstaged file list
                        ListView {
                            id: unstagedList
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: client.unstagedFiles

                            delegate: Rectangle {
                                required property var modelData
                                required property int index
                                width: unstagedList.width
                                height: 24
                                color: fileMouseUnstaged.containsMouse ? Theme.bgHover : "transparent"

                                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                MouseArea {
                                    id: fileMouseUnstaged
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    onClicked: client.loadDiff(modelData.path, false)
                                }

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp3
                                    anchors.rightMargin: Theme.sp2
                                    spacing: Theme.sp2

                                    Rectangle {
                                        width: 16; height: 16
                                        radius: Theme.radiusXs
                                        color: statusColor(modelData.status)
                                        Text {
                                            anchors.centerIn: parent
                                            text: modelData.status
                                            font.family: Theme.fontMono
                                            font.pixelSize: 9
                                            font.weight: Theme.weightSemibold
                                            color: "#ffffff"
                                        }
                                    }

                                    Text {
                                        text: modelData.path
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textPrimary
                                        elide: Text.ElideMiddle
                                        Layout.fillWidth: true
                                    }

                                    // Stage button
                                    Text {
                                        text: "+"
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textTertiary
                                        visible: fileMouseUnstaged.containsMouse
                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: client.stageFile(modelData.path)
                                        }
                                    }

                                    // Discard button
                                    Text {
                                        text: "\u2715"
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.statusError
                                        visible: fileMouseUnstaged.containsMouse && modelData.status !== "?"
                                        MouseArea {
                                            anchors.fill: parent
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: client.discardFile(modelData.path)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Commit form ─────────────────────────
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: commitLayout.implicitHeight + Theme.sp2 * 2
                    color: Theme.bgSurface
                    border.color: Theme.borderSubtle
                    border.width: 1

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                    ColumnLayout {
                        id: commitLayout
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        TextArea {
                            id: commitMessage
                            Layout.fillWidth: true
                            Layout.preferredHeight: 48
                            placeholderText: qsTr("Commit message...")
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textPrimary
                            placeholderTextColor: Theme.textTertiary
                            wrapMode: TextEdit.Wrap
                            background: Rectangle {
                                color: Theme.bgPanel
                                radius: Theme.radiusSm
                                border.color: commitMessage.activeFocus ? Theme.borderFocus : Theme.borderDefault
                                border.width: 1
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp1

                            PrimaryButton {
                                text: qsTr("Commit")
                                enabled: commitMessage.text.length > 0
                                         && client.stagedFiles.length > 0
                                         && !client.busy
                                onClicked: {
                                    client.commit(commitMessage.text)
                                    commitMessage.text = ""
                                }
                            }

                            GhostButton {
                                text: qsTr("Push")
                                enabled: client.aheadCount > 0 && !client.busy
                                onClicked: client.push()
                            }

                            GhostButton {
                                text: qsTr("Pull")
                                enabled: client.behindCount > 0 && !client.busy
                                onClicked: client.pull()
                            }

                            Item { Layout.fillWidth: true }
                        }
                    }
                }
            }

            // ── Bottom: Diff viewer ─────────────────────
            Rectangle {
                SplitView.fillWidth: true
                SplitView.preferredHeight: parent.height * 0.45
                SplitView.minimumHeight: 60
                color: Theme.bgSurface

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0

                    // Diff header
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 24
                        color: Theme.bgHover
                        visible: client.diffPath.length > 0

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.left: parent.left
                            anchors.leftMargin: Theme.sp2
                            text: client.diffPath
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textSecondary
                            elide: Text.ElideMiddle
                        }
                    }

                    // Diff content — syntax-highlighted text
                    ScrollView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true

                        TextArea {
                            id: diffView
                            readOnly: true
                            text: client.diffText.length > 0
                                  ? client.diffText
                                  : qsTr("Select a file to view diff")
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textPrimary
                            wrapMode: TextEdit.NoWrap
                            textFormat: TextEdit.RichText

                            // Convert raw diff to colored HTML
                            property string rawDiff: client.diffText
                            onRawDiffChanged: {
                                if (rawDiff.length === 0) {
                                    text = qsTr("Select a file to view diff")
                                    return
                                }
                                text = colorizeDiff(rawDiff)
                            }

                            background: Rectangle {
                                color: "transparent"
                            }
                        }
                    }

                    // Empty state
                    Text {
                        Layout.alignment: Qt.AlignHCenter
                        visible: client.diffText.length === 0
                                 && client.stagedFiles.length === 0
                                 && client.unstagedFiles.length === 0
                        text: qsTr("Working tree clean")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textTertiary
                        topPadding: Theme.sp6
                    }
                }
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────

    function statusColor(code) {
        switch (code) {
        case "M": return "#4f8aff"   // Modified — blue
        case "A": return "#5fb865"   // Added — green
        case "D": return "#fa6675"   // Deleted — red
        case "R": return "#c77dff"   // Renamed — purple
        case "?": return "#868a91"   // Untracked — gray
        case "U": return "#f0a83a"   // Conflicted — orange
        case "C": return "#4f8aff"   // Copied — blue
        default:  return "#868a91"
        }
    }

    function colorizeDiff(raw) {
        // Escape HTML entities first
        var escaped = raw
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")

        var lines = escaped.split("\n")
        var html = "<pre style='margin:0;'>"
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i]
            if (line.startsWith("+++") || line.startsWith("---")) {
                html += "<span style='color:" + Theme.textSecondary + ";'>" + line + "</span>\n"
            } else if (line.startsWith("@@")) {
                html += "<span style='color:#c77dff;'>" + line + "</span>\n"
            } else if (line.startsWith("+")) {
                html += "<span style='color:#5fb865;'>" + line + "</span>\n"
            } else if (line.startsWith("-")) {
                html += "<span style='color:#fa6675;'>" + line + "</span>\n"
            } else {
                html += "<span style='color:" + Theme.textPrimary + ";'>" + line + "</span>\n"
            }
        }
        html += "</pre>"
        return html
    }
}
