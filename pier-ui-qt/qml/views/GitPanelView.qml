import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Effects
import QtQuick.Layouts
import Pier
import "../components"

Rectangle {
    id: root

    property string repoPath: ""
    property int selectedTab: 0
    property bool statusBannerVisible: false
    property bool statusBannerSuccess: true
    property string statusBannerMessage: ""
    property bool initInProgress: false

    signal closePanelRequested()

    readonly property int totalChanges: client.stagedFiles.length + client.unstagedFiles.length
    readonly property bool workingTreeClean: totalChanges === 0
    readonly property string repoName: {
        const normalized = String(root.repoPath || "").replace(/[\\\/]+$/, "")
        if (normalized.length === 0)
            return qsTr("Git")
        const slash = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"))
        return slash >= 0 ? normalized.slice(slash + 1) : normalized
    }

    function ensureTabData(index) {
        if (index === 1 && client.commits.length === 0)
            client.loadHistory()
        if (index === 2 && client.stashes.length === 0)
            client.loadStashes()
    }

    color: Theme.bgPanel
    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierGitClient { id: client }

    Timer {
        id: bannerTimer
        interval: 2800
        repeat: false
        onTriggered: root.statusBannerVisible = false
    }

    Connections {
        target: client

        function onOperationFinished(operation, success, message) {
            root.statusBannerSuccess = success
            root.statusBannerMessage = message.length > 0
                    ? message
                    : (success
                       ? qsTr("%1 finished").arg(operation)
                       : qsTr("%1 failed").arg(operation))
            root.statusBannerVisible = true
            bannerTimer.restart()
            client.refresh()
            root.ensureTabData(root.selectedTab)
        }
    }

    onRepoPathChanged: {
        if (repoPath.length > 0)
            client.open(repoPath)
        else
            client.close()
    }

    onVisibleChanged: {
        if (!visible || !client.isGitRepo)
            return
        client.refresh()
        root.ensureTabData(root.selectedTab)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 42
            color: Theme.bgPanel
            border.width: 0

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                Rectangle {
                    Layout.preferredWidth: 18
                    Layout.preferredHeight: 18
                    radius: Theme.radiusSm
                    color: Theme.accentSubtle

                    Image {
                        anchors.centerIn: parent
                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/git-branch.svg"
                        sourceSize: Qt.size(14, 14)
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorizationColor: Theme.accent
                            colorization: 1.0
                        }
                    }
                }

                Rectangle {
                    implicitWidth: branchRow.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 26
                    radius: Theme.radiusPill
                    color: branchMouse.containsMouse ? Theme.bgHover : Theme.bgInset
                    border.color: branchMouse.containsMouse ? Theme.borderDefault : Theme.borderSubtle
                    border.width: 1

                    Row {
                        id: branchRow
                        anchors.centerIn: parent
                        spacing: Theme.sp1

                        Text {
                            text: client.currentBranch.length > 0 ? client.currentBranch : qsTr("Detached")
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeSmall
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                        }

                        Image {
                            source: "qrc:/qt/qml/Pier/resources/icons/lucide/chevron-down.svg"
                            sourceSize: Qt.size(12, 12)
                            anchors.verticalCenter: parent.verticalCenter
                            layer.enabled: true
                            layer.effect: MultiEffect {
                                colorizationColor: Theme.textTertiary
                                colorization: 1.0
                            }
                        }
                    }

                    MouseArea {
                        id: branchMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadBranches()
                            const pos = branchMouse.mapToItem(root, 0, branchMouse.height + Theme.sp1)
                            branchMenu.x = Math.max(Theme.sp2,
                                                    Math.min(root.width - branchMenu.width - Theme.sp2, pos.x))
                            branchMenu.y = Math.max(Theme.sp2, pos.y)
                            branchMenu.open()
                        }
                    }

                    PopoverPanel {
                        id: branchMenu
                        width: 220

                        Repeater {
                            model: client.branches

                            PierMenuItem {
                                required property string modelData
                                text: modelData
                                active: modelData === client.currentBranch
                                onClicked: {
                                    branchMenu.close()
                                    client.checkoutBranch(modelData)
                                }
                            }
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    visible: client.trackingBranch.length > 0
                    text: "\u2192 " + client.trackingBranch
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }

                Rectangle {
                    visible: client.aheadCount > 0
                    implicitWidth: aheadText.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 22
                    radius: Theme.radiusPill
                    color: Theme.accentSubtle

                    Text {
                        id: aheadText
                        anchors.centerIn: parent
                        text: "\u2191 " + client.aheadCount
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.statusInfo
                    }
                }

                Rectangle {
                    visible: client.behindCount > 0
                    implicitWidth: behindText.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 22
                    radius: Theme.radiusPill
                    color: Qt.rgba(240 / 255, 168 / 255, 58 / 255, Theme.dark ? 0.16 : 0.12)

                    Text {
                        id: behindText
                        anchors.centerIn: parent
                        text: "\u2193 " + client.behindCount
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.statusWarning
                    }
                }

                Rectangle {
                    implicitWidth: summaryText.implicitWidth + Theme.sp2 * 2
                    implicitHeight: 22
                    radius: Theme.radiusPill
                    color: root.workingTreeClean
                           ? Qt.rgba(95 / 255, 184 / 255, 101 / 255, Theme.dark ? 0.14 : 0.10)
                           : Theme.accentSubtle

                    Text {
                        id: summaryText
                        anchors.centerIn: parent
                        text: root.workingTreeClean
                              ? qsTr("Clean")
                              : qsTr("%1 files").arg(root.totalChanges)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        font.weight: Theme.weightMedium
                        color: root.workingTreeClean ? Theme.statusSuccess : Theme.accent
                    }
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Refresh")
                    enabled: !client.busy
                    onClicked: {
                        client.refresh()
                        root.ensureTabData(root.selectedTab)
                    }
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Theme.borderSubtle
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: root.statusBannerVisible ? 28 : 0
            visible: implicitHeight > 0
            color: root.statusBannerSuccess
                   ? Qt.rgba(95 / 255, 184 / 255, 101 / 255, Theme.dark ? 0.12 : 0.08)
                   : Qt.rgba(250 / 255, 102 / 255, 117 / 255, Theme.dark ? 0.14 : 0.10)
            border.color: root.statusBannerSuccess ? Theme.statusSuccess : Theme.statusError
            border.width: 1
            clip: true

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                spacing: Theme.sp2

                Rectangle {
                    Layout.preferredWidth: 6
                    Layout.preferredHeight: 6
                    radius: 3
                    color: root.statusBannerSuccess ? Theme.statusSuccess : Theme.statusError
                }

                Text {
                    Layout.fillWidth: true
                    text: root.statusBannerMessage
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                MouseArea {
                    Layout.preferredWidth: 18
                    Layout.preferredHeight: 18
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        bannerTimer.stop()
                        root.statusBannerVisible = false
                    }

                    Text {
                        anchors.centerIn: parent
                        text: "\u2715"
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 36
            color: Theme.bgPanel
            border.width: 0

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                spacing: Theme.sp2

                Repeater {
                    model: [
                        { label: qsTr("Changes"), idx: 0, badge: root.totalChanges > 0 ? String(root.totalChanges) : "" },
                        { label: qsTr("History"), idx: 1, badge: "" },
                        { label: qsTr("Stash"), idx: 2, badge: client.stashes.length > 0 ? String(client.stashes.length) : "" },
                        { label: qsTr("Conflicts"), idx: 3, badge: "" }
                    ]

                    delegate: Rectangle {
                        required property var modelData

                        implicitWidth: tabRow.implicitWidth + Theme.sp3 * 2
                        implicitHeight: 26
                        radius: Theme.radiusPill
                        color: root.selectedTab === modelData.idx
                               ? Theme.bgSelected
                               : tabMouse.containsMouse ? Theme.bgHover : "transparent"
                        border.color: root.selectedTab === modelData.idx ? Theme.borderFocus : "transparent"
                        border.width: root.selectedTab === modelData.idx ? 1 : 0

                        Row {
                            id: tabRow
                            anchors.centerIn: parent
                            spacing: Theme.sp1

                            Text {
                                text: modelData.label
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                font.weight: root.selectedTab === modelData.idx ? Theme.weightSemibold : Theme.weightMedium
                                color: root.selectedTab === modelData.idx ? Theme.textPrimary : Theme.textSecondary
                            }

                            Rectangle {
                                visible: modelData.badge.length > 0
                                implicitWidth: badgeText.implicitWidth + Theme.sp1 * 2
                                implicitHeight: 16
                                radius: Theme.radiusPill
                                color: root.selectedTab === modelData.idx ? Theme.accentSubtle : Theme.bgInset

                                Text {
                                    id: badgeText
                                    anchors.centerIn: parent
                                    text: modelData.badge
                                    font.family: Theme.fontMono
                                    font.pixelSize: 9
                                    color: root.selectedTab === modelData.idx ? Theme.accent : Theme.textTertiary
                                }
                            }
                        }

                        MouseArea {
                            id: tabMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                root.selectedTab = modelData.idx
                                root.ensureTabData(modelData.idx)
                            }
                        }
                    }
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Theme.borderSubtle
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !client.isGitRepo

            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp8, 320)
                spacing: Theme.sp3

                EmptyStateCard {
                    Layout.fillWidth: true
                    title: client.status === PierGitClient.Loading ? qsTr("Loading repository") : qsTr("No repository")
                    description: client.status === PierGitClient.Loading
                                 ? qsTr("Pier-X is resolving the current working tree.")
                                 : qsTr("This folder is not initialized as a Git repository yet.")
                    accentColor: Theme.accent
                }

                PrimaryButton {
                    Layout.alignment: Qt.AlignHCenter
                    text: root.initInProgress ? qsTr("Initializing…") : qsTr("Initialize Git")
                    visible: client.status !== PierGitClient.Loading && root.repoPath.length > 0
                    enabled: !root.initInProgress
                    onClicked: {
                        root.initInProgress = true
                        const ok = PierLocalSystem.initGitRepository(root.repoPath)
                        root.initInProgress = false
                        root.statusBannerSuccess = ok
                        root.statusBannerMessage = ok
                                ? qsTr("Initialized a Git repository in %1.").arg(root.repoName)
                                : qsTr("Failed to initialize a Git repository in %1.").arg(root.repoName)
                        root.statusBannerVisible = true
                        bannerTimer.restart()
                        if (ok)
                            client.open(root.repoPath)
                    }
                }
            }
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: client.isGitRepo
            currentIndex: root.selectedTab

            SplitView {
                id: gitWorkSplit
                orientation: Qt.Vertical
                handle: PierSplitHandle {
                    vertical: gitWorkSplit.orientation === Qt.Horizontal
                }

                Item {
                    SplitView.fillWidth: true
                    SplitView.preferredHeight: 368
                    SplitView.minimumHeight: 220

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp2

                        FileSection {
                            Layout.fillWidth: true
                            title: qsTr("Staged")
                            countText: client.stagedFiles.length > 0 ? String(client.stagedFiles.length) : ""
                            helperText: qsTr("Files ready to commit")
                            dotColor: Theme.statusSuccess
                            actionLabel: client.stagedFiles.length > 0 ? qsTr("Unstage all") : ""
                            model: client.stagedFiles
                            staged: true
                            onActionClicked: client.unstageAll()
                            onFileClicked: (path) => client.loadDiff(path, true)
                            onFileAction: (path) => client.unstageFile(path)
                        }

                        FileSection {
                            Layout.fillWidth: true
                            title: qsTr("Working tree")
                            countText: client.unstagedFiles.length > 0 ? String(client.unstagedFiles.length) : ""
                            helperText: qsTr("Modified and untracked files")
                            dotColor: Theme.statusWarning
                            actionLabel: client.unstagedFiles.length > 0 ? qsTr("Stage all") : ""
                            model: client.unstagedFiles
                            staged: false
                            onActionClicked: client.stageAll()
                            onFileClicked: (path) => client.loadDiff(path, false)
                            onFileAction: (path) => client.stageFile(path)
                            onFileDiscard: (path) => client.discardFile(path)
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: composerLayout.implicitHeight + Theme.sp3 * 2
                            color: Theme.bgSurface
                            border.color: Theme.borderSubtle
                            border.width: 1
                            radius: Theme.radiusMd

                            ColumnLayout {
                                id: composerLayout
                                anchors.fill: parent
                                anchors.margins: Theme.sp3
                                spacing: Theme.sp2

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    Text {
                                        text: qsTr("Commit")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightSemibold
                                        color: Theme.textPrimary
                                    }

                                    Text {
                                        text: client.stagedFiles.length > 0
                                              ? qsTr("%1 staged file(s)").arg(client.stagedFiles.length)
                                              : qsTr("Stage changes to enable commit")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                        elide: Text.ElideRight
                                        Layout.fillWidth: true
                                    }
                                }

                                PierTextArea {
                                    id: commitMsg
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 72
                                    mono: true
                                    inset: true
                                    placeholderText: qsTr("Write a focused commit message…")
                                    wrapMode: TextEdit.Wrap
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    GhostButton {
                                        compact: true
                                        minimumWidth: 0
                                        text: qsTr("Stage all")
                                        visible: client.unstagedFiles.length > 0
                                        onClicked: client.stageAll()
                                    }

                                    Item { Layout.fillWidth: true }

                                    GhostButton {
                                        compact: true
                                        minimumWidth: 0
                                        text: qsTr("Pull")
                                        enabled: client.behindCount > 0 && !client.busy
                                        onClicked: client.pull()
                                    }

                                    GhostButton {
                                        compact: true
                                        minimumWidth: 0
                                        text: qsTr("Push")
                                        enabled: client.aheadCount > 0 && !client.busy
                                        onClicked: client.push()
                                    }

                                    PrimaryButton {
                                        text: qsTr("Commit")
                                        enabled: commitMsg.text.trim().length > 0
                                                 && client.stagedFiles.length > 0
                                                 && !client.busy
                                        onClicked: {
                                            client.commit(commitMsg.text.trim())
                                            commitMsg.text = ""
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                DiffViewer {
                    SplitView.fillWidth: true
                    SplitView.fillHeight: true
                    SplitView.minimumHeight: 120
                    diffPath: client.diffPath
                    diffText: client.diffText
                    workingTreeClean: root.workingTreeClean
                }
            }

            Rectangle {
                color: Theme.bgPanel

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: Theme.sp2
                    spacing: Theme.sp2

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 36
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusMd

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            spacing: Theme.sp2

                            Text {
                                text: qsTr("Recent commits")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightSemibold
                                color: Theme.textPrimary
                            }

                            Text {
                                text: client.currentBranch.length > 0 ? client.currentBranch : root.repoName
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                Layout.fillWidth: true
                                elide: Text.ElideMiddle
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusMd
                        clip: true

                        ListView {
                            anchors.fill: parent
                            clip: true
                            model: client.commits

                            delegate: Rectangle {
                                required property var modelData
                                width: ListView.view.width
                                height: 46
                                color: historyMouse.containsMouse ? Theme.bgHover : "transparent"

                                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                MouseArea {
                                    id: historyMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                }

                                ColumnLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp3
                                    anchors.rightMargin: Theme.sp3
                                    spacing: 0

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: Theme.sp2

                                        Text {
                                            text: modelData.shortHash
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.accent
                                        }

                                        Rectangle {
                                            visible: modelData.refs.length > 0
                                            implicitWidth: refsText.implicitWidth + Theme.sp1 * 2
                                            implicitHeight: 16
                                            radius: Theme.radiusPill
                                            color: Theme.accentSubtle

                                            Text {
                                                id: refsText
                                                anchors.centerIn: parent
                                                text: modelData.refs.split(",")[0].trim()
                                                font.family: Theme.fontMono
                                                font.pixelSize: 9
                                                color: Theme.accent
                                            }
                                        }

                                        Text {
                                            text: modelData.message
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                            Layout.fillWidth: true
                                        }
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: Theme.sp2

                                        Text {
                                            text: modelData.author
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textTertiary
                                            elide: Text.ElideRight
                                        }

                                        Item { Layout.fillWidth: true }

                                        Text {
                                            text: modelData.relativeDate
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textTertiary
                                        }
                                    }
                                }

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: Theme.borderSubtle
                                }
                            }

                            EmptyStateCard {
                                anchors.centerIn: parent
                                width: Math.min(parent.width - Theme.sp8, 250)
                                visible: client.commits.length === 0
                                title: qsTr("No history yet")
                                description: qsTr("Commit history will appear here after Git metadata is loaded.")
                                accentColor: Theme.accent
                            }
                        }
                    }
                }
            }

            Rectangle {
                color: Theme.bgPanel

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: Theme.sp2
                    spacing: Theme.sp2

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 36
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusMd

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            spacing: Theme.sp2

                            Text {
                                text: qsTr("Stash")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightSemibold
                                color: Theme.textPrimary
                            }

                            Text {
                                text: client.stashes.length > 0
                                      ? qsTr("%1 entries").arg(client.stashes.length)
                                      : qsTr("Snapshot unfinished work")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                Layout.fillWidth: true
                                elide: Text.ElideRight
                            }

                            GhostButton {
                                compact: true
                                minimumWidth: 0
                                text: qsTr("Stash changes")
                                enabled: !root.workingTreeClean && !client.busy
                                onClicked: client.stashPush("")
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusMd
                        clip: true

                        ListView {
                            anchors.fill: parent
                            clip: true
                            model: client.stashes

                            delegate: Rectangle {
                                required property var modelData
                                width: ListView.view.width
                                height: 48
                                color: stashMouse.containsMouse ? Theme.bgHover : "transparent"

                                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                MouseArea {
                                    id: stashMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                                    cursorShape: Qt.PointingHandCursor
                                    onPressed: (mouse) => {
                                        if (mouse.button === Qt.RightButton) {
                                            const pos = stashMouse.mapToItem(root, mouse.x, mouse.y)
                                            stashMenu.x = Math.max(Theme.sp2,
                                                                   Math.min(root.width - stashMenu.width - Theme.sp2, pos.x))
                                            stashMenu.y = Math.max(Theme.sp2, pos.y)
                                            stashMenu.open()
                                            mouse.accepted = true
                                        }
                                    }
                                }

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp3
                                    anchors.rightMargin: Theme.sp3
                                    spacing: Theme.sp2

                                    Rectangle {
                                        Layout.preferredWidth: 8
                                        Layout.preferredHeight: 8
                                        radius: 4
                                        color: "#c77dff"
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 0

                                        Text {
                                            text: modelData.message
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                            Layout.fillWidth: true
                                        }

                                        Text {
                                            text: modelData.relativeDate
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textTertiary
                                        }
                                    }

                                    Row {
                                        visible: stashMouse.containsMouse
                                        spacing: Theme.sp1

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Apply")
                                            onClicked: client.stashApply(modelData.index)
                                        }

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Pop")
                                            onClicked: client.stashPop(modelData.index)
                                        }
                                    }
                                }

                                Rectangle {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    height: 1
                                    color: Theme.borderSubtle
                                }

                                PopoverPanel {
                                    id: stashMenu
                                    width: 188
                                    PierMenuItem {
                                        text: qsTr("Apply")
                                        onClicked: {
                                            stashMenu.close()
                                            client.stashApply(modelData.index)
                                        }
                                    }
                                    PierMenuItem {
                                        text: qsTr("Pop")
                                        onClicked: {
                                            stashMenu.close()
                                            client.stashPop(modelData.index)
                                        }
                                    }
                                    Rectangle {
                                        width: stashMenu.width - stashMenu.leftPadding - stashMenu.rightPadding
                                        height: 1
                                        color: Theme.borderSubtle
                                    }
                                    PierMenuItem {
                                        text: qsTr("Drop")
                                        destructive: true
                                        onClicked: {
                                            stashMenu.close()
                                            client.stashDrop(modelData.index)
                                        }
                                    }
                                }
                            }

                            EmptyStateCard {
                                anchors.centerIn: parent
                                width: Math.min(parent.width - Theme.sp8, 250)
                                visible: client.stashes.length === 0
                                title: qsTr("No stashes")
                                description: qsTr("Use stash to park unfinished work without leaving the current branch.")
                                accentColor: "#c77dff"
                            }
                        }
                    }
                }
            }

            Item {
                EmptyStateCard {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - Theme.sp8, 280)
                    title: qsTr("No merge conflicts")
                    description: qsTr("Conflicted files will appear here when Git requires manual resolution.")
                    accentColor: Theme.statusWarning
                }
            }
        }
    }

    component EmptyStateCard: Rectangle {
        property string title: ""
        property string description: ""
        property color accentColor: Theme.accent

        height: 136
        radius: Theme.radiusMd
        color: Theme.bgInset
        border.color: Theme.borderSubtle
        border.width: 1

        ColumnLayout {
            anchors.centerIn: parent
            width: parent.width - Theme.sp6 * 2
            spacing: Theme.sp2

            Rectangle {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: 26
                Layout.preferredHeight: 26
                radius: Theme.radiusMd
                color: Qt.rgba(accentColor.r, accentColor.g, accentColor.b, Theme.dark ? 0.16 : 0.10)

                Rectangle {
                    anchors.centerIn: parent
                    width: 8
                    height: 8
                    radius: 4
                    color: accentColor
                }
            }

            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBodyLg
                font.weight: Theme.weightSemibold
                color: Theme.textPrimary
                wrapMode: Text.WordWrap
            }

            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: description
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeSmall
                color: Theme.textTertiary
                wrapMode: Text.WordWrap
            }
        }
    }

    component FileSection: Rectangle {
        property string title: ""
        property string helperText: ""
        property string countText: ""
        property color dotColor: Theme.textTertiary
        property string actionLabel: ""
        property var model: []
        property bool staged: false
        signal actionClicked()
        signal fileClicked(string path)
        signal fileAction(string path)
        signal fileDiscard(string path)

        readonly property int rowCount: model ? model.length : 0
        readonly property int bodyHeight: rowCount > 0 ? Math.min(rowCount, staged ? 4 : 6) * Theme.compactRowHeight : 68

        Layout.fillWidth: true
        implicitHeight: 36 + bodyHeight + 1
        color: Theme.bgSurface
        border.color: Theme.borderSubtle
        border.width: 1
        radius: Theme.radiusMd
        clip: true

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 36
                color: Theme.bgPanel

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp2

                    Rectangle {
                        Layout.preferredWidth: 7
                        Layout.preferredHeight: 7
                        radius: 3.5
                        color: dotColor
                    }

                    Text {
                        text: title
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightSemibold
                        color: Theme.textPrimary
                    }

                    Rectangle {
                        visible: countText.length > 0
                        implicitWidth: countLabel.implicitWidth + Theme.sp1 * 2
                        implicitHeight: 16
                        radius: Theme.radiusPill
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1

                        Text {
                            id: countLabel
                            anchors.centerIn: parent
                            text: countText
                            font.family: Theme.fontMono
                            font.pixelSize: 9
                            color: Theme.textTertiary
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        text: helperText
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    GhostButton {
                        visible: actionLabel.length > 0
                        compact: true
                        minimumWidth: 0
                        text: actionLabel
                        onClicked: actionClicked()
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 1
                color: Theme.borderSubtle
            }

            Item {
                Layout.fillWidth: true
                Layout.preferredHeight: bodyHeight

                ListView {
                    anchors.fill: parent
                    clip: true
                    model: parent.parent.parent.model
                    visible: parent.parent.parent.rowCount > 0

                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width
                        height: Theme.compactRowHeight
                        color: rowMouse.containsMouse ? Theme.bgHover : "transparent"

                        Behavior on color { ColorAnimation { duration: Theme.durFast } }

                        MouseArea {
                            id: rowMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: parent.parent.parent.parent.parent.fileClicked(modelData.path)
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            spacing: Theme.sp2

                            Rectangle {
                                Layout.preferredWidth: 16
                                Layout.preferredHeight: 16
                                radius: Theme.radiusXs
                                color: root.statusColor(modelData.status)

                                Text {
                                    anchors.centerIn: parent
                                    text: modelData.status
                                    font.family: Theme.fontMono
                                    font.pixelSize: 9
                                    font.weight: Theme.weightSemibold
                                    color: "#ffffff"
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData.fileName || modelData.path.split("/").pop()
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textPrimary
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    visible: modelData.path.indexOf("/") >= 0
                                    text: modelData.path.substring(0, modelData.path.lastIndexOf("/"))
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textTertiary
                                    elide: Text.ElideMiddle
                                }
                            }

                            Text {
                                visible: rowMouse.containsMouse
                                text: parent.parent.parent.parent.parent.staged ? "\u2212" : "+"
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBodyLg
                                color: Theme.textSecondary

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        mouse.accepted = true
                                        parent.parent.parent.parent.parent.parent.fileAction(modelData.path)
                                    }
                                }
                            }

                            Text {
                                visible: !parent.parent.parent.parent.parent.staged
                                         && rowMouse.containsMouse
                                         && modelData.status !== "?"
                                text: "\u2715"
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.statusError

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        mouse.accepted = true
                                        parent.parent.parent.parent.parent.parent.fileDiscard(modelData.path)
                                    }
                                }
                            }
                        }

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 1
                            color: Theme.borderSubtle
                        }
                    }
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    visible: parent.parent.parent.rowCount === 0
                    width: Math.min(parent.width - Theme.sp6, 220)
                    spacing: Theme.sp1

                    Text {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        text: parent.parent.parent.staged ? qsTr("Nothing staged") : qsTr("Working tree clean")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }

                    Text {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        text: parent.parent.parent.staged
                              ? qsTr("Stage files from the working tree to prepare a commit.")
                              : qsTr("Modified files will appear here automatically.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }

    component DiffViewer: Rectangle {
        property string diffPath: ""
        property string diffText: ""
        property bool workingTreeClean: false

        color: Theme.bgSurface
        border.color: Theme.borderSubtle
        border.width: 1
        radius: Theme.radiusMd
        clip: true

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 30
                color: Theme.bgInset

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp2

                    Text {
                        text: diffPath.length > 0 ? diffPath : qsTr("Diff")
                        font.family: diffPath.length > 0 ? Theme.fontMono : Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: diffPath.length > 0 ? Theme.textSecondary : Theme.textTertiary
                        elide: Text.ElideMiddle
                        Layout.fillWidth: true
                    }
                }
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                PierScrollView {
                    anchors.fill: parent
                    clip: true
                    visible: diffText.length > 0

                    PierTextArea {
                        readOnly: true
                        frameVisible: false
                        mono: true
                        textFormat: TextEdit.RichText
                        wrapMode: TextEdit.NoWrap
                        text: root.colorizeDiff(diffText)
                        font.pixelSize: Theme.sizeSmall
                    }
                }

                EmptyStateCard {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - Theme.sp8, 250)
                    visible: diffText.length === 0
                    title: workingTreeClean ? qsTr("Working tree clean") : qsTr("Select a file")
                    description: workingTreeClean
                                 ? qsTr("Git diff output will appear here once files change.")
                                 : qsTr("Choose a staged or modified file to inspect its patch.")
                    accentColor: workingTreeClean ? Theme.statusSuccess : Theme.accent
                }
            }
        }
    }

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
