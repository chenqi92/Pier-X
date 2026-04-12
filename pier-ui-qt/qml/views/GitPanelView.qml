import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Pier
import "../components"

// Git panel — matches the original Pier layout:
// Branch bar → Tab bar (Changes/History/Stash/Conflicts) → Content → Diff
Rectangle {
    id: root

    property string repoPath: ""
    property int selectedTab: 0

    signal closePanelRequested()

    color: Theme.bgPanel
    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierGitClient { id: client }

    onRepoPathChanged: {
        if (repoPath.length > 0) client.open(repoPath)
    }
    onVisibleChanged: {
        if (visible && client.isGitRepo) client.refresh()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ── Branch bar ──────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 32
            color: Theme.bgSurface
            visible: client.isGitRepo

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp1

                // Branch selector
                Rectangle {
                    implicitWidth: branchRow.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 22
                    radius: Theme.radiusSm
                    color: branchMouse.containsMouse ? Theme.bgHover : "transparent"

                    Row {
                        id: branchRow
                        anchors.centerIn: parent
                        spacing: Theme.sp1

                        Text {
                            text: client.currentBranch
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeSmall
                            font.weight: Theme.weightSemibold
                            color: Theme.textPrimary
                        }
                        Text {
                            text: "\u25BE"
                            font.pixelSize: 8
                            color: Theme.textTertiary
                            anchors.verticalCenter: parent.verticalCenter
                        }
                    }

                    MouseArea {
                        id: branchMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            client.loadBranches()
                            branchMenu.open()
                        }
                    }

                    Menu {
                        id: branchMenu
                        Repeater {
                            model: client.branches
                            MenuItem {
                                text: modelData
                                checkable: true
                                checked: modelData === client.currentBranch
                                onTriggered: client.checkoutBranch(modelData)
                            }
                        }
                    }
                }

                // Tracking
                Text {
                    visible: client.trackingBranch.length > 0
                    text: "\u2192 " + client.trackingBranch
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Item { Layout.fillWidth: true; visible: client.trackingBranch.length === 0 }

                // Ahead/behind
                Text {
                    visible: client.aheadCount > 0
                    text: "\u2191" + client.aheadCount
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.statusInfo
                }
                Text {
                    visible: client.behindCount > 0
                    text: "\u2193" + client.behindCount
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.statusWarning
                }

                // Pull / Push / Refresh
                IconButton {
                    icon: "refresh-cw"
                    tooltip: qsTr("Refresh")
                    onClicked: {
                        client.refresh()
                        if (selectedTab === 1) client.loadHistory()
                        if (selectedTab === 2) client.loadStashes()
                    }
                    Layout.preferredWidth: 22; Layout.preferredHeight: 22
                    enabled: !client.busy
                }
            }

            Rectangle {
                anchors.bottom: parent.bottom; width: parent.width; height: 1
                color: Theme.borderSubtle
            }
        }

        // ── Tab bar ─────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 28
            color: Theme.bgPanel
            visible: client.isGitRepo

            Row {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                spacing: 0

                Repeater {
                    model: [
                        { label: qsTr("Changes"), idx: 0 },
                        { label: qsTr("History"), idx: 1 },
                        { label: qsTr("Stash"),   idx: 2 },
                        { label: qsTr("Conflicts"), idx: 3 }
                    ]
                    delegate: Rectangle {
                        required property var modelData
                        width: tabLabel.implicitWidth + Theme.sp3 * 2
                        height: 28
                        color: selectedTab === modelData.idx
                               ? Theme.accentMuted
                               : tabArea.containsMouse ? Theme.bgHover : "transparent"
                        radius: Theme.radiusSm

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        Text {
                            id: tabLabel
                            anchors.centerIn: parent
                            text: modelData.label
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            font.weight: selectedTab === modelData.idx ? Theme.weightSemibold : Theme.weightRegular
                            color: selectedTab === modelData.idx ? Theme.textPrimary : Theme.textSecondary
                        }

                        MouseArea {
                            id: tabArea
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                selectedTab = modelData.idx
                                if (modelData.idx === 1 && client.commits.length === 0) client.loadHistory()
                                if (modelData.idx === 2) client.loadStashes()
                            }
                        }
                    }
                }
            }

            Rectangle {
                anchors.bottom: parent.bottom; width: parent.width; height: 1
                color: Theme.borderSubtle
            }
        }

        // ── Not a repo / Loading ────────────────────────
        Item {
            Layout.fillWidth: true; Layout.fillHeight: true
            visible: !client.isGitRepo

            Text {
                anchors.centerIn: parent
                text: client.status === PierGitClient.Loading ? qsTr("Loading...") : qsTr("Not a Git repository")
                font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody; color: Theme.textTertiary
            }
        }

        // ── Tab content ─────────────────────────────────
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: client.isGitRepo
            currentIndex: selectedTab

            // ═══ TAB 0: Changes ═════════════════════════
            SplitView {
                orientation: Qt.Vertical

                ColumnLayout {
                    SplitView.fillWidth: true
                    SplitView.preferredHeight: 300
                    SplitView.minimumHeight: 100
                    spacing: 0

                    // Staged section
                    FileSection {
                        Layout.fillWidth: true
                        Layout.maximumHeight: 200
                        visible: client.stagedFiles.length > 0
                        title: qsTr("Staged (%1)").arg(client.stagedFiles.length)
                        dotColor: Theme.statusSuccess
                        actionLabel: qsTr("Unstage All")
                        onActionClicked: client.unstageAll()
                        model: client.stagedFiles
                        staged: true
                        onFileClicked: (path) => client.loadDiff(path, true)
                        onFileAction: (path) => client.unstageFile(path)
                    }

                    // Unstaged section
                    FileSection {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        title: qsTr("Unstaged (%1)").arg(client.unstagedFiles.length)
                        dotColor: Theme.statusWarning
                        actionLabel: qsTr("Stage All")
                        onActionClicked: client.stageAll()
                        model: client.unstagedFiles
                        staged: false
                        onFileClicked: (path) => client.loadDiff(path, false)
                        onFileAction: (path) => client.stageFile(path)
                        onFileDiscard: (path) => client.discardFile(path)
                    }

                    // Commit form
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: commitCol.implicitHeight + Theme.sp2 * 2
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle; border.width: 1

                        ColumnLayout {
                            id: commitCol
                            anchors.fill: parent; anchors.margins: Theme.sp2
                            spacing: Theme.sp1

                            TextArea {
                                id: commitMsg
                                Layout.fillWidth: true
                                Layout.preferredHeight: 48
                                placeholderText: qsTr("Commit message...")
                                font.family: Theme.fontMono; font.pixelSize: Theme.sizeSmall
                                color: Theme.textPrimary; placeholderTextColor: Theme.textTertiary
                                wrapMode: TextEdit.Wrap
                                background: Rectangle {
                                    color: Theme.bgPanel; radius: Theme.radiusSm
                                    border.color: commitMsg.activeFocus ? Theme.borderFocus : Theme.borderDefault
                                    border.width: 1
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true; spacing: Theme.sp1

                                GhostButton {
                                    text: qsTr("Stage All")
                                    visible: client.unstagedFiles.length > 0
                                    onClicked: client.stageAll()
                                }
                                Item { Layout.fillWidth: true }

                                PrimaryButton {
                                    text: qsTr("Commit")
                                    enabled: commitMsg.text.length > 0 && client.stagedFiles.length > 0 && !client.busy
                                    onClicked: { client.commit(commitMsg.text); commitMsg.text = "" }
                                }
                                GhostButton {
                                    text: qsTr("Push"); enabled: client.aheadCount > 0 && !client.busy
                                    onClicked: client.push()
                                }
                                GhostButton {
                                    text: qsTr("Pull"); enabled: client.behindCount > 0 && !client.busy
                                    onClicked: client.pull()
                                }
                            }
                        }
                    }
                }

                // Diff viewer
                DiffViewer {
                    SplitView.fillWidth: true
                    SplitView.preferredHeight: 200
                    SplitView.minimumHeight: 60
                    diffPath: client.diffPath
                    diffText: client.diffText
                }
            }

            // ═══ TAB 1: History ═════════════════════════
            ColumnLayout {
                spacing: 0

                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: client.commits

                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: ListView.view.width
                        height: 36
                        color: histMouse.containsMouse ? Theme.bgHover : "transparent"

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        MouseArea {
                            id: histMouse; anchors.fill: parent; hoverEnabled: true
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2; anchors.rightMargin: Theme.sp2
                            spacing: Theme.sp2

                            // Hash
                            Text {
                                text: modelData.shortHash
                                font.family: Theme.fontMono; font.pixelSize: Theme.sizeSmall
                                color: Theme.accent
                                Layout.preferredWidth: 60
                            }

                            // Refs badge
                            Rectangle {
                                visible: modelData.refs.length > 0
                                implicitWidth: refText.implicitWidth + Theme.sp1 * 2
                                implicitHeight: 16
                                radius: Theme.radiusPill
                                color: Theme.accentMuted

                                Text {
                                    id: refText; anchors.centerIn: parent
                                    text: modelData.refs.split(",")[0].trim()
                                    font.family: Theme.fontMono; font.pixelSize: 9
                                    color: Theme.accent
                                }
                            }

                            // Message
                            Text {
                                text: modelData.message
                                font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            // Author
                            Text {
                                text: modelData.author
                                font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                Layout.preferredWidth: 80
                                elide: Text.ElideRight
                            }

                            // Date
                            Text {
                                text: modelData.relativeDate
                                font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                Layout.preferredWidth: 80
                                horizontalAlignment: Text.AlignRight
                            }
                        }
                    }
                }
            }

            // ═══ TAB 2: Stash ═══════════════════════════
            ColumnLayout {
                spacing: 0

                // Stash button bar
                Rectangle {
                    Layout.fillWidth: true; implicitHeight: 32
                    color: Theme.bgSurface

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2; anchors.rightMargin: Theme.sp2

                        Item { Layout.fillWidth: true }
                        GhostButton {
                            text: qsTr("Stash Changes")
                            enabled: (client.stagedFiles.length > 0 || client.unstagedFiles.length > 0) && !client.busy
                            onClicked: client.stashPush("")
                        }
                    }

                    Rectangle {
                        anchors.bottom: parent.bottom; width: parent.width; height: 1
                        color: Theme.borderSubtle
                    }
                }

                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: client.stashes

                    delegate: Rectangle {
                        required property var modelData
                        required property int index
                        width: ListView.view.width
                        height: 40
                        color: stashMouse.containsMouse ? Theme.bgHover : "transparent"

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        MouseArea {
                            id: stashMouse; anchors.fill: parent; hoverEnabled: true
                            acceptedButtons: Qt.LeftButton | Qt.RightButton
                            onClicked: (mouse) => {
                                if (mouse.button === Qt.RightButton) stashCtx.popup()
                            }
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3; anchors.rightMargin: Theme.sp2
                            spacing: Theme.sp2

                            Rectangle {
                                width: 6; height: 6; radius: 3
                                color: "#c77dff"
                                Layout.alignment: Qt.AlignVCenter
                            }

                            ColumnLayout {
                                Layout.fillWidth: true; spacing: 0

                                Text {
                                    text: modelData.message
                                    font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                                    color: Theme.textPrimary
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                                Text {
                                    text: modelData.relativeDate
                                    font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                                    color: Theme.textTertiary
                                }
                            }

                            // Apply / Pop / Drop buttons on hover
                            Row {
                                visible: stashMouse.containsMouse
                                spacing: Theme.sp1

                                GhostButton {
                                    text: qsTr("Apply")
                                    onClicked: client.stashApply(modelData.index)
                                }
                                GhostButton {
                                    text: qsTr("Pop")
                                    onClicked: client.stashPop(modelData.index)
                                }
                            }
                        }

                        Menu {
                            id: stashCtx
                            MenuItem { text: qsTr("Apply"); onTriggered: client.stashApply(modelData.index) }
                            MenuItem { text: qsTr("Pop"); onTriggered: client.stashPop(modelData.index) }
                            MenuSeparator {}
                            MenuItem { text: qsTr("Drop"); onTriggered: client.stashDrop(modelData.index) }
                        }
                    }

                    // Empty state
                    Text {
                        anchors.centerIn: parent
                        visible: client.stashes.length === 0
                        text: qsTr("No stashes")
                        font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody
                        color: Theme.textTertiary
                    }
                }
            }

            // ═══ TAB 3: Conflicts ═══════════════════════
            Item {
                Text {
                    anchors.centerIn: parent
                    text: qsTr("No merge conflicts")
                    font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody
                    color: Theme.textTertiary
                }
            }
        }
    }

    // ── Inline components ───────────────────────────────

    // Reusable file section (Staged / Unstaged)
    component FileSection: ColumnLayout {
        property string title: ""
        property color dotColor: Theme.textTertiary
        property string actionLabel: ""
        property var model: []
        property bool staged: false
        signal actionClicked()
        signal fileClicked(string path)
        signal fileAction(string path)
        signal fileDiscard(string path)

        spacing: 0

        Rectangle {
            Layout.fillWidth: true; implicitHeight: 24; color: Theme.bgHover

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2; anchors.rightMargin: Theme.sp2

                Rectangle { width: 6; height: 6; radius: 3; color: dotColor; Layout.alignment: Qt.AlignVCenter }
                Text {
                    text: title; font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                    font.weight: Theme.weightSemibold; color: Theme.textSecondary; Layout.fillWidth: true
                }
                Text {
                    text: actionLabel; font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall
                    color: Theme.accent
                    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: actionClicked() }
                }
            }
        }

        ListView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            model: parent.model

            delegate: Rectangle {
                required property var modelData
                required property int index
                width: ListView.view.width; height: 24
                color: fMouse.containsMouse ? Theme.bgHover : "transparent"
                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                MouseArea { id: fMouse; anchors.fill: parent; hoverEnabled: true; onClicked: fileClicked(modelData.path) }

                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: Theme.sp3; anchors.rightMargin: Theme.sp2; spacing: Theme.sp2

                    Rectangle {
                        width: 16; height: 16; radius: Theme.radiusXs; color: statusColor(modelData.status)
                        Text { anchors.centerIn: parent; text: modelData.status; font.family: Theme.fontMono; font.pixelSize: 9; font.weight: Theme.weightSemibold; color: "#fff" }
                    }
                    Text {
                        text: modelData.fileName || modelData.path.split("/").pop()
                        font.family: Theme.fontMono; font.pixelSize: Theme.sizeSmall; color: Theme.textPrimary; elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                    Text {
                        text: modelData.path.substring(0, modelData.path.lastIndexOf("/"))
                        font.family: Theme.fontUi; font.pixelSize: Theme.sizeSmall; font.italic: true; color: Theme.textTertiary
                        elide: Text.ElideMiddle; Layout.maximumWidth: 120
                        visible: modelData.path.indexOf("/") >= 0
                    }
                    // Action button (+/-)
                    Text {
                        text: staged ? "\u2212" : "+"; font.pixelSize: Theme.sizeBody; color: Theme.textTertiary
                        visible: fMouse.containsMouse
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: fileAction(modelData.path) }
                    }
                    // Discard (unstaged only)
                    Text {
                        text: "\u2715"; font.pixelSize: Theme.sizeSmall; color: Theme.statusError
                        visible: !staged && fMouse.containsMouse && modelData.status !== "?"
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: fileDiscard(modelData.path) }
                    }
                }
            }
        }
    }

    // Reusable diff viewer
    component DiffViewer: Rectangle {
        property string diffPath: ""
        property string diffText: ""

        color: Theme.bgSurface
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }

        ColumnLayout {
            anchors.fill: parent; spacing: 0

            Rectangle {
                Layout.fillWidth: true; implicitHeight: 24; color: Theme.bgHover; visible: diffPath.length > 0
                Text {
                    anchors.verticalCenter: parent.verticalCenter; x: Theme.sp2
                    text: diffPath; font.family: Theme.fontMono; font.pixelSize: Theme.sizeSmall; color: Theme.textSecondary
                    elide: Text.ElideMiddle; width: parent.width - Theme.sp4
                }
            }

            ScrollView {
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true

                TextArea {
                    readOnly: true; font.family: Theme.fontMono; font.pixelSize: Theme.sizeSmall
                    color: Theme.textPrimary; wrapMode: TextEdit.NoWrap; textFormat: TextEdit.RichText
                    text: diffText.length > 0 ? colorizeDiff(diffText) : qsTr("Select a file to view diff")
                    background: Rectangle { color: "transparent" }
                }
            }

            Text {
                Layout.alignment: Qt.AlignHCenter; visible: diffText.length === 0 && client.stagedFiles.length === 0 && client.unstagedFiles.length === 0
                text: qsTr("Working tree clean"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody; color: Theme.textTertiary; topPadding: Theme.sp6
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────

    function statusColor(code) {
        switch (code) {
        case "M": return "#4f8aff"
        case "A": return "#5fb865"
        case "D": return "#fa6675"
        case "R": return "#c77dff"
        case "?": return "#868a91"
        case "U": return "#f0a83a"
        case "C": return "#4f8aff"
        default:  return "#868a91"
        }
    }

    function colorizeDiff(raw) {
        var escaped = raw.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
        var lines = escaped.split("\n")
        var html = "<pre style='margin:0;'>"
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i]
            if (line.startsWith("+++") || line.startsWith("---"))
                html += "<span style='color:" + Theme.textSecondary + ";'>" + line + "</span>\n"
            else if (line.startsWith("@@"))
                html += "<span style='color:#c77dff;'>" + line + "</span>\n"
            else if (line.startsWith("+"))
                html += "<span style='color:#5fb865;'>" + line + "</span>\n"
            else if (line.startsWith("-"))
                html += "<span style='color:#fa6675;'>" + line + "</span>\n"
            else
                html += "<span style='color:" + Theme.textPrimary + ";'>" + line + "</span>\n"
        }
        return html + "</pre>"
    }
}
