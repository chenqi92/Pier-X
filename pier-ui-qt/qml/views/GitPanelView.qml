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
    property string historySearchText: ""
    property string historyBranchFilter: ""
    property string historyAuthorFilter: ""
    property string historyDateFilter: "all"
    property string historyPathFilter: ""
    property string historySortMode: "topo"
    property bool historyFirstParent: false
    property bool historyNoMerges: false
    property bool historyShowLongEdges: true
    property bool historyShowZebraStripes: true
    property bool historyShowHash: true
    property bool historyShowAuthor: true
    property bool historyShowDate: true
    property string historyHighlightMode: "none"
    property bool historyPathDialogOpen: false
    property string historyPathSearchText: ""
    property var historyPathSelection: []
    property string historySelectedHash: ""
    property var historySelectedCommit: ({})
    property var historyContextCommit: ({})
    property bool historyCompareDialogOpen: false
    property string historyCompareSelectedPath: ""
    property bool historyBranchDialogOpen: false
    property bool historyTagDialogOpen: false
    property bool historyResetDialogOpen: false
    property bool historyEditMessageDialogOpen: false
    property bool historyDropDialogOpen: false
    property string branchDraftName: ""
    property string branchRenameSource: ""
    property string branchRenameTarget: ""
    property string trackingBranchTarget: ""
    property string trackingUpstreamTarget: ""
    property string branchManagerMode: "local"
    property string branchManagerSearchText: ""
    property bool branchCreateExpanded: false
    property string tagDraftName: ""
    property string tagDraftMessage: ""
    property string tagSearchText: ""
    property bool tagCreateExpanded: false
    property string remoteDraftName: ""
    property string remoteDraftUrl: ""
    property string remoteEditSourceName: ""
    property string remoteSearchText: ""
    property bool remoteComposerExpanded: false
    property string configDraftKey: ""
    property string configDraftValue: ""
    property bool configDraftGlobal: false
    property bool configSelectedGlobal: false
    property string configSearchText: ""
    property bool configComposerExpanded: false
    property string submoduleSearchText: ""
    property bool blameDialogOpen: false
    property int rebaseCommitCount: 10
    property var rebaseDraftItems: []
    property string historyTagDraftName: ""
    property string historyTagDraftMessage: ""
    property string historyBranchDraftName: ""
    property string historyResetMode: "mixed"
    property string historyAmendMessage: ""
    property string selectedConflictPath: ""
    readonly property var conflictFiles: client.conflictFiles || []
    readonly property var selectedConflictFile: {
        const files = root.conflictFiles || []
        for (let i = 0; i < files.length; ++i) {
            if (String(files[i].path || "") === String(root.selectedConflictPath || ""))
                return files[i]
        }
        return files.length > 0 ? files[0] : ({})
    }
    readonly property var graphPalette: [
        Theme.statusSuccess,
        Theme.accent,
        "#d97706",
        "#8b5cf6",
        Theme.statusError,
        "#06b6d4",
        "#eab308",
        "#ec4899"
    ]

    signal closePanelRequested()

    readonly property int totalChanges: client.stagedFiles.length + client.unstagedFiles.length
    readonly property bool workingTreeClean: totalChanges === 0
    readonly property var activeCommitDetail: {
        const detail = client.commitDetail || {}
        if (detail.hash)
            return detail
        return root.historySelectedCommit || {}
    }
    readonly property bool selectedCommitIsHead: {
        if (!client.graphRows || client.graphRows.length === 0)
            return false
        return String(root.activeCommitDetail.hash || "") === String(client.graphRows[0].hash || "")
    }
    readonly property string repoName: {
        const normalized = String(root.repoPath || "").replace(/[\\\/]+$/, "")
        if (normalized.length === 0)
            return qsTr("Git")
        const slash = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"))
        return slash >= 0 ? normalized.slice(slash + 1) : normalized
    }
    readonly property int conflictCount: {
        return (root.conflictFiles || []).length
    }
    readonly property var navigationTabs: [
        { label: qsTr("Changes"), icon: "file-text", idx: 0, badge: root.totalChanges > 0 ? String(root.totalChanges) : "" },
        { label: qsTr("History"), icon: "scroll-text", idx: 1, badge: "" },
        { label: qsTr("Stash"), icon: "hard-drive", idx: 2, badge: client.stashes.length > 0 ? String(client.stashes.length) : "" },
        { label: qsTr("Conflicts"), icon: "layers", idx: 3, badge: root.conflictCount > 0 ? String(root.conflictCount) : "" }
    ]

    function loadGraphTab() {
        client.loadGraphMetadata()
        client.loadGraphHistory(180, 0,
                                historyBranchFilter,
                                historyAuthorFilter,
                                historySearchText,
                                historyFirstParent,
                                historyNoMerges,
                                root.historyAfterTimestamp(),
                                historyPathFilter,
                                root.historySortMode === "topo",
                                root.historyShowLongEdges)
    }

    function historyDateOptions() {
        return [
            qsTr("Any time"),
            qsTr("Last 7 days"),
            qsTr("Last 30 days"),
            qsTr("Last 90 days"),
            qsTr("Last year")
        ]
    }

    function historyDateIndex(key) {
        switch (String(key || "")) {
        case "7d": return 1
        case "30d": return 2
        case "90d": return 3
        case "365d": return 4
        default: return 0
        }
    }

    function historyDateKeyAt(index) {
        switch (index) {
        case 1: return "7d"
        case 2: return "30d"
        case 3: return "90d"
        case 4: return "365d"
        default: return "all"
        }
    }

    function historyAfterTimestamp() {
        const now = Math.floor(Date.now() / 1000)
        switch (root.historyDateFilter) {
        case "7d": return now - 7 * 24 * 60 * 60
        case "30d": return now - 30 * 24 * 60 * 60
        case "90d": return now - 90 * 24 * 60 * 60
        case "365d": return now - 365 * 24 * 60 * 60
        default: return 0
        }
    }

    function ensureTabData(index) {
        if (index === 1)
            loadGraphTab()
        if (index === 2 && client.stashes.length === 0)
            client.loadStashes()
        if (index === 3)
            client.detectConflicts()
    }

    function openPopoverFrom(item, popup, widthHint) {
        const desiredWidth = widthHint > 0 ? widthHint : popup.width
        const pos = item.mapToItem(root, 0, item.height + Theme.sp1)
        popup.width = desiredWidth
        popup.x = Math.max(Theme.sp2,
                           Math.min(root.width - popup.width - Theme.sp2, pos.x))
        popup.y = Math.max(Theme.sp2, pos.y)
        popup.open()
    }

    function refTokens(rawRefs) {
        if (!rawRefs || String(rawRefs).trim().length === 0)
            return []
        return String(rawRefs).replace(/^\s*\(/, "").replace(/\)\s*$/, "").split(",").map(function(entry) {
            return String(entry).trim()
        }).filter(function(entry) {
            return entry.length > 0
        })
    }

    function historyCommitByHash(hash) {
        const rows = client.graphRows || []
        for (let i = 0; i < rows.length; ++i) {
            if ((rows[i].hash || "") === hash)
                return rows[i]
        }
        return ({})
    }

    function isLocalBranch(name) {
        return String(name || "").indexOf("/") === -1
    }

    function managerBranchList(localOnly) {
        const source = client.graphBranches || client.branches || []
        return source.filter(function(name) {
            return localOnly ? root.isLocalBranch(name) : !root.isLocalBranch(name)
        })
    }

    function filteredManagerBranchList(localOnly) {
        const needle = String(root.branchManagerSearchText || "").trim().toLowerCase()
        return root.managerBranchList(localOnly).filter(function(name) {
            if (!needle.length)
                return true
            return String(name || "").toLowerCase().indexOf(needle) >= 0
        })
    }

    function filteredTagEntries() {
        const needle = String(root.tagSearchText || "").trim().toLowerCase()
        return (client.tags || []).filter(function(tag) {
            if (!needle.length)
                return true
            const name = String(tag.name || "").toLowerCase()
            const hash = String(tag.hash || "").toLowerCase()
            const message = String(tag.message || "").toLowerCase()
            return name.indexOf(needle) >= 0
                    || hash.indexOf(needle) >= 0
                    || message.indexOf(needle) >= 0
        })
    }

    function filteredRemoteEntries() {
        const needle = String(root.remoteSearchText || "").trim().toLowerCase()
        return (client.remotes || []).filter(function(remote) {
            if (!needle.length)
                return true
            const name = String(remote.name || "").toLowerCase()
            const fetchUrl = String(remote.fetch_url || remote.fetchUrl || "").toLowerCase()
            const pushUrl = String(remote.push_url || remote.pushUrl || "").toLowerCase()
            return name.indexOf(needle) >= 0
                    || fetchUrl.indexOf(needle) >= 0
                    || pushUrl.indexOf(needle) >= 0
        })
    }

    function filteredSubmodules() {
        const needle = String(root.submoduleSearchText || "").trim().toLowerCase()
        return (client.submodules || []).filter(function(submodule) {
            if (!needle.length)
                return true
            const path = String(submodule.path || "").toLowerCase()
            const url = String(submodule.url || "").toLowerCase()
            const hash = String(submodule.shortHash || submodule.short_hash || "").toLowerCase()
            return path.indexOf(needle) >= 0
                    || url.indexOf(needle) >= 0
                    || hash.indexOf(needle) >= 0
        })
    }

    function beginConfigEdit(entry) {
        const isGlobal = String(entry.scope || "") === "global"
        root.configSelectedGlobal = isGlobal
        root.configDraftGlobal = isGlobal
        root.configDraftKey = String(entry.key || "")
        root.configDraftValue = String(entry.value || "")
        root.configComposerExpanded = true
    }

    function beginRemoteEdit(entry) {
        root.remoteEditSourceName = String(entry.name || "")
        root.remoteDraftName = String(entry.name || "")
        root.remoteDraftUrl = String(entry.fetch_url || entry.fetchUrl || "")
        root.remoteComposerExpanded = true
    }

    function clearRemoteDraft() {
        root.remoteEditSourceName = ""
        root.remoteDraftName = ""
        root.remoteDraftUrl = ""
        root.remoteComposerExpanded = false
    }

    function graphColor(index) {
        const palette = root.graphPalette
        if (!palette || palette.length === 0)
            return Theme.accent
        const safeIndex = Math.abs(Number(index || 0)) % palette.length
        return palette[safeIndex]
    }

    function formatGraphDate(timestamp) {
        const value = Number(timestamp || 0)
        if (!value)
            return ""
        return Qt.formatDateTime(new Date(value * 1000), "yyyy-MM-dd HH:mm")
    }

    function copyText(value) {
        if (!value || !String(value).length)
            return
        PierLocalSystem.copyText(String(value))
    }

    function historyFilterPaths() {
        return String(root.historyPathFilter || "")
            .split(/\n+/)
            .map(function(entry) { return String(entry || "").trim() })
            .filter(function(entry) { return entry.length > 0 })
    }

    function historyPathSummary() {
        const paths = root.historyFilterPaths()
        if (paths.length === 0)
            return qsTr("Path")
        if (paths.length === 1)
            return paths[0]
        return qsTr("%1 paths").arg(paths.length)
    }

    function historyFilteredRepoFiles() {
        const needle = String(root.historyPathSearchText || "").trim().toLowerCase()
        const source = client.graphRepoFiles || []
        if (!needle.length)
            return source
        return source.filter(function(path) {
            return String(path || "").toLowerCase().indexOf(needle) >= 0
        })
    }

    function selectHistoryCommit(commitData, openDetail) {
        const commit = commitData || ({})
        const hash = String(commit.hash || "")
        root.historyContextCommit = commit
        if (hash.length > 0) {
            root.historySelectedHash = hash
            root.historySelectedCommit = commit
            client.loadCommitDetail(hash)
        }
        if (openDetail === true && hash.length > 0)
            root.selectedTab = 1
    }

    function historyContextParentHash() {
        if (String(root.activeCommitDetail.hash || "") === String(root.historyContextCommit.hash || "")
                && String(root.activeCommitDetail.parentHash || "").length > 0)
            return String(root.activeCommitDetail.parentHash || "")
        const parents = String((root.historyContextCommit && root.historyContextCommit.parents) || "").trim()
        if (!parents.length)
            return ""
        return parents.split(/\s+/)[0] || ""
    }

    function historyContextIsHead() {
        const rows = client.graphRows || []
        if (!rows.length)
            return false
        return String(root.historyContextCommit.hash || "") === String(rows[0].hash || "")
    }

    function historyContextCheckoutTargets() {
        const items = []
        const hash = String(root.historyContextCommit.hash || "")
        if (hash.length > 0) {
            items.push({
                "label": qsTr("Checkout this revision"),
                "target": hash,
                "tracking": ""
            })
        }
        const seen = {}
        const refs = root.refTokens(root.historyContextCommit.refs || "")
        for (let i = 0; i < refs.length; ++i) {
            let ref = String(refs[i] || "").trim()
            if (!ref.length || ref === "HEAD" || ref.indexOf("tag:") === 0)
                continue
            if (ref.indexOf("→ ") === 0)
                ref = ref.slice(2)
            if (!ref.length)
                continue
            let target = ref
            let tracking = ""
            if (ref.indexOf("/") >= 0) {
                tracking = ref
                target = ref.replace(/^[^\/]+\//, "")
            }
            const key = target + "::" + tracking
            if (seen[key])
                continue
            seen[key] = true
            items.push({
                "label": qsTr("Checkout branch '%1'").arg(ref),
                "target": target,
                "tracking": tracking
            })
        }
        return items
    }

    function historyRowIsMerge(rowData) {
        const parents = String((rowData && rowData.parents) || "").trim()
        return parents.length > 0 && parents.split(/\s+/).length > 1
    }

    function historyRowShouldDim(rowData) {
        switch (String(root.historyHighlightMode || "none")) {
        case "mine":
            return String(rowData.author || "").trim().length > 0
                    && String(rowData.author || "").trim() !== String(client.graphGitUserName || "").trim()
        case "merge":
            return !root.historyRowIsMerge(rowData)
        case "branch":
            return String(rowData.refs || "").indexOf(client.currentBranch || "") === -1
                    && String(rowData.refs || "").indexOf("HEAD") === -1
        default:
            return false
        }
    }

    function normalizeRemoteBaseUrl(url) {
        const raw = String(url || "").trim()
        if (!raw.length)
            return ""
        if (raw.indexOf("git@") === 0) {
            const match = raw.match(/^git@([^:]+):(.+?)(?:\\.git)?$/)
            if (match)
                return "https://" + match[1] + "/" + match[2]
        }
        if (raw.indexOf("ssh://git@") === 0) {
            let normalized = raw.replace(/^ssh:\/\/git@/, "https://")
            normalized = normalized.replace(/:(\d+)\//, "/")
            normalized = normalized.replace(/\.git$/, "")
            return normalized
        }
        if (raw.indexOf("http://") === 0 || raw.indexOf("https://") === 0)
            return raw.replace(/\.git$/, "")
        return ""
    }

    function browserUrlForCommit(hash) {
        const remotes = client.remotes || []
        for (let i = 0; i < remotes.length; ++i) {
            const remote = remotes[i] || {}
            const base = root.normalizeRemoteBaseUrl(remote.fetch_url || remote.fetchUrl || remote.push_url || remote.pushUrl || "")
            if (!base.length)
                continue
            if (base.indexOf("github.com/") >= 0 || base.indexOf("gitlab.") >= 0 || base.indexOf("gitlab.com/") >= 0)
                return base + "/commit/" + hash
        }
        return ""
    }

    function openCommitInBrowser(hash) {
        const url = root.browserUrlForCommit(hash)
        if (url.length)
            PierLocalSystem.openExternalUrl(url)
    }

    function branchFilterOptions() {
        return [qsTr("All branches")].concat(client.graphBranches || [])
    }

    function authorFilterOptions() {
        return [qsTr("All authors")].concat(client.graphAuthors || [])
    }

    function optionIndex(options, value) {
        if (!value || !String(value).length)
            return 0
        const idx = options.indexOf(value)
        return idx >= 0 ? idx : 0
    }

    function ensureTrackingDefaults() {
        if (!root.trackingBranchTarget.length) {
            const locals = root.managerBranchList(true)
            root.trackingBranchTarget = locals.length > 0 ? locals[0] : client.currentBranch
        }
        if (!root.trackingUpstreamTarget.length) {
            const remotes = root.managerBranchList(false)
            root.trackingUpstreamTarget = remotes.length > 0 ? remotes[0] : ""
        }
    }

    function syncRebaseDraft() {
        const rows = client.rebaseTodoItems || []
        root.rebaseDraftItems = rows.map(function(item) {
            return {
                id: item.id || item.hash || "",
                action: item.action || "pick",
                hash: item.hash || "",
                shortHash: item.shortHash || "",
                message: item.message || ""
            }
        })
    }

    function moveRebaseItem(fromIndex, toIndex) {
        if (fromIndex === toIndex
                || fromIndex < 0 || toIndex < 0
                || fromIndex >= root.rebaseDraftItems.length
                || toIndex >= root.rebaseDraftItems.length)
            return
        const copy = root.rebaseDraftItems.slice()
        const row = copy.splice(fromIndex, 1)[0]
        copy.splice(toIndex, 0, row)
        root.rebaseDraftItems = copy
    }

    function ensureConflictSelection() {
        const files = root.conflictFiles || []
        if (files.length === 0) {
            root.selectedConflictPath = ""
            return
        }
        for (let i = 0; i < files.length; ++i) {
            if (String(files[i].path || "") === String(root.selectedConflictPath || ""))
                return
        }
        root.selectedConflictPath = String(files[0].path || "")
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

    Timer {
        id: historyFilterTimer
        interval: 220
        repeat: false
        onTriggered: {
            if (root.selectedTab === 1 && client.isGitRepo)
                root.loadGraphTab()
        }
    }

    Connections {
        target: client

        function onOperationFinished(operation, success, message) {
            let displayMessage = message
            if (success && (!displayMessage || String(displayMessage).trim().length === 0)) {
                if (operation === "tagPush")
                    displayMessage = qsTr("Pushed tag.")
                else if (operation === "tagPushAll")
                    displayMessage = qsTr("Pushed all tags.")
            }
            root.statusBannerSuccess = success
            root.statusBannerMessage = displayMessage.length > 0
                    ? displayMessage
                    : (success
                       ? qsTr("%1 finished").arg(operation)
                       : qsTr("%1 failed").arg(operation))
            root.statusBannerVisible = true
            bannerTimer.restart()
            client.refresh()
            client.loadGraphMetadata()
            root.ensureTabData(root.selectedTab)
        }

        function onGraphChanged() {
            if (!root.historySelectedHash.length && client.graphRows.length > 0) {
                root.historySelectedHash = client.graphRows[0].hash || ""
                root.historySelectedCommit = root.historyCommitByHash(root.historySelectedHash)
            } else if (root.historySelectedHash.length > 0) {
                root.historySelectedCommit = root.historyCommitByHash(root.historySelectedHash)
            }
        }

        function onCommitDetailChanged() {
            if (root.selectedCommitIsHead)
                root.historyAmendMessage = client.commitDetail.message || ""
        }

        function onRebaseChanged() {
            root.syncRebaseDraft()
        }

        function onConflictFilesChanged() {
            root.ensureConflictSelection()
        }

        function onComparisonChanged() {
            const files = client.comparisonFiles || []
            if (!root.historyCompareDialogOpen && files.length > 0)
                root.historyCompareDialogOpen = true
            if (files.length === 0) {
                root.historyCompareSelectedPath = ""
                return
            }
            let found = false
            for (let i = 0; i < files.length; ++i) {
                if (String(files[i].path || "") === String(root.historyCompareSelectedPath || "")) {
                    found = true
                    break
                }
            }
            if (!found) {
                root.historyCompareSelectedPath = String(files[0].path || "")
                if (root.historyCompareSelectedPath.length > 0 && client.comparisonBaseHash.length > 0)
                    client.loadComparisonDiff(client.comparisonBaseHash, root.historyCompareSelectedPath)
            }
        }
    }

    onRepoPathChanged: {
        root.historySelectedHash = ""
        root.historySelectedCommit = ({})
        root.historyContextCommit = ({})
        root.historyBranchDraftName = ""
        root.historyCompareSelectedPath = ""
        root.historyCompareDialogOpen = false
        root.selectedConflictPath = ""
        client.clearComparison()
        if (repoPath.length > 0) {
            client.open(repoPath)
            client.loadGraphMetadata()
            client.loadTags()
            client.loadRemotes()
            client.loadConfig()
            client.loadSubmodules()
        } else {
            client.close()
        }
    }

    onHistorySelectedHashChanged: {
        root.historySelectedCommit = root.historyCommitByHash(root.historySelectedHash)
        root.historyBranchDraftName = ""
        if (root.historySelectedHash.length > 0)
            client.loadCommitDetail(root.historySelectedHash)
    }

    onVisibleChanged: {
        if (!visible || !client.isGitRepo)
            return
        client.refresh()
        client.loadGraphMetadata()
        client.loadRemotes()
        root.ensureTabData(root.selectedTab)
    }

    onSelectedTabChanged: root.ensureTabData(root.selectedTab)

    onHistorySearchTextChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryBranchFilterChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryAuthorFilterChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryDateFilterChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryPathFilterChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistorySortModeChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryFirstParentChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryNoMergesChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryShowLongEdgesChanged: if (root.selectedTab === 1) historyFilterTimer.restart()
    onHistoryCompareSelectedPathChanged: {
        if (root.historyCompareSelectedPath.length > 0 && client.comparisonBaseHash.length > 0)
            client.loadComparisonDiff(client.comparisonBaseHash, root.historyCompareSelectedPath)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 66
            color: Theme.bgPanel
            border.width: 0

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp2
                anchors.topMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                spacing: Theme.sp1_5

                RowLayout {
                    Layout.fillWidth: true
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
                            width: 224

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
                        text: client.trackingBranch.length > 0
                              ? qsTr("Tracking %1").arg(client.trackingBranch)
                              : qsTr("No upstream branch configured")
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideMiddle
                    }

                    StatusPill {
                        text: root.workingTreeClean
                              ? qsTr("Workspace Clean")
                              : root.conflictCount > 0
                                ? qsTr("%1 Changes, %2 Conflicts").arg(root.totalChanges).arg(root.conflictCount)
                                : qsTr("%1 Changes").arg(root.totalChanges)
                        tone: root.workingTreeClean ? "success" : root.conflictCount > 0 ? "warning" : "info"
                    }

                    IconButton {
                        id: branchManagerButton
                        compact: true
                        icon: "git-branch"
                        tooltip: qsTr("Branches")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadGraphMetadata()
                            root.openPopoverFrom(branchManagerButton, branchManagerPopover, 320)
                        }
                    }

                    IconButton {
                        id: tagManagerButton
                        compact: true
                        icon: "tag"
                        tooltip: qsTr("Tags")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadTags()
                            root.openPopoverFrom(tagManagerButton, tagManagerPopover, 320)
                        }
                    }

                    IconButton {
                        id: remoteManagerButton
                        compact: true
                        icon: "network"
                        tooltip: qsTr("Remotes")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadRemotes()
                            root.openPopoverFrom(remoteManagerButton, remoteManagerPopover, 360)
                        }
                    }

                    IconButton {
                        id: configManagerButton
                        compact: true
                        icon: "settings-2"
                        tooltip: qsTr("Config")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadConfig()
                            root.openPopoverFrom(configManagerButton, configManagerPopover, 360)
                        }
                    }

                    IconButton {
                        id: rebaseManagerButton
                        compact: true
                        icon: "git-merge"
                        tooltip: qsTr("Interactive rebase")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadRebasePlan(root.rebaseCommitCount)
                            root.openPopoverFrom(rebaseManagerButton, rebaseManagerPopover, 420)
                        }
                    }

                    IconButton {
                        id: submoduleManagerButton
                        compact: true
                        icon: "layers"
                        tooltip: qsTr("Submodules")
                        enabled: client.isGitRepo
                        onClicked: {
                            client.loadSubmodules()
                            root.openPopoverFrom(submoduleManagerButton, submoduleManagerPopover, 392)
                        }
                    }

                    Rectangle {
                        Layout.preferredWidth: 1
                        Layout.preferredHeight: 14
                        color: Theme.borderSubtle
                    }

                    IconButton {
                        compact: true
                        icon: "download"
                        tooltip: qsTr("Pull")
                        enabled: client.isGitRepo && !client.busy
                        onClicked: client.pull()
                    }

                    IconButton {
                        compact: true
                        icon: "upload"
                        tooltip: qsTr("Push")
                        enabled: client.isGitRepo && !client.busy
                        onClicked: client.push()
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

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: client.currentBranch.length > 0
                              ? qsTr("Repository %1").arg(root.repoName)
                              : qsTr("Detached HEAD")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    StatusPill {
                        visible: client.aheadCount > 0
                        text: qsTr("Ahead %1").arg(client.aheadCount)
                        tone: "info"
                    }

                    StatusPill {
                        visible: client.behindCount > 0
                        text: qsTr("Behind %1").arg(client.behindCount)
                        tone: "warning"
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

        Item {
            Layout.fillWidth: true
            implicitHeight: 34

            Rectangle {
                anchors.left: parent.left
                anchors.leftMargin: Theme.sp3
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(parent.width - Theme.sp3 * 2, gitTabsRow.implicitWidth + Theme.sp1)
                height: 30
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1

                RowLayout {
                    id: gitTabsRow
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp0_5
                    anchors.rightMargin: Theme.sp0_5
                    spacing: Theme.sp0_5

                    Repeater {
                        model: root.navigationTabs

                        delegate: GitTabButton {
                            required property var modelData
                            title: modelData.label
                            icon: modelData.icon
                            badge: modelData.badge
                            active: root.selectedTab === modelData.idx
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

                                ToolSectionHeader {
                                    Layout.fillWidth: true
                                    title: qsTr("Commit")
                                    subtitle: client.stagedFiles.length > 0
                                              ? qsTr("%1 staged file(s) ready to commit").arg(client.stagedFiles.length)
                                              : qsTr("Stage changes to enable commit")
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

                                    RowLayout {
                                        spacing: Theme.sp1

                                        PrimaryButton {
                                            text: qsTr("Commit")
                                            enabled: commitMsg.text.trim().length > 0
                                                     && client.stagedFiles.length > 0
                                                     && !client.busy
                                            onClicked: {
                                                root.runPrimaryCommitAction(commitMsg.text)
                                                commitMsg.text = ""
                                            }
                                        }

                                        IconButton {
                                            id: commitActionsButton
                                            compact: true
                                            icon: "chevron-down"
                                            tooltip: qsTr("More commit actions")
                                            enabled: commitMsg.text.trim().length > 0
                                                     && client.stagedFiles.length > 0
                                                     && !client.busy
                                            onClicked: root.openPopoverFrom(commitActionsButton, commitActionsPopover, 188)
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

                    ToolHeroPanel {
                        Layout.fillWidth: true
                        accentColor: Theme.accent
                        padding: Theme.sp2

                        ToolSectionHeader {
                            Layout.fillWidth: true
                            title: qsTr("History")
                            subtitle: root.historyBranchFilter.length > 0
                                      ? qsTr("Graph filtered by %1").arg(root.historyBranchFilter)
                                      : qsTr("Branch graph for %1").arg(root.repoName)

                            StatusPill {
                                text: qsTr("%1 commits").arg(client.graphRows.length)
                                tone: "info"
                            }

                            StatusPill {
                                visible: root.historyFirstParent || root.historyNoMerges
                                text: root.historyFirstParent && root.historyNoMerges
                                      ? qsTr("First parent · No merges")
                                      : root.historyFirstParent
                                        ? qsTr("First parent")
                                        : qsTr("No merges")
                                tone: "neutral"
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            PierSearchField {
                                Layout.preferredWidth: 220
                                text: root.historySearchText
                                placeholder: qsTr("Search commit message or hash")
                                clearable: true
                                compact: true
                                onTextChanged: root.historySearchText = text
                            }

                            PierComboBox {
                                id: historyBranchCombo
                                Layout.preferredWidth: 164
                                options: root.branchFilterOptions()
                                placeholder: qsTr("All branches")
                                currentIndex: root.optionIndex(options, root.historyBranchFilter)
                                onActivated: (index) => {
                                    root.historyBranchFilter = index === 0 ? "" : String(options[index] || "")
                                }
                            }

                            PierComboBox {
                                id: historyAuthorCombo
                                Layout.preferredWidth: 142
                                options: root.authorFilterOptions()
                                placeholder: qsTr("All authors")
                                currentIndex: root.optionIndex(options, root.historyAuthorFilter)
                                onActivated: (index) => {
                                    root.historyAuthorFilter = index === 0 ? "" : String(options[index] || "")
                                }
                            }

                            PierComboBox {
                                id: historyDateCombo
                                Layout.preferredWidth: 132
                                options: root.historyDateOptions()
                                currentIndex: root.historyDateIndex(root.historyDateFilter)
                                onActivated: (index) => root.historyDateFilter = root.historyDateKeyAt(index)
                            }

                            GhostButton {
                                Layout.fillWidth: true
                                compact: true
                                text: root.historyPathSummary()
                                onClicked: {
                                    root.historyPathSearchText = ""
                                    root.historyPathSelection = root.historyFilterPaths()
                                    root.historyPathDialogOpen = true
                                }
                            }

                            IconButton {
                                visible: root.historyFilterPaths().length > 0
                                compact: true
                                icon: "x"
                                tooltip: qsTr("Clear path filter")
                                onClicked: root.historyPathFilter = ""
                            }

                            IconButton {
                                id: historyOptionsButton
                                compact: true
                                icon: "settings-2"
                                tooltip: qsTr("History options")
                                onClicked: root.openPopoverFrom(historyOptionsButton, historyOptionsPopover, 228)
                            }

                            IconButton {
                                compact: true
                                icon: "refresh-cw"
                                tooltip: qsTr("Reload graph")
                                onClicked: root.loadGraphTab()
                            }
                        }
                    }

                    SplitView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        orientation: Qt.Horizontal
                        handle: PierSplitHandle {
                            vertical: true
                        }

                        Rectangle {
                            SplitView.fillWidth: true
                            SplitView.preferredWidth: Math.max(420, parent.width * 0.62)
                            SplitView.minimumWidth: 320
                            color: Theme.bgSurface
                            border.color: Theme.borderSubtle
                            border.width: 1
                            radius: Theme.radiusMd
                            clip: true

                            ListView {
                                id: historyList
                                anchors.fill: parent
                                clip: true
                                boundsBehavior: Flickable.StopAtBounds
                                model: client.graphRows
                                spacing: 0

                                delegate: HistoryGraphRow {
                                    width: historyList.width
                                    rowData: modelData
                                    rowIndex: index
                                    selected: (rowData.hash || "") === root.historySelectedHash
                                    onClicked: {
                                        root.historySelectedHash = rowData.hash || ""
                                        root.historySelectedCommit = rowData
                                    }
                                }

                                EmptyStateCard {
                                    anchors.centerIn: parent
                                    width: Math.min(parent.width - Theme.sp8, 260)
                                    visible: client.graphRows.length === 0 && !client.busy
                                    title: qsTr("No history matches")
                                    description: qsTr("Adjust branch, author, date, path, or message filters to load commit graph data.")
                                    accentColor: Theme.accent
                                }
                            }
                        }

                        Rectangle {
                            SplitView.preferredWidth: 286
                            SplitView.minimumWidth: 248
                            SplitView.fillHeight: true
                            color: Theme.bgSurface
                            border.color: Theme.borderSubtle
                            border.width: 1
                            radius: Theme.radiusMd
                            clip: true

                            Item {
                                anchors.fill: parent

                                ColumnLayout {
                                    anchors.fill: parent
                                    anchors.margins: Theme.sp3
                                    spacing: Theme.sp2
                                    visible: !!(root.activeCommitDetail.hash)

                                    ToolSectionHeader {
                                        Layout.fillWidth: true
                                        title: qsTr("Commit detail")
                                        subtitle: root.activeCommitDetail.shortHash || root.activeCommitDetail.short_hash || ""

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Copy hash")
                                            onClicked: root.copyText(root.activeCommitDetail.hash || "")
                                        }
                                    }

                                    ToolPanelSurface {
                                        Layout.fillWidth: true
                                        inset: true

                                        ColumnLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp1_5

                                            Text {
                                                Layout.fillWidth: true
                                                text: root.activeCommitDetail.message || ""
                                                font.family: Theme.fontUi
                                                font.pixelSize: Theme.sizeBodyLg
                                                font.weight: Theme.weightSemibold
                                                color: Theme.textPrimary
                                                wrapMode: Text.WordWrap
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp1_5

                                                ToolFactChip {
                                                    label: qsTr("Author")
                                                    value: root.activeCommitDetail.author || ""
                                                }

                                                ToolFactChip {
                                                    label: qsTr("Date")
                                                    value: root.activeCommitDetail.date || root.formatGraphDate(root.historySelectedCommit.date_timestamp || 0)
                                                }

                                                ToolFactChip {
                                                    visible: String(root.activeCommitDetail.stats || "").length > 0
                                                    label: qsTr("Stats")
                                                    value: root.activeCommitDetail.stats || ""
                                                }
                                            }
                                        }
                                    }

                                    ToolPanelSurface {
                                        Layout.fillWidth: true
                                        inset: true

                                        ColumnLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp1_5

                                            ToolSectionHeader {
                                                Layout.fillWidth: true
                                                title: qsTr("References")
                                                subtitle: qsTr("Branches, HEAD, and tags on this commit")
                                            }

                                            Flow {
                                                Layout.fillWidth: true
                                                width: parent.width
                                                spacing: Theme.sp1

                                                Repeater {
                                                    model: root.refTokens(root.historySelectedCommit.refs || "")

                                                    delegate: RefBadge {
                                                        token: modelData
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    ToolPanelSurface {
                                        Layout.fillWidth: true
                                        inset: true

                                        ColumnLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp1_5

                                            ToolSectionHeader {
                                                Layout.fillWidth: true
                                                title: qsTr("Parents")
                                                subtitle: qsTr("Commit ancestry")
                                            }

                                            Repeater {
                                                model: (root.activeCommitDetail.parentHashes || []).length > 0
                                                       ? (root.activeCommitDetail.parentHashes || [])
                                                       : []

                                                delegate: Rectangle {
                                                    required property string modelData
                                                    width: parent.width
                                                    implicitHeight: 24
                                                    radius: Theme.radiusSm
                                                    color: Theme.bgInset
                                                    border.color: Theme.borderSubtle
                                                    border.width: 1

                                                    RowLayout {
                                                        anchors.fill: parent
                                                        anchors.leftMargin: Theme.sp2
                                                        anchors.rightMargin: Theme.sp2
                                                        spacing: Theme.sp2

                                                        Text {
                                                            Layout.fillWidth: true
                                                            text: modelData
                                                            font.family: Theme.fontMono
                                                            font.pixelSize: Theme.sizeCaption
                                                            color: Theme.textSecondary
                                                            elide: Text.ElideMiddle
                                                        }

                                                        GhostButton {
                                                            compact: true
                                                            minimumWidth: 0
                                                            text: qsTr("Copy")
                                                            onClicked: root.copyText(modelData)
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    ToolPanelSurface {
                                        Layout.fillWidth: true
                                        inset: true

                                        ColumnLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp1_5

                                            ToolSectionHeader {
                                                Layout.fillWidth: true
                                                title: qsTr("Actions")
                                                subtitle: qsTr("Common history operations")
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp1_5

                                                PierTextField {
                                                    Layout.fillWidth: true
                                                    text: root.historyBranchDraftName
                                                    placeholder: qsTr("Branch name from commit")
                                                    onTextChanged: root.historyBranchDraftName = text
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: qsTr("Branch")
                                                    enabled: root.historyBranchDraftName.trim().length > 0
                                                             && !!(root.activeCommitDetail.hash)
                                                             && !client.busy
                                                    onClicked: {
                                                        client.createBranchAt(root.historyBranchDraftName.trim(),
                                                                              root.activeCommitDetail.hash || "")
                                                        root.historyBranchDraftName = ""
                                                    }
                                                }
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp1_5

                                                PierTextField {
                                                    Layout.fillWidth: true
                                                    text: root.historyTagDraftName
                                                    placeholder: qsTr("Tag name")
                                                    onTextChanged: root.historyTagDraftName = text
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: qsTr("Tag")
                                                    enabled: root.historyTagDraftName.trim().length > 0 && !client.busy
                                                    onClicked: {
                                                        client.createTagAt(root.historyTagDraftName.trim(),
                                                                           root.activeCommitDetail.hash || "",
                                                                           root.historyTagDraftMessage.trim())
                                                        root.historyTagDraftName = ""
                                                        root.historyTagDraftMessage = ""
                                                    }
                                                }
                                            }

                                            PierTextField {
                                                Layout.fillWidth: true
                                                text: root.historyTagDraftMessage
                                                placeholder: qsTr("Annotated tag message (optional)")
                                                onTextChanged: root.historyTagDraftMessage = text
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp1_5

                                                SegmentedControl {
                                                    Layout.fillWidth: true
                                                    options: [qsTr("Soft"), qsTr("Mixed"), qsTr("Hard")]
                                                    currentIndex: root.historyResetMode === "soft" ? 0 : root.historyResetMode === "hard" ? 2 : 1
                                                    onActivated: (index) => root.historyResetMode = index === 0 ? "soft" : index === 2 ? "hard" : "mixed"
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: qsTr("Reset")
                                                    enabled: !!(root.activeCommitDetail.hash) && !client.busy
                                                    onClicked: client.resetToCommit(root.activeCommitDetail.hash || "", root.historyResetMode)
                                                }
                                            }

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: Theme.sp1_5
                                                visible: root.selectedCommitIsHead

                                                PierTextField {
                                                    Layout.fillWidth: true
                                                    text: root.historyAmendMessage
                                                    placeholder: qsTr("Amend HEAD message")
                                                    onTextChanged: root.historyAmendMessage = text
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: qsTr("Amend")
                                                    enabled: root.historyAmendMessage.trim().length > 0 && !client.busy
                                                    onClicked: client.amendHeadCommitMessage(root.activeCommitDetail.hash || "",
                                                                                            root.historyAmendMessage.trim())
                                                }
                                            }

                                            GhostButton {
                                                compact: true
                                                minimumWidth: 0
                                                text: qsTr("Drop commit")
                                                enabled: !!(root.activeCommitDetail.hash) && !client.busy
                                                onClicked: client.dropCommit(root.activeCommitDetail.hash || "",
                                                                             root.activeCommitDetail.parentHash || "")
                                            }
                                        }
                                    }

                                    ToolPanelSurface {
                                        Layout.fillWidth: true
                                        Layout.fillHeight: true
                                        inset: true

                                        ColumnLayout {
                                            anchors.fill: parent
                                            spacing: Theme.sp1_5

                                            ToolSectionHeader {
                                                Layout.fillWidth: true
                                                title: qsTr("Changed files")
                                                subtitle: qsTr("%1 files").arg((root.activeCommitDetail.changedFiles || []).length || 0)
                                            }

                                            Rectangle {
                                                Layout.fillWidth: true
                                                Layout.fillHeight: true
                                                radius: Theme.radiusSm
                                                color: Theme.bgInset
                                                border.color: Theme.borderSubtle
                                                border.width: 1
                                                clip: true

                                                Item {
                                                    anchors.fill: parent

                                                    PierScrollView {
                                                        anchors.fill: parent
                                                        visible: (root.activeCommitDetail.changedFiles || []).length > 0

                                                        Column {
                                                            width: parent.width

                                                            Repeater {
                                                                model: root.activeCommitDetail.changedFiles || []

                                                                delegate: Rectangle {
                                                                    required property var modelData
                                                                    width: parent.width
                                                                    implicitHeight: 34
                                                                    color: changedFileMouse.containsMouse ? Theme.bgHover : "transparent"

                                                                    RowLayout {
                                                                        anchors.fill: parent
                                                                        anchors.leftMargin: Theme.sp2
                                                                        anchors.rightMargin: Theme.sp2
                                                                        spacing: Theme.sp1_5

                                                                        Text {
                                                                            Layout.fillWidth: true
                                                                            text: modelData.path || ""
                                                                            font.family: Theme.fontMono
                                                                            font.pixelSize: Theme.sizeSmall
                                                                            color: Theme.textPrimary
                                                                            elide: Text.ElideMiddle
                                                                        }

                                                                        Text {
                                                                            visible: Number(modelData.additions || 0) > 0
                                                                            text: "+" + Number(modelData.additions || 0)
                                                                            font.family: Theme.fontMono
                                                                            font.pixelSize: Theme.sizeCaption
                                                                            color: Theme.statusSuccess
                                                                        }

                                                                        Text {
                                                                            visible: Number(modelData.deletions || 0) > 0
                                                                            text: "-" + Number(modelData.deletions || 0)
                                                                            font.family: Theme.fontMono
                                                                            font.pixelSize: Theme.sizeCaption
                                                                            color: Theme.statusError
                                                                        }

                                                                        GhostButton {
                                                                            visible: changedFileMouse.containsMouse
                                                                            compact: true
                                                                            minimumWidth: 0
                                                                            text: qsTr("Diff")
                                                                            onClicked: client.loadCommitFileDiff(root.activeCommitDetail.hash || "",
                                                                                                                 modelData.path || "")
                                                                        }
                                                                    }

                                                                    Rectangle {
                                                                        anchors.left: parent.left
                                                                        anchors.right: parent.right
                                                                        anchors.bottom: parent.bottom
                                                                        height: 1
                                                                        color: Theme.borderSubtle
                                                                    }

                                                                    MouseArea {
                                                                        id: changedFileMouse
                                                                        anchors.fill: parent
                                                                        hoverEnabled: true
                                                                        cursorShape: Qt.PointingHandCursor
                                                                        onClicked: client.loadCommitFileDiff(root.activeCommitDetail.hash || "",
                                                                                                              modelData.path || "")
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }

                                                    ToolEmptyState {
                                                        anchors.centerIn: parent
                                                        width: Math.min(parent.width - Theme.sp6, 220)
                                                        visible: (root.activeCommitDetail.changedFiles || []).length === 0
                                                        icon: "file-text"
                                                        title: qsTr("No changed files")
                                                        description: qsTr("Changed files for the selected commit will appear here.")
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                ToolEmptyState {
                                    anchors.centerIn: parent
                                    width: Math.min(parent.width - Theme.sp6, 248)
                                    visible: !root.activeCommitDetail.hash
                                    icon: "git-branch"
                                    title: qsTr("Select a commit")
                                    description: qsTr("Choose a row in the graph to inspect refs, author, and ancestry.")
                                }
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

                        ToolSectionHeader {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp3
                            anchors.rightMargin: Theme.sp3
                            title: qsTr("Stash")
                            subtitle: client.stashes.length > 0
                                      ? qsTr("%1 entries").arg(client.stashes.length)
                                      : qsTr("Snapshot unfinished work")

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
                                        color: Theme.accent
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

                    ToolHeroPanel {
                        Layout.fillWidth: true
                        accentColor: Theme.statusWarning

                        ToolSectionHeader {
                            Layout.fillWidth: true
                            title: qsTr("Conflicts")
                            subtitle: root.conflictFiles.length > 0
                                      ? qsTr("%1 conflicted file(s)").arg(root.conflictFiles.length)
                                      : qsTr("Files requiring manual merge resolution")

                            StatusPill {
                                text: root.conflictFiles.length > 0
                                      ? qsTr("%1 open").arg(root.conflictFiles.length)
                                      : qsTr("Clean")
                                tone: root.conflictFiles.length > 0 ? "warning" : "success"
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

                        SplitView {
                            anchors.fill: parent
                            orientation: Qt.Horizontal
                            handle: PierSplitHandle {
                                vertical: true
                            }
                            visible: root.conflictFiles.length > 0

                            Rectangle {
                                SplitView.preferredWidth: 238
                                SplitView.minimumWidth: 196
                                SplitView.fillHeight: true
                                color: Theme.bgInset
                                border.color: Theme.borderSubtle
                                border.width: 0
                                clip: true

                                ColumnLayout {
                                    anchors.fill: parent
                                    spacing: 0

                                    Rectangle {
                                        Layout.fillWidth: true
                                        implicitHeight: 34
                                        color: Theme.bgPanel

                                        ToolSectionHeader {
                                            anchors.fill: parent
                                            anchors.leftMargin: Theme.sp3
                                            anchors.rightMargin: Theme.sp3
                                            compact: true
                                            title: qsTr("Files")
                                            subtitle: qsTr("%1 open").arg(root.conflictFiles.length)
                                        }
                                    }

                                    PierScrollView {
                                        Layout.fillWidth: true
                                        Layout.fillHeight: true

                                        Column {
                                            width: parent.width

                                            Repeater {
                                                model: root.conflictFiles

                                                delegate: ConflictFileRow {
                                                    width: parent.width
                                                    fileData: modelData
                                                    selected: String((modelData.path || "")) === String(root.selectedConflictPath || "")
                                                    onOpenRequested: {
                                                        root.selectedConflictPath = fileData.path || ""
                                                        client.loadDiff(fileData.path || "", false)
                                                    }
                                                    onStageRequested: client.stageFile(fileData.path || "")
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                SplitView.fillWidth: true
                                SplitView.fillHeight: true
                                SplitView.minimumWidth: 280
                                color: Theme.bgSurface
                                border.color: Theme.borderSubtle
                                border.width: 0
                                clip: true

                                ColumnLayout {
                                    anchors.fill: parent
                                    anchors.margins: Theme.sp3
                                    spacing: Theme.sp2

                                    ToolHeroPanel {
                                        Layout.fillWidth: true
                                        compact: true
                                        accentColor: Theme.statusWarning

                                        ToolSectionHeader {
                                            Layout.fillWidth: true
                                            title: (root.selectedConflictFile.name || root.selectedConflictFile.path || qsTr("Resolution"))
                                            subtitle: root.selectedConflictFile.path || qsTr("Select a conflicted file to resolve hunks")

                                            StatusPill {
                                                visible: !!(root.selectedConflictFile.path)
                                                text: qsTr("%1 hunks").arg((root.selectedConflictFile.conflicts || []).length || 0)
                                                tone: "warning"
                                            }
                                        }
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: Theme.sp1_5
                                        visible: !!(root.selectedConflictFile.path)

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Diff")
                                            onClicked: client.loadDiff(root.selectedConflictFile.path || "", false)
                                        }

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Accept all ours")
                                            onClicked: client.acceptAllOurs(root.selectedConflictFile.path || "")
                                        }

                                        GhostButton {
                                            compact: true
                                            minimumWidth: 0
                                            text: qsTr("Accept all theirs")
                                            onClicked: client.acceptAllTheirs(root.selectedConflictFile.path || "")
                                        }

                                        Item { Layout.fillWidth: true }

                                        PrimaryButton {
                                            compact: true
                                            text: qsTr("Mark resolved")
                                            onClicked: client.markConflictResolved(root.selectedConflictFile.path || "")
                                        }
                                    }

                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.fillHeight: true
                                        radius: Theme.radiusMd
                                        color: Theme.bgInset
                                        border.color: Theme.borderSubtle
                                        border.width: 1
                                        clip: true

                                        Item {
                                            anchors.fill: parent

                                            PierScrollView {
                                                anchors.fill: parent
                                                visible: !!(root.selectedConflictFile.path)

                                                Column {
                                                    width: parent.width
                                                    spacing: Theme.sp2

                                                    Repeater {
                                                        model: root.selectedConflictFile.conflicts || []

                                                        delegate: ConflictHunkCard {
                                                            width: parent.width
                                                            filePath: root.selectedConflictFile.path || ""
                                                            hunkIndex: index
                                                            hunkData: modelData
                                                        }
                                                    }
                                                }
                                            }

                                            ToolEmptyState {
                                                anchors.centerIn: parent
                                                width: Math.min(parent.width - Theme.sp8, 260)
                                                visible: !root.selectedConflictFile.path
                                                icon: "git-merge"
                                                title: qsTr("Select a conflict")
                                                description: qsTr("Choose a conflicted file to inspect ours, theirs, and apply a resolution.")
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        ToolEmptyState {
                            anchors.centerIn: parent
                            width: Math.min(parent.width - Theme.sp8, 280)
                            visible: root.conflictFiles.length === 0
                            icon: "git-merge"
                            title: qsTr("No merge conflicts")
                            description: qsTr("Conflicted files will appear here when Git requires manual resolution.")
                        }
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: historyOptionsPopover
        width: 228

        Column {
            width: historyOptionsPopover.width - historyOptionsPopover.padding * 2
            spacing: Theme.sp1

            Text {
                width: parent.width
                text: qsTr("Sort")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Topology order")
                active: root.historySortMode === "topo"
                onClicked: {
                    root.historySortMode = "topo"
                    historyOptionsPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Date order")
                active: root.historySortMode === "date"
                onClicked: {
                    root.historySortMode = "date"
                    historyOptionsPopover.close()
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            Text {
                width: parent.width
                text: qsTr("Graph options")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("First parent only")
                active: root.historyFirstParent
                onClicked: root.historyFirstParent = !root.historyFirstParent
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Hide merge commits")
                active: root.historyNoMerges
                onClicked: root.historyNoMerges = !root.historyNoMerges
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Expand long edges")
                active: root.historyShowLongEdges
                onClicked: root.historyShowLongEdges = !root.historyShowLongEdges
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            Text {
                width: parent.width
                text: qsTr("Highlight")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("No highlight")
                active: root.historyHighlightMode === "none"
                onClicked: root.historyHighlightMode = "none"
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("My commits")
                active: root.historyHighlightMode === "mine"
                onClicked: root.historyHighlightMode = "mine"
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Merge commits")
                active: root.historyHighlightMode === "merge"
                onClicked: root.historyHighlightMode = "merge"
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Current branch")
                active: root.historyHighlightMode === "branch"
                onClicked: root.historyHighlightMode = "branch"
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            Text {
                width: parent.width
                text: qsTr("Display")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textTertiary
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Zebra stripes")
                active: root.historyShowZebraStripes
                onClicked: root.historyShowZebraStripes = !root.historyShowZebraStripes
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Show hash column")
                active: root.historyShowHash
                onClicked: root.historyShowHash = !root.historyShowHash
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Show author column")
                active: root.historyShowAuthor
                onClicked: root.historyShowAuthor = !root.historyShowAuthor
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Show date column")
                active: root.historyShowDate
                onClicked: root.historyShowDate = !root.historyShowDate
            }
        }
    }

    PopoverPanel {
        id: commitActionsPopover
        width: 188

        Column {
            width: commitActionsPopover.width - commitActionsPopover.padding * 2
            spacing: Theme.sp1

            PierMenuItem {
                width: parent.width
                text: qsTr("Commit")
                enabled: commitMsg.text.trim().length > 0
                         && client.stagedFiles.length > 0
                         && !client.busy
                onClicked: {
                    root.runPrimaryCommitAction(commitMsg.text)
                    commitMsg.text = ""
                    commitActionsPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Commit & Push")
                enabled: commitMsg.text.trim().length > 0
                         && client.stagedFiles.length > 0
                         && !client.busy
                onClicked: {
                    root.runCommitAndPushAction(commitMsg.text)
                    commitMsg.text = ""
                    commitActionsPopover.close()
                }
            }
        }
    }

    PopoverPanel {
        id: historyCommitPopover
        width: 232

        Column {
            width: historyCommitPopover.width - historyCommitPopover.padding * 2
            spacing: Theme.sp1

            PierMenuItem {
                width: parent.width
                text: qsTr("Copy hash")
                onClicked: {
                    root.copyText(root.historyContextCommit.hash || "")
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Checkout this revision")
                onClicked: {
                    client.checkoutTarget(root.historyContextCommit.hash || "")
                    historyCommitPopover.close()
                }
            }

            Repeater {
                model: root.historyContextCheckoutTargets().slice(1)

                delegate: PierMenuItem {
                    required property var modelData
                    width: parent.width
                    text: modelData.label || ""
                    onClicked: {
                        client.checkoutTarget(String(modelData.target || ""),
                                              String(modelData.tracking || ""))
                        historyCommitPopover.close()
                    }
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Compare with local")
                onClicked: {
                    root.historyCompareSelectedPath = ""
                    client.loadComparisonFiles(root.historyContextCommit.hash || "")
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Open in browser")
                enabled: root.browserUrlForCommit(root.historyContextCommit.hash || "").length > 0
                onClicked: {
                    root.openCommitInBrowser(root.historyContextCommit.hash || "")
                    historyCommitPopover.close()
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Create branch from commit")
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, false)
                    root.historyBranchDialogOpen = true
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Create tag from commit")
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, false)
                    root.historyTagDialogOpen = true
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Reset current branch")
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, false)
                    root.historyResetDialogOpen = true
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Undo commit")
                enabled: root.historyContextIsHead() && root.historyContextParentHash().length > 0 && !client.busy
                onClicked: {
                    client.resetToCommit(root.historyContextParentHash(), "soft")
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Edit commit message")
                enabled: root.historyContextIsHead() && !client.busy
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, false)
                    root.historyAmendMessage = client.commitDetail.message || root.historyContextCommit.message || ""
                    root.historyEditMessageDialogOpen = true
                    historyCommitPopover.close()
                }
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Drop commit")
                enabled: !!(root.historyContextCommit.hash) && !client.busy
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, false)
                    root.historyDropDialogOpen = true
                    historyCommitPopover.close()
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            PierMenuItem {
                width: parent.width
                text: qsTr("Open in detail")
                onClicked: {
                    root.selectHistoryCommit(root.historyContextCommit, true)
                    historyCommitPopover.close()
                }
            }
        }
    }

    ModalDialogShell {
        id: historyPathDialog
        open: root.historyPathDialogOpen
        title: qsTr("Tracked files")
        subtitle: qsTr("Filter commit graph to specific repository paths")
        dialogWidth: 640
        dialogHeight: 620
        bodyPadding: Theme.sp4
        onRequestClose: root.historyPathDialogOpen = false

        body: Item {
            anchors.fill: parent

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp3

                PierSearchField {
                    Layout.fillWidth: true
                    text: root.historyPathSearchText
                    placeholder: qsTr("Search tracked files")
                    clearable: true
                    compact: true
                    onTextChanged: root.historyPathSearchText = text
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    inset: true

                    Item {
                        anchors.fill: parent

                        PierScrollView {
                            anchors.fill: parent
                            visible: root.historyFilteredRepoFiles().length > 0

                            Column {
                                width: parent.width

                                Repeater {
                                    model: root.historyFilteredRepoFiles()

                                    delegate: Rectangle {
                                        id: pathRowRoot
                                        required property string modelData
                                        width: parent.width
                                        implicitHeight: 30
                                        readonly property bool selected: root.historyPathSelection.indexOf(modelData) >= 0
                                        color: selected ? Theme.bgSelected : pathRowMouse.containsMouse ? Theme.bgHover : "transparent"

                                        RowLayout {
                                            anchors.fill: parent
                                            anchors.leftMargin: Theme.sp2
                                            anchors.rightMargin: Theme.sp2
                                            spacing: Theme.sp2

                                            Rectangle {
                                                width: 16
                                                height: 16
                                                radius: Theme.radiusSm
                                                color: pathRowRoot.selected ? Theme.accentMuted : Theme.bgInset
                                                border.color: pathRowRoot.selected ? Theme.borderFocus : Theme.borderSubtle
                                                border.width: 1

                                                Image {
                                                    anchors.centerIn: parent
                                                    visible: pathRowRoot.selected
                                                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/check.svg"
                                                    sourceSize: Qt.size(10, 10)
                                                }
                                            }

                                            Text {
                                                Layout.fillWidth: true
                                                text: modelData
                                                font.family: Theme.fontMono
                                                font.pixelSize: Theme.sizeBody
                                                color: Theme.textPrimary
                                                elide: Text.ElideMiddle
                                            }
                                        }

                                        MouseArea {
                                            id: pathRowMouse
                                            anchors.fill: parent
                                            hoverEnabled: true
                                            onClicked: {
                                                const copy = root.historyPathSelection.slice()
                                                const idx = copy.indexOf(modelData)
                                                if (idx >= 0)
                                                    copy.splice(idx, 1)
                                                else
                                                    copy.push(modelData)
                                                root.historyPathSelection = copy
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        ToolEmptyState {
                            anchors.centerIn: parent
                            visible: root.historyFilteredRepoFiles().length === 0
                            icon: "folder"
                            title: qsTr("No tracked files")
                            description: qsTr("Try a different search or refresh repository metadata.")
                        }
                    }
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Clear")
                onClicked: root.historyPathSelection = []
            }

            Item { Layout.fillWidth: true }

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: root.historyPathDialogOpen = false
            }

            PrimaryButton {
                compact: true
                text: qsTr("Apply")
                onClicked: {
                    root.historyPathFilter = root.historyPathSelection.join("\n")
                    root.historyPathDialogOpen = false
                }
            }
        }
    }

    ModalDialogShell {
        id: historyCompareDialog
        open: root.historyCompareDialogOpen
        title: qsTr("Compare with local")
        subtitle: client.comparisonBaseHash.length > 0 ? client.comparisonBaseHash : qsTr("Commit comparison")
        dialogWidth: 980
        dialogHeight: 660
        bodyPadding: Theme.sp4
        onRequestClose: {
            root.historyCompareDialogOpen = false
            root.historyCompareSelectedPath = ""
            client.clearComparison()
        }

        body: SplitView {
            anchors.fill: parent
            orientation: Qt.Horizontal
            handle: PierSplitHandle { vertical: true }

            ToolPanelSurface {
                SplitView.preferredWidth: 268
                SplitView.minimumWidth: 220
                inset: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: client.comparisonFiles.length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: client.comparisonFiles

                                delegate: Rectangle {
                                    required property var modelData
                                    width: parent.width
                                    implicitHeight: 42
                                    readonly property bool selected: String(modelData.path || "") === String(root.historyCompareSelectedPath || "")
                                    color: selected ? Theme.bgSelected : compareFileMouse.containsMouse ? Theme.bgHover : "transparent"

                                    ColumnLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp2
                                        anchors.rightMargin: Theme.sp2
                                        anchors.topMargin: Theme.sp1
                                        anchors.bottomMargin: Theme.sp1
                                        spacing: 0

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData.name || modelData.path || ""
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeBody
                                            font.weight: Theme.weightMedium
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: modelData.path || ""
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeCaption
                                            color: Theme.textTertiary
                                            elide: Text.ElideMiddle
                                        }
                                    }

                                    MouseArea {
                                        id: compareFileMouse
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        onClicked: root.historyCompareSelectedPath = String(modelData.path || "")
                                    }
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: client.comparisonFiles.length === 0 && !client.busy
                        icon: "git-compare"
                        title: qsTr("No local diff")
                        description: qsTr("This commit matches local HEAD, or there are no comparable files.")
                    }
                }
            }

            ToolPanelSurface {
                SplitView.fillWidth: true
                SplitView.minimumWidth: 360
                inset: true

                PierTextArea {
                    anchors.fill: parent
                    readOnly: true
                    mono: true
                    frameVisible: false
                    text: client.comparisonDiff
                    placeholderText: qsTr("Select a changed file to inspect the diff against local HEAD.")
                }
            }
        }
    }

    ModalDialogShell {
        id: historyBranchDialog
        open: root.historyBranchDialogOpen
        title: qsTr("Create branch from commit")
        subtitle: qsTr("Create a branch that starts at this commit")
        dialogWidth: 520
        dialogHeight: 268
        bodyPadding: Theme.sp4
        onRequestClose: {
            root.historyBranchDialogOpen = false
            root.historyBranchDraftName = ""
        }

        body: ToolPanelSurface {
            anchors.fill: parent
            inset: true

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    title: root.activeCommitDetail.shortHash || root.historyContextCommit.shortHash || qsTr("Commit")
                    subtitle: root.activeCommitDetail.message || root.historyContextCommit.message || ""
                }

                PierTextField {
                    Layout.fillWidth: true
                    text: root.historyBranchDraftName
                    placeholder: qsTr("Branch name")
                    onTextChanged: root.historyBranchDraftName = text
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: historyBranchDialog.requestClose()
            }

            Item { Layout.fillWidth: true }

            PrimaryButton {
                compact: true
                text: qsTr("Create branch")
                enabled: root.historyBranchDraftName.trim().length > 0
                         && !!(root.historyContextCommit.hash)
                         && !client.busy
                onClicked: {
                    client.createBranchAt(root.historyBranchDraftName.trim(),
                                          root.historyContextCommit.hash || "")
                    historyBranchDialog.requestClose()
                }
            }
        }
    }

    ModalDialogShell {
        id: historyTagDialog
        open: root.historyTagDialogOpen
        title: qsTr("Create tag from commit")
        subtitle: qsTr("Create a lightweight or annotated tag at this commit")
        dialogWidth: 560
        dialogHeight: 360
        bodyPadding: Theme.sp4
        onRequestClose: {
            root.historyTagDialogOpen = false
            root.historyTagDraftName = ""
            root.historyTagDraftMessage = ""
        }

        body: ToolPanelSurface {
            anchors.fill: parent
            inset: true

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    title: root.activeCommitDetail.shortHash || root.historyContextCommit.shortHash || qsTr("Commit")
                    subtitle: root.activeCommitDetail.message || root.historyContextCommit.message || ""
                }

                PierTextField {
                    Layout.fillWidth: true
                    text: root.historyTagDraftName
                    placeholder: qsTr("Tag name")
                    onTextChanged: root.historyTagDraftName = text
                }

                PierTextArea {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    text: root.historyTagDraftMessage
                    placeholderText: qsTr("Annotated tag message (optional)")
                    onTextChanged: root.historyTagDraftMessage = text
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: historyTagDialog.requestClose()
            }

            Item { Layout.fillWidth: true }

            PrimaryButton {
                compact: true
                text: qsTr("Create tag")
                enabled: root.historyTagDraftName.trim().length > 0
                         && !!(root.historyContextCommit.hash)
                         && !client.busy
                onClicked: {
                    client.createTagAt(root.historyTagDraftName.trim(),
                                       root.historyContextCommit.hash || "",
                                       root.historyTagDraftMessage.trim())
                    historyTagDialog.requestClose()
                }
            }
        }
    }

    ModalDialogShell {
        id: historyResetDialog
        open: root.historyResetDialogOpen
        title: qsTr("Reset current branch")
        subtitle: qsTr("Move the current branch pointer to this commit")
        dialogWidth: 560
        dialogHeight: 360
        bodyPadding: Theme.sp4
        onRequestClose: root.historyResetDialogOpen = false

        body: ToolPanelSurface {
            anchors.fill: parent
            inset: true

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    title: qsTr("Reset mode")
                    subtitle: qsTr("Soft keeps changes staged, mixed keeps changes unstaged, hard discards working tree changes.")
                }

                SegmentedControl {
                    Layout.fillWidth: true
                    options: [qsTr("Soft"), qsTr("Mixed"), qsTr("Hard")]
                    currentIndex: root.historyResetMode === "soft" ? 0 : root.historyResetMode === "hard" ? 2 : 1
                    onActivated: (index) => root.historyResetMode = index === 0 ? "soft" : index === 2 ? "hard" : "mixed"
                }

                ToolBanner {
                    Layout.fillWidth: true
                    tone: root.historyResetMode === "hard" ? "warning" : "info"
                    text: root.historyResetMode === "hard"
                          ? qsTr("Hard reset will discard working tree changes.")
                          : root.historyResetMode === "soft"
                            ? qsTr("Soft reset keeps all changes staged for recommit.")
                            : qsTr("Mixed reset keeps changes in the working tree but unstaged.")
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    inset: true

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: Theme.sp1

                        Text {
                            Layout.fillWidth: true
                            text: root.activeCommitDetail.shortHash || root.historyContextCommit.shortHash || ""
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.accent
                            elide: Text.ElideRight
                        }

                        Text {
                            Layout.fillWidth: true
                            text: root.activeCommitDetail.message || root.historyContextCommit.message || ""
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            color: Theme.textPrimary
                            wrapMode: Text.Wrap
                        }
                    }
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: historyResetDialog.requestClose()
            }

            Item { Layout.fillWidth: true }

            PrimaryButton {
                compact: true
                text: qsTr("Apply reset")
                enabled: !!(root.historyContextCommit.hash) && !client.busy
                onClicked: {
                    client.resetToCommit(root.historyContextCommit.hash || "", root.historyResetMode)
                    historyResetDialog.requestClose()
                }
            }
        }
    }

    ModalDialogShell {
        id: historyEditMessageDialog
        open: root.historyEditMessageDialogOpen
        title: qsTr("Edit commit message")
        subtitle: qsTr("Amend the HEAD commit message")
        dialogWidth: 620
        dialogHeight: 420
        bodyPadding: Theme.sp4
        onRequestClose: root.historyEditMessageDialogOpen = false

        body: ToolPanelSurface {
            anchors.fill: parent
            inset: true

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                ToolBanner {
                    Layout.fillWidth: true
                    tone: "info"
                    text: qsTr("The HEAD commit will be amended with the message below.")
                }

                PierTextArea {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    text: root.historyAmendMessage
                    placeholderText: qsTr("Update commit message")
                    onTextChanged: root.historyAmendMessage = text
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: historyEditMessageDialog.requestClose()
            }

            Item { Layout.fillWidth: true }

            PrimaryButton {
                compact: true
                text: qsTr("Edit message")
                enabled: root.historyContextIsHead()
                         && root.historyAmendMessage.trim().length > 0
                         && !client.busy
                onClicked: {
                    client.amendHeadCommitMessage(root.historyContextCommit.hash || "",
                                                  root.historyAmendMessage.trim())
                    historyEditMessageDialog.requestClose()
                }
            }
        }
    }

    ModalDialogShell {
        id: historyDropDialog
        open: root.historyDropDialogOpen
        title: qsTr("Drop commit")
        subtitle: qsTr("Remove this commit from history")
        dialogWidth: 560
        dialogHeight: 320
        bodyPadding: Theme.sp4
        onRequestClose: root.historyDropDialogOpen = false

        body: ToolPanelSurface {
            anchors.fill: parent
            inset: true

            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                ToolBanner {
                    Layout.fillWidth: true
                    tone: "warning"
                    text: qsTr("This will permanently rewrite Git history for the current branch.")
                }

                Text {
                    Layout.fillWidth: true
                    text: root.historyContextIsHead()
                          ? qsTr("The current HEAD commit will be removed by resetting to its parent.")
                          : qsTr("This non-HEAD commit will be removed using rebase --onto.")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textSecondary
                    wrapMode: Text.Wrap
                }
            }
        }

        footer: RowLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            GhostButton {
                compact: true
                text: qsTr("Cancel")
                onClicked: historyDropDialog.requestClose()
            }

            Item { Layout.fillWidth: true }

            PrimaryButton {
                compact: true
                destructive: true
                text: qsTr("Drop")
                enabled: !!(root.historyContextCommit.hash) && !client.busy
                onClicked: {
                    client.dropCommit(root.historyContextCommit.hash || "",
                                      root.historyContextParentHash())
                    historyDropDialog.requestClose()
                }
            }
        }
    }

    PopoverPanel {
        id: branchManagerPopover
        width: 352
        onOpened: {
            client.loadGraphMetadata()
            root.ensureTrackingDefaults()
        }
        onClosed: {
            root.branchManagerMode = "local"
            root.branchManagerSearchText = ""
            root.branchCreateExpanded = false
            root.branchDraftName = ""
            root.branchRenameSource = ""
            root.branchRenameTarget = ""
        }

        ColumnLayout {
            width: branchManagerPopover.width - branchManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Branches")
                subtitle: qsTr("Create, switch, rename, and manage tracking")

                IconButton {
                    compact: true
                    icon: root.branchCreateExpanded ? "x" : "plus"
                    tooltip: root.branchCreateExpanded ? qsTr("Hide composer") : qsTr("New branch")
                    onClicked: root.branchCreateExpanded = !root.branchCreateExpanded
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload branches")
                    onClicked: {
                        client.loadBranches()
                        client.loadGraphMetadata()
                    }
                }
            }

            SegmentedControl {
                Layout.fillWidth: true
                options: [qsTr("Local"), qsTr("Remote")]
                currentIndex: root.branchManagerMode === "remote" ? 1 : 0
                onActivated: (index) => root.branchManagerMode = index === 1 ? "remote" : "local"
            }

            PierSearchField {
                Layout.fillWidth: true
                text: root.branchManagerSearchText
                placeholder: qsTr("Filter branches")
                clearable: true
                compact: true
                onTextChanged: root.branchManagerSearchText = text
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                visible: root.branchManagerMode === "local" && root.branchCreateExpanded
                inset: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1_5

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        title: qsTr("Create branch")
                        subtitle: qsTr("Create a local branch from the current HEAD")
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1_5

                        PierTextField {
                            Layout.fillWidth: true
                            text: root.branchDraftName
                            placeholder: qsTr("Branch name")
                            onTextChanged: root.branchDraftName = text
                        }

                        PrimaryButton {
                            compact: true
                            text: qsTr("Create")
                            enabled: root.branchDraftName.trim().length > 0 && !client.busy
                            onClicked: {
                                client.createBranch(root.branchDraftName.trim())
                                root.branchDraftName = ""
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        height: 1
                        color: Theme.borderSubtle
                    }

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        title: qsTr("Tracking")
                        subtitle: qsTr("Set or remove upstream for a local branch")
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1_5

                        PierComboBox {
                            id: trackingBranchCombo
                            Layout.fillWidth: true
                            options: root.managerBranchList(true)
                            placeholder: qsTr("Local branch")
                            currentIndex: root.optionIndex(options, root.trackingBranchTarget)
                            onActivated: (index) => root.trackingBranchTarget = index === 0 && options.length === 0
                                                                       ? ""
                                                                       : String(options[index] || "")
                        }

                        PierComboBox {
                            id: trackingUpstreamCombo
                            Layout.fillWidth: true
                            options: root.managerBranchList(false)
                            placeholder: qsTr("Remote branch")
                            currentIndex: root.optionIndex(options, root.trackingUpstreamTarget)
                            onActivated: (index) => root.trackingUpstreamTarget = index === 0 && options.length === 0
                                                                         ? ""
                                                                         : String(options[index] || "")
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1_5

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Unset")
                            enabled: root.trackingBranchTarget.length > 0 && !client.busy
                            onClicked: client.unsetBranchTracking(root.trackingBranchTarget)
                        }

                        Item { Layout.fillWidth: true }

                        PrimaryButton {
                            compact: true
                            text: qsTr("Set tracking")
                            enabled: root.trackingBranchTarget.length > 0
                                     && root.trackingUpstreamTarget.length > 0
                                     && !client.busy
                            onClicked: client.setBranchTracking(root.trackingBranchTarget, root.trackingUpstreamTarget)
                        }
                    }
                }
            }

            ToolSectionHeader {
                Layout.fillWidth: true
                visible: root.branchManagerMode === "local"
                title: qsTr("Local branches")
                subtitle: qsTr("%1 branches").arg(root.filteredManagerBranchList(true).length)
            }

            Rectangle {
                Layout.fillWidth: true
                visible: root.branchManagerMode === "local"
                implicitHeight: 214
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.filteredManagerBranchList(true).length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.filteredManagerBranchList(true)

                                delegate: BranchManagerRow {
                                    width: parent.width
                                    branchName: modelData
                                    trackingName: modelData === client.currentBranch ? client.trackingBranch : ""
                                    current: modelData === client.currentBranch
                                    remote: false
                                    renameMode: root.branchRenameSource === modelData
                                    renameText: root.branchRenameSource === modelData ? root.branchRenameTarget : ""
                                    onRenameTextChanged: (value) => root.branchRenameTarget = value
                                    onCheckoutClicked: client.checkoutBranch(branchName)
                                    onMergeClicked: client.mergeBranch(branchName)
                                    onRenameRequested: {
                                        root.branchRenameSource = branchName
                                        root.branchRenameTarget = branchName
                                    }
                                    onRenameSubmitted: {
                                        if (renameText.trim().length > 0)
                                            client.renameBranch(branchName, renameText.trim())
                                        root.branchRenameSource = ""
                                        root.branchRenameTarget = ""
                                    }
                                    onRenameCancelled: {
                                        root.branchRenameSource = ""
                                        root.branchRenameTarget = ""
                                    }
                                    onDeleteClicked: client.deleteBranch(branchName)
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.filteredManagerBranchList(true).length === 0
                        icon: "git-branch"
                        title: qsTr("No local branches")
                        description: qsTr("Create a branch to start parallel workstreams.")
                    }
                }
            }

            ToolSectionHeader {
                Layout.fillWidth: true
                visible: root.branchManagerMode === "remote"
                title: qsTr("Remote branches")
                subtitle: qsTr("%1 refs").arg(root.filteredManagerBranchList(false).length)
            }

            Rectangle {
                Layout.fillWidth: true
                visible: root.branchManagerMode === "remote"
                implicitHeight: 144
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.filteredManagerBranchList(false).length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.filteredManagerBranchList(false)

                                delegate: BranchManagerRow {
                                    width: parent.width
                                    branchName: modelData
                                    trackingName: ""
                                    current: false
                                    remote: true
                                    renameMode: root.branchRenameSource === branchName
                                    renameText: root.branchRenameSource === branchName ? root.branchRenameTarget : ""
                                    onRenameTextChanged: (value) => root.branchRenameTarget = value
                                    onCheckoutClicked: {
                                        const localName = String(branchName).replace(/^[^\/]+\//, "")
                                        client.checkoutTarget(localName, branchName)
                                    }
                                    onRenameRequested: {
                                        root.branchRenameSource = branchName
                                        root.branchRenameTarget = String(branchName).replace(/^[^\/]+\//, "")
                                    }
                                    onRenameSubmitted: {
                                        const parts = String(branchName).split("/")
                                        const remoteName = parts.shift() || "origin"
                                        const remoteBranch = parts.join("/")
                                        if (renameText.trim().length > 0)
                                            client.renameRemoteBranch(remoteName, remoteBranch, renameText.trim())
                                        root.branchRenameSource = ""
                                        root.branchRenameTarget = ""
                                    }
                                    onRenameCancelled: {
                                        root.branchRenameSource = ""
                                        root.branchRenameTarget = ""
                                    }
                                    onDeleteClicked: {
                                        const parts = String(branchName).split("/")
                                        const remoteName = parts.shift() || "origin"
                                        const remoteBranch = parts.join("/")
                                        client.deleteRemoteBranch(remoteName, remoteBranch)
                                    }
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.filteredManagerBranchList(false).length === 0
                        icon: "git-branch"
                        title: qsTr("No remote branches")
                        description: qsTr("Remote refs will appear here after fetch or clone.")
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: tagManagerPopover
        width: 344
        onOpened: client.loadTags()
        onClosed: {
            root.tagCreateExpanded = false
            root.tagDraftName = ""
            root.tagDraftMessage = ""
            root.tagSearchText = ""
        }

        ColumnLayout {
            width: tagManagerPopover.width - tagManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Tags")
                subtitle: qsTr("Create, push, and delete release markers")

                IconButton {
                    compact: true
                    icon: root.tagCreateExpanded ? "x" : "plus"
                    tooltip: root.tagCreateExpanded ? qsTr("Hide composer") : qsTr("New tag")
                    onClicked: root.tagCreateExpanded = !root.tagCreateExpanded
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Push all")
                    enabled: client.tags.length > 0 && !client.busy
                    onClicked: client.pushAllTags()
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload tags")
                    onClicked: client.loadTags()
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                visible: root.tagCreateExpanded
                inset: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1_5

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.tagDraftName
                        placeholder: qsTr("Tag name")
                        onTextChanged: root.tagDraftName = text
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.tagDraftMessage
                        placeholder: qsTr("Tag message (optional)")
                        onTextChanged: root.tagDraftMessage = text
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Item { Layout.fillWidth: true }

                        PrimaryButton {
                            compact: true
                            text: qsTr("Create tag")
                            enabled: root.tagDraftName.trim().length > 0 && !client.busy
                            onClicked: {
                                client.createTag(root.tagDraftName.trim(), root.tagDraftMessage.trim())
                                root.tagDraftName = ""
                                root.tagDraftMessage = ""
                            }
                        }
                    }
                }
            }

            PierSearchField {
                Layout.fillWidth: true
                text: root.tagSearchText
                placeholder: qsTr("Filter tags")
                clearable: true
                compact: true
                onTextChanged: root.tagSearchText = text
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 232
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.filteredTagEntries().length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.filteredTagEntries()

                                delegate: ManagerInfoRow {
                                    width: parent.width
                                    leadingIcon: "tag"
                                    accentColor: "#f59e0b"
                                    title: modelData.name || ""
                                    subtitle: modelData.message || ""
                                    metaText: modelData.hash || ""
                                    primaryActionText: qsTr("Push")
                                    secondaryActionText: qsTr("Copy hash")
                                    tertiaryActionText: qsTr("Delete")
                                    onPrimaryAction: client.pushTag(modelData.name || "")
                                    onSecondaryAction: root.copyText(modelData.hash || "")
                                    onTertiaryAction: client.deleteTag(modelData.name || "")
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.filteredTagEntries().length === 0
                        icon: "tag"
                        title: qsTr("No tags")
                        description: qsTr("Create release or checkpoint tags for this repository.")
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: remoteManagerPopover
        width: 372
        onOpened: client.loadRemotes()
        onClosed: {
            root.clearRemoteDraft()
            root.remoteSearchText = ""
        }

        ColumnLayout {
            width: remoteManagerPopover.width - remoteManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Remotes")
                subtitle: root.remoteEditSourceName.length > 0
                          ? qsTr("Update fetch/push URL for %1").arg(root.remoteEditSourceName)
                          : qsTr("Manage upstream repository endpoints")

                IconButton {
                    compact: true
                    icon: (root.remoteComposerExpanded || root.remoteEditSourceName.length > 0) ? "x" : "plus"
                    tooltip: (root.remoteComposerExpanded || root.remoteEditSourceName.length > 0)
                             ? qsTr("Hide composer")
                             : qsTr("Add remote")
                    onClicked: {
                        if (root.remoteEditSourceName.length > 0)
                            root.clearRemoteDraft()
                        else
                            root.remoteComposerExpanded = !root.remoteComposerExpanded
                    }
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload remotes")
                    onClicked: client.loadRemotes()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Fetch all")
                    enabled: client.isGitRepo && !client.busy
                    onClicked: client.fetchRemote("")
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                visible: root.remoteComposerExpanded || root.remoteEditSourceName.length > 0
                inset: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1_5

                    ToolBanner {
                        Layout.fillWidth: true
                        visible: root.remoteEditSourceName.length > 0
                        tone: "info"
                        text: qsTr("Editing remote %1.").arg(root.remoteEditSourceName)
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.remoteDraftName
                        placeholder: qsTr("Remote name")
                        enabled: root.remoteEditSourceName.length === 0
                        onTextChanged: root.remoteDraftName = text
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.remoteDraftUrl
                        placeholder: qsTr("Remote URL")
                        onTextChanged: root.remoteDraftUrl = text
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1_5

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            visible: root.remoteEditSourceName.length > 0
                            text: qsTr("Cancel edit")
                            onClicked: root.clearRemoteDraft()
                        }

                        Item { Layout.fillWidth: true }

                        PrimaryButton {
                            compact: true
                            text: root.remoteEditSourceName.length > 0 ? qsTr("Update remote") : qsTr("Add remote")
                            enabled: root.remoteDraftName.trim().length > 0
                                     && root.remoteDraftUrl.trim().length > 0
                                     && !client.busy
                            onClicked: {
                                if (root.remoteEditSourceName.length > 0)
                                    client.setRemoteUrl(root.remoteEditSourceName, root.remoteDraftUrl.trim())
                                else
                                    client.addRemote(root.remoteDraftName.trim(), root.remoteDraftUrl.trim())
                                root.clearRemoteDraft()
                            }
                        }
                    }
                }
            }

            PierSearchField {
                Layout.fillWidth: true
                text: root.remoteSearchText
                placeholder: qsTr("Filter remotes")
                clearable: true
                compact: true
                onTextChanged: root.remoteSearchText = text
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 220
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.filteredRemoteEntries().length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.filteredRemoteEntries()

                                delegate: ManagerInfoRow {
                                    width: parent.width
                                    leadingIcon: "network"
                                    accentColor: Theme.accent
                                    title: modelData.name || ""
                                    subtitle: modelData.fetch_url || ""
                                    metaText: modelData.push_url || ""
                                    alwaysShowActions: false
                                    primaryActionText: qsTr("Fetch")
                                    secondaryActionText: qsTr("Edit")
                                    tertiaryActionText: qsTr("Remove")
                                    onPrimaryAction: client.fetchRemote(modelData.name || "")
                                    onSecondaryAction: root.beginRemoteEdit(modelData)
                                    onTertiaryAction: client.removeRemote(modelData.name || "")
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.filteredRemoteEntries().length === 0
                        icon: "network"
                        title: qsTr("No remotes")
                        description: qsTr("Add an origin or upstream remote to enable pull and push.")
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: configManagerPopover
        width: 392
        onOpened: client.loadConfig()
        onClosed: {
            root.configSearchText = ""
            root.configComposerExpanded = false
            root.configSelectedGlobal = false
            root.configDraftKey = ""
            root.configDraftValue = ""
            root.configDraftGlobal = false
        }

        ColumnLayout {
            width: configManagerPopover.width - configManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Config")
                subtitle: qsTr("View and edit local or global Git configuration")

                IconButton {
                    compact: true
                    icon: root.configComposerExpanded ? "x" : "plus"
                    tooltip: root.configComposerExpanded ? qsTr("Hide composer") : qsTr("Add setting")
                    onClicked: {
                        root.configComposerExpanded = !root.configComposerExpanded
                        if (!root.configComposerExpanded) {
                            root.configDraftKey = ""
                            root.configDraftValue = ""
                            root.configDraftGlobal = false
                        }
                    }
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload config")
                    onClicked: client.loadConfig()
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                visible: root.configComposerExpanded
                inset: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1_5

                    ToolBanner {
                        Layout.fillWidth: true
                        visible: root.configDraftKey.trim().length > 0
                        tone: "info"
                        text: qsTr("Editing %1").arg(root.configDraftKey.trim())
                    }

                    SegmentedControl {
                        Layout.fillWidth: true
                        options: [qsTr("Local"), qsTr("Global")]
                        currentIndex: root.configDraftGlobal ? 1 : 0
                        onActivated: (index) => root.configDraftGlobal = index === 1
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.configDraftKey
                        placeholder: qsTr("Config key")
                        onTextChanged: root.configDraftKey = text
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        text: root.configDraftValue
                        placeholder: qsTr("Config value")
                        onTextChanged: root.configDraftValue = text
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Item { Layout.fillWidth: true }

                        PrimaryButton {
                            compact: true
                            text: qsTr("Set value")
                            enabled: root.configDraftKey.trim().length > 0 && !client.busy
                            onClicked: client.setConfigValue(root.configDraftKey.trim(),
                                                             root.configDraftValue,
                                                             root.configDraftGlobal)
                        }
                    }
                }
            }

            PierSearchField {
                Layout.fillWidth: true
                text: root.configSearchText
                placeholder: qsTr("Filter key or value")
                compact: true
                clearable: true
                onTextChanged: root.configSearchText = text
            }

            SegmentedControl {
                Layout.fillWidth: true
                options: [qsTr("Local"), qsTr("Global")]
                currentIndex: root.configSelectedGlobal ? 1 : 0
                onActivated: (index) => root.configSelectedGlobal = index === 1
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 236
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.configEntriesForScope(root.configSelectedGlobal).length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.configEntriesForScope(root.configSelectedGlobal)

                                delegate: ManagerInfoRow {
                                    width: parent.width
                                    leadingIcon: "settings-2"
                                    accentColor: Theme.textSecondary
                                    title: modelData.key || ""
                                    subtitle: modelData.value || ""
                                    metaText: modelData.scope || ""
                                    primaryActionText: qsTr("Edit")
                                    secondaryActionText: qsTr("Copy")
                                    tertiaryActionText: qsTr("Unset")
                                    onPrimaryAction: root.beginConfigEdit(modelData)
                                    onSecondaryAction: root.copyText(modelData.value || "")
                                    onTertiaryAction: client.unsetConfigValue(modelData.key || "", root.configSelectedGlobal)
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.configEntriesForScope(root.configSelectedGlobal).length === 0
                        icon: "settings-2"
                        title: qsTr("No config entries")
                        description: root.configSelectedGlobal
                                     ? qsTr("Set global Git configuration values that apply across repositories.")
                                     : qsTr("Set repository-specific Git configuration values for this project.")
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: rebaseManagerPopover
        width: 432
        onOpened: client.loadRebasePlan(root.rebaseCommitCount)

        ColumnLayout {
            width: rebaseManagerPopover.width - rebaseManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Interactive rebase")
                subtitle: client.rebaseInProgress
                          ? qsTr("Continue or abort the active rebase session")
                          : qsTr("Reorder, squash, or drop recent commits")

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload rebase plan")
                    onClicked: client.loadRebasePlan(root.rebaseCommitCount)
                }
            }

            ToolBanner {
                Layout.fillWidth: true
                visible: client.rebaseInProgress
                tone: "warning"
                text: qsTr("Git reports that an interactive rebase is already in progress.")
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1_5
                visible: client.rebaseInProgress

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Abort")
                    onClicked: client.abortRebase()
                }

                PrimaryButton {
                    compact: true
                    text: qsTr("Continue")
                    onClicked: client.continueRebase()
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                inset: true
                visible: !client.rebaseInProgress

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1_5

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1_5

                        PierComboBox {
                            Layout.preferredWidth: 132
                            options: ["10", "20", "50"]
                            currentIndex: root.rebaseCommitCount === 20 ? 1 : root.rebaseCommitCount === 50 ? 2 : 0
                            onActivated: (index) => {
                                root.rebaseCommitCount = Number(options[index] || "10")
                                client.loadRebasePlan(root.rebaseCommitCount)
                            }
                        }

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Recent commits")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                        }

                        PrimaryButton {
                            compact: true
                            text: qsTr("Execute")
                            enabled: root.rebaseDraftItems.length > 0 && !client.busy
                            onClicked: {
                                const base = root.rebaseDraftItems.length > 0
                                        ? String(root.rebaseDraftItems[root.rebaseDraftItems.length - 1].hash || "") + "~1"
                                        : ""
                                client.executeRebase(root.rebaseDraftItems, base)
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 280
                        radius: Theme.radiusMd
                        color: Theme.bgInset
                        border.color: Theme.borderSubtle
                        border.width: 1
                        clip: true

                        Item {
                            anchors.fill: parent

                            PierScrollView {
                                anchors.fill: parent
                                visible: root.rebaseDraftItems.length > 0

                                Column {
                                    width: parent.width

                                    Repeater {
                                        model: root.rebaseDraftItems

                                        delegate: Rectangle {
                                            required property var modelData
                                            required property int index
                                            readonly property var actionValues: ["pick", "reword", "edit", "squash", "fixup", "drop"]

                                            width: parent.width
                                            implicitHeight: 38
                                            color: "transparent"

                                            RowLayout {
                                                anchors.fill: parent
                                                anchors.leftMargin: Theme.sp2
                                                anchors.rightMargin: Theme.sp2
                                                spacing: Theme.sp1_5

                                                PierComboBox {
                                                    Layout.preferredWidth: 112
                                                    options: [qsTr("Pick"), qsTr("Reword"), qsTr("Edit"), qsTr("Squash"), qsTr("Fixup"), qsTr("Drop")]
                                                    currentIndex: Math.max(0, actionValues.indexOf(String(modelData.action || "pick")))
                                                    onActivated: (comboIndex) => {
                                                        const copy = root.rebaseDraftItems.slice()
                                                        const row = Object.assign({}, copy[index])
                                                        row.action = actionValues[comboIndex] || "pick"
                                                        copy[index] = row
                                                        root.rebaseDraftItems = copy
                                                    }
                                                }

                                                Text {
                                                    text: modelData.shortHash || ""
                                                    font.family: Theme.fontMono
                                                    font.pixelSize: Theme.sizeCaption
                                                    color: Theme.accent
                                                }

                                                Text {
                                                    Layout.fillWidth: true
                                                    text: modelData.message || ""
                                                    font.family: Theme.fontUi
                                                    font.pixelSize: Theme.sizeBody
                                                    color: Theme.textPrimary
                                                    elide: Text.ElideRight
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: "↑"
                                                    enabled: index > 0
                                                    onClicked: root.moveRebaseItem(index, index - 1)
                                                }

                                                GhostButton {
                                                    compact: true
                                                    minimumWidth: 0
                                                    text: "↓"
                                                    enabled: index < root.rebaseDraftItems.length - 1
                                                    onClicked: root.moveRebaseItem(index, index + 1)
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
                                }
                            }

                            ToolEmptyState {
                                anchors.centerIn: parent
                                visible: root.rebaseDraftItems.length === 0
                                icon: "git-merge"
                                title: qsTr("No rebase plan")
                                description: qsTr("Load recent commits to start an interactive rebase.")
                            }
                        }
                    }
                }
            }
        }
    }

    PopoverPanel {
        id: submoduleManagerPopover
        width: 392
        onOpened: client.loadSubmodules()
        onClosed: root.submoduleSearchText = ""

        ColumnLayout {
            width: submoduleManagerPopover.width - submoduleManagerPopover.padding * 2
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Submodules")
                subtitle: qsTr("Inspect and update nested repositories")

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Reload submodules")
                    onClicked: client.loadSubmodules()
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1_5

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Init")
                    onClicked: client.initSubmodules()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Update")
                    onClicked: client.updateSubmodules(true)
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Sync")
                    onClicked: client.syncSubmodules()
                }
            }

            PierSearchField {
                Layout.fillWidth: true
                text: root.submoduleSearchText
                placeholder: qsTr("Filter submodules")
                clearable: true
                compact: true
                onTextChanged: root.submoduleSearchText = text
            }

            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 268
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    PierScrollView {
                        anchors.fill: parent
                        visible: root.filteredSubmodules().length > 0

                        Column {
                            width: parent.width

                            Repeater {
                                model: root.filteredSubmodules()

                                delegate: Rectangle {
                                    required property var modelData
                                    width: parent.width
                                    implicitHeight: 54
                                    color: submoduleMouse.containsMouse ? Theme.bgHover : "transparent"

                                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp2
                                        anchors.rightMargin: Theme.sp2
                                        spacing: Theme.sp2

                                        Rectangle {
                                            width: 10
                                            height: 10
                                            radius: 5
                                            color: modelData.status === "ok"
                                                   ? Theme.statusSuccess
                                                   : modelData.status === "modified"
                                                     ? Theme.statusWarning
                                                     : Theme.statusError
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 0

                                            Text {
                                                Layout.fillWidth: true
                                                text: modelData.path || ""
                                                font.family: Theme.fontMono
                                                font.pixelSize: Theme.sizeBody
                                                color: Theme.textPrimary
                                                elide: Text.ElideRight
                                            }

                                            Text {
                                                Layout.fillWidth: true
                                                visible: String(modelData.url || "").length > 0
                                                text: modelData.url || ""
                                                font.family: Theme.fontUi
                                                font.pixelSize: Theme.sizeSmall
                                                color: Theme.textTertiary
                                                elide: Text.ElideMiddle
                                            }
                                        }

                                        Text {
                                            text: modelData.shortHash || ""
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeCaption
                                            color: Theme.textSecondary
                                        }

                                        Row {
                                            visible: submoduleMouse.containsMouse
                                            spacing: Theme.sp1

                                            GhostButton {
                                                compact: true
                                                minimumWidth: 0
                                                text: qsTr("Copy path")
                                                onClicked: root.copyText(modelData.path || "")
                                            }

                                            GhostButton {
                                                visible: String(modelData.url || "").length > 0
                                                compact: true
                                                minimumWidth: 0
                                                text: qsTr("Copy URL")
                                                onClicked: root.copyText(modelData.url || "")
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

                                    MouseArea {
                                        id: submoduleMouse
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        acceptedButtons: Qt.NoButton
                                    }
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: root.filteredSubmodules().length === 0
                        icon: "layers"
                        title: qsTr("No submodules")
                        description: qsTr("Nested repositories will appear here after you add or initialize them.")
                    }
                }
            }
        }
    }

    ModalDialogShell {
        id: blameDialog
        open: root.blameDialogOpen
        title: qsTr("Blame")
        subtitle: client.blameFilePath.length > 0 ? client.blameFilePath : qsTr("Line ownership")
        dialogWidth: 900
        dialogHeight: 620
        bodyPadding: Theme.sp4
        onRequestClose: root.blameDialogOpen = false

        body: Rectangle {
            anchors.fill: parent
            color: "transparent"

            Rectangle {
                anchors.fill: parent
                radius: Theme.radiusMd
                color: Theme.bgSurface
                border.color: Theme.borderSubtle
                border.width: 1
                clip: true

                Item {
                    anchors.fill: parent

                    ListView {
                        anchors.fill: parent
                        clip: true
                        model: client.blameLines
                        visible: client.blameLines.length > 0

                        delegate: Rectangle {
                            required property var modelData
                            width: ListView.view.width
                            implicitHeight: 28
                            color: index % 2 === 0 ? "transparent" : Theme.bgHover

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                spacing: Theme.sp2

                                Text {
                                    text: String(modelData.line_number || modelData.lineNumber || "")
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeCaption
                                    color: Theme.textTertiary
                                    Layout.preferredWidth: 42
                                    horizontalAlignment: Text.AlignRight
                                }

                                Text {
                                    text: modelData.short_hash || modelData.shortHash || ""
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeCaption
                                    color: Theme.accent
                                    Layout.preferredWidth: 64
                                }

                                Text {
                                    text: modelData.author || ""
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textSecondary
                                    Layout.preferredWidth: 120
                                    elide: Text.ElideRight
                                }

                                Text {
                                    text: modelData.date || ""
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeCaption
                                    color: Theme.textTertiary
                                    Layout.preferredWidth: 108
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData.content || ""
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textPrimary
                                    elide: Text.ElideRight
                                }
                            }
                        }
                    }

                    ToolEmptyState {
                        anchors.centerIn: parent
                        visible: client.blameLines.length === 0 && !client.busy
                        icon: "file-text"
                        title: qsTr("No blame data")
                        description: qsTr("Select a file diff and run blame to inspect line ownership.")
                    }
                }
            }
        }
    }

    component RefBadge: Rectangle {
        property string token: ""

        readonly property string trimmed: String(token || "").trim()
        readonly property color tone: {
            if (trimmed.indexOf("HEAD") >= 0)
                return Theme.accent
            if (trimmed.indexOf("tag:") === 0)
                return "#f59e0b"
            if (trimmed.indexOf("/") >= 0)
                return Theme.statusSuccess
            return Theme.textSecondary
        }

        visible: trimmed.length > 0
        implicitHeight: 18
        implicitWidth: label.implicitWidth + Theme.sp1_5 * 2
        radius: Theme.radiusPill
        color: Qt.rgba(tone.r, tone.g, tone.b, Theme.dark ? 0.14 : 0.10)
        border.color: Qt.rgba(tone.r, tone.g, tone.b, Theme.dark ? 0.26 : 0.16)
        border.width: 1

        Text {
            id: label
            anchors.centerIn: parent
            text: parent.trimmed
            font.family: Theme.fontMono
            font.pixelSize: 9
            font.weight: Theme.weightMedium
            color: parent.tone
        }
    }

    component GraphLaneCanvas: Canvas {
        property var rowData: ({})

        implicitWidth: 78
        implicitHeight: 44
        antialiasing: true

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            ctx.clearRect(0, 0, width, height)

            const offsetX = 10
            const offsetY = Math.max(0, Math.round((height - 24) / 2))
            const segments = rowData.segments || []
            const arrows = rowData.arrows || []

            for (let i = 0; i < segments.length; ++i) {
                const seg = segments[i]
                ctx.beginPath()
                ctx.moveTo(offsetX + Number(seg.x_top || 0), offsetY + Number(seg.y_top || 0))
                ctx.lineTo(offsetX + Number(seg.x_bottom || 0), offsetY + Number(seg.y_bottom || 0))
                ctx.strokeStyle = root.graphColor(seg.color_index || 0)
                ctx.lineWidth = 1.6
                ctx.lineCap = "round"
                ctx.stroke()
            }

            for (let j = 0; j < arrows.length; ++j) {
                const arrow = arrows[j]
                const color = root.graphColor(arrow.color_index || 0)
                const x = offsetX + Number(arrow.x || 0)
                const y = offsetY + Number(arrow.y || 0)
                ctx.beginPath()
                if (arrow.is_down) {
                    ctx.moveTo(x - 3, y - 2)
                    ctx.lineTo(x + 3, y - 2)
                    ctx.lineTo(x, y + 3)
                } else {
                    ctx.moveTo(x - 3, y + 2)
                    ctx.lineTo(x + 3, y + 2)
                    ctx.lineTo(x, y - 3)
                }
                ctx.closePath()
                ctx.fillStyle = color
                ctx.fill()
            }

            const nodeX = offsetX + Number(rowData.node_column || 0) * 12 + 6
            const nodeY = offsetY + 12
            const nodeColor = root.graphColor(rowData.color_index || 0)
            ctx.beginPath()
            ctx.arc(nodeX, nodeY, 4.2, 0, Math.PI * 2, false)
            ctx.fillStyle = nodeColor
            ctx.fill()
            ctx.lineWidth = 1
            ctx.strokeStyle = Theme.bgSurface
            ctx.stroke()
        }

        Component.onCompleted: requestPaint()
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        onRowDataChanged: requestPaint()
    }

    component HistoryGraphRow: Rectangle {
        property var rowData: ({})
        property int rowIndex: 0
        property bool selected: false
        signal clicked()

        readonly property var refList: root.refTokens(rowData.refs || "")
        readonly property bool hasRefs: refList.length > 0
        readonly property bool dimmed: root.historyRowShouldDim(rowData)
        readonly property color zebraColor: rowIndex % 2 === 1 ? Theme.bgHover : "transparent"

        width: parent ? parent.width : 0
        height: hasRefs ? 56 : 46
        color: selected ? Theme.bgSelected
                        : rowMouse.containsMouse ? Theme.bgHover
                        : (root.historyShowZebraStripes ? zebraColor : "transparent")
        opacity: dimmed ? 0.46 : 1.0

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp3
            spacing: Theme.sp2

            GraphLaneCanvas {
                Layout.preferredWidth: 78
                Layout.fillHeight: true
                rowData: parent.parent.rowData
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: hasRefs ? Theme.sp0_5 : 0

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Text {
                        visible: root.historyShowHash
                        text: rowData.short_hash || rowData.shortHash || ""
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        font.weight: Theme.weightMedium
                        color: Theme.accent
                    }

                    Text {
                        Layout.fillWidth: true
                        text: rowData.message || ""
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textPrimary
                        elide: Text.ElideRight
                    }

                    Text {
                        visible: root.historyShowDate
                        text: root.formatGraphDate(rowData.date_timestamp || 0)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textTertiary
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp1_5

                    Text {
                        visible: root.historyShowAuthor
                        text: rowData.author || ""
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textSecondary
                    }

                    Flow {
                        Layout.fillWidth: true
                        width: parent.width
                        visible: hasRefs
                        spacing: Theme.sp1

                        Repeater {
                            model: refList.slice(0, 3)

                            delegate: RefBadge {
                                token: modelData
                            }
                        }
                    }

                    Text {
                        visible: refList.length > 3
                        text: qsTr("+%1").arg(refList.length - 3)
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textTertiary
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

        MouseArea {
            id: rowMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            acceptedButtons: Qt.LeftButton | Qt.RightButton
            onClicked: (mouse) => {
                if (mouse.button === Qt.RightButton) {
                    root.selectHistoryCommit(rowData, false)
                    const pos = mapToItem(root, mouse.x, mouse.y)
                    historyCommitPopover.width = 232
                    historyCommitPopover.x = Math.max(Theme.sp2,
                                                      Math.min(root.width - historyCommitPopover.width - Theme.sp2, pos.x))
                    historyCommitPopover.y = Math.max(Theme.sp2, pos.y)
                    historyCommitPopover.open()
                    return
                }
                parent.clicked()
            }
        }
    }

    component ManagerInfoRow: Rectangle {
        id: infoRowRoot
        property string leadingIcon: "circle"
        property color accentColor: Theme.accent
        property string title: ""
        property string subtitle: ""
        property string metaText: ""
        property bool alwaysShowActions: false
        property string primaryActionText: ""
        property string secondaryActionText: ""
        property string tertiaryActionText: ""
        signal primaryAction()
        signal secondaryAction()
        signal tertiaryAction()

        width: parent ? parent.width : 0
        implicitHeight: 50
        color: infoMouse.containsMouse ? Theme.bgHover : "transparent"

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp2
            spacing: Theme.sp2

            Rectangle {
                width: 18
                height: 18
                radius: Theme.radiusSm
                color: Qt.rgba(accentColor.r, accentColor.g, accentColor.b, Theme.dark ? 0.14 : 0.10)

                Image {
                    anchors.centerIn: parent
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + leadingIcon + ".svg"
                    sourceSize: Qt.size(12, 12)
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Text {
                    Layout.fillWidth: true
                    text: infoRowRoot.title
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                Text {
                    Layout.fillWidth: true
                    text: infoRowRoot.subtitle
                    visible: text.length > 0
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }
            }

            Text {
                text: infoRowRoot.metaText
                visible: text.length > 0
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary
                elide: Text.ElideMiddle
            }

            Row {
                visible: infoRowRoot.alwaysShowActions || infoMouse.containsMouse
                spacing: Theme.sp1

                GhostButton {
                    visible: infoRowRoot.primaryActionText.length > 0
                    compact: true
                    minimumWidth: 0
                    text: infoRowRoot.primaryActionText
                    onClicked: infoRowRoot.primaryAction()
                }

                GhostButton {
                    visible: infoRowRoot.secondaryActionText.length > 0
                    compact: true
                    minimumWidth: 0
                    text: infoRowRoot.secondaryActionText
                    onClicked: infoRowRoot.secondaryAction()
                }

                GhostButton {
                    visible: infoRowRoot.tertiaryActionText.length > 0
                    compact: true
                    minimumWidth: 0
                    text: infoRowRoot.tertiaryActionText
                    onClicked: infoRowRoot.tertiaryAction()
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

        MouseArea {
            id: infoMouse
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
        }
    }

    component BranchManagerRow: Rectangle {
        id: branchRowRoot
        property string branchName: ""
        property string trackingName: ""
        property bool current: false
        property bool remote: false
        property bool renameMode: false
        property string renameText: ""
        signal checkoutClicked()
        signal mergeClicked()
        signal renameRequested()
        signal renameSubmitted()
        signal renameCancelled()
        signal deleteClicked()

        width: parent ? parent.width : 0
        implicitHeight: 50
        color: branchMouse.containsMouse ? Theme.bgHover : "transparent"

        Behavior on color { ColorAnimation { duration: Theme.durFast } }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp2
            spacing: Theme.sp2

            Rectangle {
                width: 10
                height: 10
                radius: 5
                color: current ? Theme.statusSuccess : remote ? Theme.accent : Theme.textTertiary
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Loader {
                    Layout.fillWidth: true
                    sourceComponent: renameMode ? renameEditor : branchTitle
                }

                Text {
                    visible: trackingName.length > 0 && !renameMode
                    Layout.fillWidth: true
                    text: qsTr("Tracking %1").arg(trackingName)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }
            }

            StatusPill {
                visible: current
                text: qsTr("Current")
                tone: "success"
            }

            Row {
                visible: !renameMode && branchMouse.containsMouse
                spacing: Theme.sp1

                GhostButton {
                    visible: !current
                    compact: true
                    minimumWidth: 0
                    text: remote ? qsTr("Checkout") : qsTr("Switch")
                    onClicked: checkoutClicked()
                }

                GhostButton {
                    visible: !remote && !current
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Merge")
                    onClicked: mergeClicked()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Rename")
                    onClicked: renameRequested()
                }

                GhostButton {
                    visible: !current
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Delete")
                    onClicked: deleteClicked()
                }
            }
        }

        Component {
            id: branchTitle

            Text {
                text: branchName
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: current ? Theme.weightSemibold : Theme.weightRegular
                color: Theme.textPrimary
                elide: Text.ElideMiddle
            }
        }

        Component {
            id: renameEditor

            RowLayout {
                spacing: Theme.sp1_5

                PierTextField {
                    Layout.fillWidth: true
                    text: renameText
                    placeholder: qsTr("Rename branch")
                    onTextChanged: branchRowRoot.renameText = text
                    onEditingFinished: renameSubmitted()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Cancel")
                    onClicked: renameCancelled()
                }

                PrimaryButton {
                    compact: true
                    text: qsTr("Save")
                    enabled: renameText.trim().length > 0
                    onClicked: renameSubmitted()
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

        MouseArea {
            id: branchMouse
            anchors.fill: parent
            hoverEnabled: true
            acceptedButtons: Qt.NoButton
        }
    }

    component ConflictFileRow: Rectangle {
        property var fileData: ({})
        property bool selected: false
        signal openRequested()
        signal stageRequested()

        width: parent ? parent.width : 0
        implicitHeight: 52
        color: selected ? Theme.bgSelected : conflictFileMouse.containsMouse ? Theme.bgHover : "transparent"

        Behavior on color { ColorAnimation { duration: Theme.durFast } }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp3
            anchors.rightMargin: Theme.sp3
            spacing: Theme.sp2

            Rectangle {
                width: 10
                height: 10
                radius: 5
                color: Theme.statusWarning
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Text {
                    Layout.fillWidth: true
                    text: fileData.name || fileData.path || ""
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                Text {
                    Layout.fillWidth: true
                    text: fileData.path || ""
                    visible: text.length > 0
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }
            }

            StatusPill {
                text: qsTr("%1").arg(Number(fileData.conflictCount || 0))
                tone: "warning"
            }

            GhostButton {
                compact: true
                minimumWidth: 0
                text: qsTr("Stage")
                onClicked: stageRequested()
            }
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.borderSubtle
        }

        MouseArea {
            id: conflictFileMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: openRequested()
        }
    }

    component ConflictHunkCard: Rectangle {
        property string filePath: ""
        property int hunkIndex: -1
        property var hunkData: ({})
        readonly property string resolution: String(hunkData.resolution || "")

        width: parent ? parent.width : 0
        implicitHeight: bodyLayout.implicitHeight + Theme.sp3 * 2
        radius: Theme.radiusMd
        color: Theme.bgSurface
        border.color: Theme.borderSubtle
        border.width: 1

        ColumnLayout {
            id: bodyLayout
            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                compact: true
                title: qsTr("Conflict %1").arg(hunkIndex + 1)
                subtitle: resolution.length > 0
                          ? qsTr("Selected: %1").arg(resolution)
                          : qsTr("Choose a resolution for this hunk")

                StatusPill {
                    visible: resolution.length > 0
                    text: resolution === "ours"
                          ? qsTr("Ours")
                          : resolution === "theirs"
                            ? qsTr("Theirs")
                            : qsTr("Both")
                    tone: resolution === "theirs" ? "info" : resolution === "both" ? "warning" : "success"
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: oursColumn.implicitHeight + Theme.sp2 * 2
                    radius: Theme.radiusSm
                    color: Qt.rgba(Theme.statusSuccess.r, Theme.statusSuccess.g, Theme.statusSuccess.b, Theme.dark ? 0.08 : 0.06)
                    border.color: Qt.rgba(Theme.statusSuccess.r, Theme.statusSuccess.g, Theme.statusSuccess.b, Theme.dark ? 0.18 : 0.12)
                    border.width: 1

                    ColumnLayout {
                        id: oursColumn
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        Text {
                            text: qsTr("Ours")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            font.weight: Theme.weightSemibold
                            color: Theme.statusSuccess
                        }

                        Repeater {
                            model: hunkData.oursLines || []

                            delegate: Text {
                                width: parent.width
                                text: modelData
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textPrimary
                                wrapMode: Text.WrapAnywhere
                            }
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: theirsColumn.implicitHeight + Theme.sp2 * 2
                    radius: Theme.radiusSm
                    color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, Theme.dark ? 0.08 : 0.06)
                    border.color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, Theme.dark ? 0.18 : 0.12)
                    border.width: 1

                    ColumnLayout {
                        id: theirsColumn
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        Text {
                            text: qsTr("Theirs")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            font.weight: Theme.weightSemibold
                            color: Theme.accent
                        }

                        Repeater {
                            model: hunkData.theirsLines || []

                            delegate: Text {
                                width: parent.width
                                text: modelData
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textPrimary
                                wrapMode: Text.WrapAnywhere
                            }
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1_5

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Accept ours")
                    onClicked: client.resolveConflict(filePath, hunkIndex, "ours")
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Accept theirs")
                    onClicked: client.resolveConflict(filePath, hunkIndex, "theirs")
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Accept both")
                    onClicked: client.resolveConflict(filePath, hunkIndex, "both")
                }
            }
        }
    }

    component GitTabButton: Rectangle {
        property string title: ""
        property string icon: ""
        property string badge: ""
        property bool active: false
        signal clicked

        implicitHeight: 25
        implicitWidth: row.implicitWidth + Theme.sp2 * 2
        radius: Theme.radiusSm
        color: active ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, Theme.dark ? 0.14 : 0.11)
                      : tabMouse.containsMouse ? Theme.bgHover : "transparent"
        border.width: 0

        RowLayout {
            id: row
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp2
            spacing: Theme.sp1

            Item {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 12
                Layout.preferredHeight: 12

                Image {
                    anchors.centerIn: parent
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
                    sourceSize: Qt.size(12, 12)
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        colorization: 1.0
                        colorizationColor: active ? Theme.accent : Theme.textSecondary
                    }
                }
            }

            Text {
                Layout.alignment: Qt.AlignVCenter
                text: title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: active ? Theme.weightMedium : Theme.weightRegular
                color: active ? Theme.accent : Theme.textSecondary
                elide: Text.ElideRight
            }

            Rectangle {
                visible: badge.length > 0
                implicitWidth: badgeLabel.implicitWidth + Theme.sp1 * 2
                implicitHeight: 14
                radius: Theme.radiusPill
                color: active ? Theme.accentSubtle : Theme.bgSurface

                Text {
                    id: badgeLabel
                    anchors.centerIn: parent
                    text: badge
                    font.family: Theme.fontMono
                    font.pixelSize: 9
                    color: active ? Theme.accent : Theme.textTertiary
                }
            }
        }

        MouseArea {
            id: tabMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
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

                    GhostButton {
                        visible: diffPath.length > 0
                        compact: true
                        minimumWidth: 0
                        text: qsTr("Blame")
                        onClicked: {
                            client.loadBlame(diffPath)
                            root.blameDialogOpen = true
                        }
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
        case "M": return Theme.accent
        case "A": return Theme.statusSuccess
        case "D": return Theme.statusError
        case "R": return Theme.accent
        case "?": return Theme.textTertiary
        case "U": return Theme.statusWarning
        case "C": return Theme.accent
        default:  return Theme.textTertiary
        }
    }

    function configEntriesForScope(globalScope) {
        const needle = String(root.configSearchText || "").trim().toLowerCase()
        return (client.configEntries || []).filter(function(entry) {
            if (String(entry.scope || "") !== (globalScope ? "global" : "local"))
                return false
            if (!needle.length)
                return true
            const key = String(entry.key || "").toLowerCase()
            const value = String(entry.value || "").toLowerCase()
            return key.indexOf(needle) >= 0 || value.indexOf(needle) >= 0
        })
    }

    function runPrimaryCommitAction(message) {
        const trimmed = String(message || "").trim()
        if (!trimmed.length)
            return
        client.commit(trimmed)
    }

    function runCommitAndPushAction(message) {
        const trimmed = String(message || "").trim()
        if (!trimmed.length)
            return
        client.commitAndPush(trimmed)
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
                html += "<span style='color:" + Theme.accent + ";'>" + line + "</span>\n"
            else if (line.startsWith("+"))
                html += "<span style='color:" + Theme.statusSuccess + ";'>" + line + "</span>\n"
            else if (line.startsWith("-"))
                html += "<span style='color:" + Theme.statusError + ";'>" + line + "</span>\n"
            else
                html += "<span style='color:" + Theme.textPrimary + ";'>" + line + "</span>\n"
        }
        return html + "</pre>"
    }
}
