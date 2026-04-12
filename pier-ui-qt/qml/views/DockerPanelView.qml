import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier
import "../components"

Rectangle {
    id: root

    clip: true
    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    property var sharedSession: null
    property string sshHost: ""
    property int sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool sshUsesAgent: false

    readonly property var tabLabels: [
        qsTr("Containers"),
        qsTr("Images"),
        qsTr("Volumes"),
        qsTr("Networks"),
        qsTr("Compose")
    ]
    readonly property var restartLabels: [
        qsTr("No restart"),
        qsTr("Always"),
        qsTr("On failure"),
        qsTr("Unless stopped")
    ]
    readonly property var restartValues: [
        "no",
        "always",
        "on-failure",
        "unless-stopped"
    ]
    readonly property var networkDrivers: [
        "bridge",
        "overlay",
        "macvlan",
        "ipvlan"
    ]

    property int currentTab: 0
    property string feedbackText: ""
    property string feedbackTone: "neutral"
    property string pendingDeleteId: ""

    property string inspectKind: ""
    property string inspectKey: ""
    property string inspectTitle: ""
    property string inspectSubtitle: ""
    property var inspectFacts: []

    property string pullImageName: ""
    property string networkName: ""
    property int networkDriverIndex: 0
    property string composeFilePath: ""

    property bool runDialogOpen: false
    property string runImageRef: ""
    property string runContainerName: ""
    property string runPortsText: ""
    property string runEnvText: ""
    property string runVolumesText: ""
    property string runCommand: ""
    property int runRestartIndex: 0

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierDockerClient {
        id: client
        onActionFinished: (ok, message) => root._showFeedback(ok, message)
    }

    Timer {
        id: feedbackTimer
        interval: 3200
        repeat: false
        onTriggered: root.feedbackText = ""
    }

    Component.onCompleted: Qt.callLater(_dispatchConnect)

    Connections {
        target: root.sharedSession
        function onConnectedChanged() {
            if (root.sharedSession && root.sharedSession.connected)
                root._dispatchConnect()
        }
    }

    Connections {
        target: client
        function onStatusChanged() {
            if (client.status === PierDockerClient.Connected && root.currentTab !== 0)
                root._refreshCurrentTab()
        }
    }

    function _showFeedback(ok, message) {
        if (!message || message.length === 0)
            return
        root.feedbackText = message
        root.feedbackTone = ok ? "success" : "error"
        feedbackTimer.restart()
    }

    function _dispatchConnect() {
        if (client.status === PierDockerClient.Connecting
                || client.status === PierDockerClient.Connected)
            return

        if (root.sharedSession && root.sharedSession.connected) {
            client.connectToSession(root.sharedSession)
            return
        }

        if (root.sshHost.length === 0 || root.sshUser.length === 0) {
            client.connectLocal()
            return
        }

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
        client.connectTo(root.sshHost, root.sshPort, root.sshUser, kind, secret, extra)
    }

    function _refreshCurrentTab() {
        if (client.status !== PierDockerClient.Connected)
            return
        switch (root.currentTab) {
        case 0:
            client.refresh()
            break
        case 1:
            client.refreshImages()
            break
        case 2:
            client.refreshVolumes()
            break
        case 3:
            client.refreshNetworks()
            break
        case 4:
            client.refreshCompose(root.composeFilePath)
            break
        }
    }

    function _shellQuote(value) {
        var raw = String(value || "")
        if (raw.length === 0)
            return "''"
        return "'" + raw.replace(/'/g, "'\\''") + "'"
    }

    function _clearInspect() {
        root.inspectKind = ""
        root.inspectKey = ""
        root.inspectTitle = ""
        root.inspectSubtitle = ""
        root.inspectFacts = []
        client.clearInspect()
    }

    function _refreshInspect() {
        switch (root.inspectKind) {
        case "container":
            client.inspect(root.inspectKey)
            break
        case "image":
            client.inspectImage(root.inspectKey)
            break
        case "volume":
            client.inspectVolume(root.inspectKey)
            break
        case "network":
            client.inspectNetwork(root.inspectKey)
            break
        }
    }

    function _openInspect(kind, key, title, subtitle, facts) {
        root.inspectKind = kind
        root.inspectKey = key
        root.inspectTitle = title || key
        root.inspectSubtitle = subtitle || ""
        root.inspectFacts = facts || []
        root._refreshInspect()
    }

    function _openLogsFor(id, name) {
        if (typeof window.openLogTab !== "function") {
            root._showFeedback(false, qsTr("Log tab integration is unavailable in this build."))
            return
        }
        var conn = {
            name: name || id,
            host: root.sshHost,
            port: root.sshPort,
            username: root.sshUser,
            password: root.sshPassword,
            credentialId: root.sshCredentialId,
            keyPath: root.sshKeyPath,
            passphraseCredentialId: root.sshPassphraseCredentialId,
            usesAgent: root.sshUsesAgent
        }
        var cmd = "docker logs -f --tail 500 " + id
        window.openLogTab(conn, cmd, name || id)
    }

    function _openComposeLogsFor(service) {
        if (root.composeFilePath.trim().length === 0) {
            root._showFeedback(false, qsTr("Set a compose file path first."))
            return
        }
        if (typeof window.openLogTab !== "function") {
            root._showFeedback(false, qsTr("Log tab integration is unavailable in this build."))
            return
        }
        var conn = {
            name: service,
            host: root.sshHost,
            port: root.sshPort,
            username: root.sshUser,
            password: root.sshPassword,
            credentialId: root.sshCredentialId,
            keyPath: root.sshKeyPath,
            passphraseCredentialId: root.sshPassphraseCredentialId,
            usesAgent: root.sshUsesAgent
        }
        var cmd = "docker compose -f " + root._shellQuote(root.composeFilePath.trim()) + " logs -f --tail 500 " + service
        window.openLogTab(conn, cmd, service)
    }

    function _openRunDialog(imageRef) {
        root.runImageRef = imageRef
        root.runContainerName = ""
        root.runPortsText = ""
        root.runEnvText = ""
        root.runVolumesText = ""
        root.runCommand = ""
        root.runRestartIndex = 0
        root.runDialogOpen = true
    }

    function _lines(text) {
        return String(text || "")
            .split(/\r?\n/)
            .map(function(line) { return line.trim() })
            .filter(function(line) { return line.length > 0 })
    }

    function _parsePortMappings(text) {
        return root._lines(text).map(function(line) {
            var parts = line.split(":")
            return {
                host: parts.length > 0 ? parts[0].trim() : "",
                container: parts.length > 1 ? parts.slice(1).join(":").trim() : ""
            }
        }).filter(function(entry) {
            return entry.host.length > 0 && entry.container.length > 0
        })
    }

    function _parseEnvMappings(text) {
        return root._lines(text).map(function(line) {
            var idx = line.indexOf("=")
            if (idx < 0)
                return { key: line.trim(), value: "" }
            return {
                key: line.slice(0, idx).trim(),
                value: line.slice(idx + 1)
            }
        }).filter(function(entry) {
            return entry.key.length > 0
        })
    }

    function _parseVolumeMappings(text) {
        return root._lines(text).map(function(line) {
            var idx = line.indexOf(":")
            if (idx < 0)
                return { host: "", container: "" }
            return {
                host: line.slice(0, idx).trim(),
                container: line.slice(idx + 1).trim()
            }
        }).filter(function(entry) {
            return entry.host.length > 0 && entry.container.length > 0
        })
    }

    function _submitRunDialog() {
        client.runImage(
            root.runImageRef,
            root.runContainerName,
            root._parsePortMappings(root.runPortsText),
            root._parseEnvMappings(root.runEnvText),
            root._parseVolumeMappings(root.runVolumesText),
            root.restartValues[root.runRestartIndex],
            root.runCommand,
            true
        )
        root.runDialogOpen = false
    }

    function _currentCount() {
        switch (root.currentTab) {
        case 0:
            return client.containerCount
        case 1:
            return client.images.length
        case 2:
            return client.volumes.length
        case 3:
            return client.networks.length
        case 4:
            return client.composeServices.length
        default:
            return 0
        }
    }

    function _footerText() {
        var count = root._currentCount()
        switch (root.currentTab) {
        case 0:
            return count + " " + (count === 1 ? qsTr("container") : qsTr("containers"))
        case 1:
            return count + " " + (count === 1 ? qsTr("image") : qsTr("images"))
        case 2:
            return count + " " + (count === 1 ? qsTr("volume") : qsTr("volumes"))
        case 3:
            return count + " " + (count === 1 ? qsTr("network") : qsTr("networks"))
        case 4:
            return count + " " + (count === 1 ? qsTr("compose service") : qsTr("compose services"))
        default:
            return ""
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        ToolHeroPanel {
            Layout.fillWidth: true
            accentColor: Theme.accent

            ColumnLayout {
                id: topBar
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    prominent: true
                    title: client.target.length > 0 ? client.target : "localhost"
                    subtitle: root.currentTab === 0 ? qsTr("Containers")
                              : (root.currentTab === 1 ? qsTr("Images")
                                 : (root.currentTab === 2 ? qsTr("Volumes")
                                    : (root.currentTab === 3 ? qsTr("Networks")
                                       : qsTr("Compose"))))

                    GhostButton {
                        compact: true
                        minimumWidth: 0
                        text: qsTr("Refresh")
                        enabled: client.status === PierDockerClient.Connected
                        onClicked: root._refreshCurrentTab()
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    StatusPill {
                        text: root.sshHost.length > 0 || (root.sharedSession && root.sharedSession.connected)
                              ? qsTr("Remote")
                              : qsTr("Local")
                        tone: "info"
                    }

                    StatusPill {
                        text: client.busy ? qsTr("Busy") : qsTr("Ready")
                        tone: client.busy ? "warning" : "success"
                    }

                    StatusPill {
                        text: qsTr("%1 items").arg(root._currentCount())
                        tone: "neutral"
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    ToolFactChip {
                        label: qsTr("Target")
                        value: client.target
                        monoValue: true
                    }

                    ToolFactChip {
                        label: qsTr("Mode")
                        value: root.sshHost.length > 0 || (root.sharedSession && root.sharedSession.connected)
                               ? qsTr("SSH")
                               : qsTr("Local")
                    }

                    ToolFactChip {
                        label: qsTr("View")
                        value: root.tabLabels[root.currentTab]
                    }
                }

                SegmentedControl {
                    Layout.fillWidth: true
                    options: root.tabLabels
                    currentIndex: root.currentTab
                    onActivated: (index) => {
                        root.currentTab = index
                        root.pendingDeleteId = ""
                        root._refreshCurrentTab()
                    }
                }
            }
        }

        ToolBanner {
            Layout.fillWidth: true
            text: root.feedbackText
            tone: root.feedbackTone
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp2

            ToolPanelSurface {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumWidth: 460
                padding: Theme.sp2

                Loader {
                    anchors.fill: parent
                    sourceComponent: root.currentTab === 0 ? containersPanel
                                      : (root.currentTab === 1 ? imagesPanel
                                         : (root.currentTab === 2 ? volumesPanel
                                            : (root.currentTab === 3 ? networksPanel : composePanel)))
                }
            }

            ToolPanelSurface {
                Layout.preferredWidth: 400
                Layout.minimumWidth: 360
                Layout.fillHeight: true
                visible: root.inspectKind.length > 0
                padding: Theme.sp3

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp2

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        title: qsTr("Inspect")
                        subtitle: root.inspectTitle

                        StatusPill {
                            text: root.inspectKind.length > 0
                                  ? root.inspectKind.charAt(0).toUpperCase() + root.inspectKind.slice(1)
                                  : ""
                            tone: "neutral"
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Refresh")
                            enabled: !client.inspectBusy
                            onClicked: root._refreshInspect()
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Close")
                            onClicked: root._clearInspect()
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        implicitHeight: factsColumn.implicitHeight + Theme.sp3 * 2

                        ColumnLayout {
                            id: factsColumn
                            anchors.fill: parent
                            anchors.margins: Theme.sp3
                            spacing: Theme.sp1

                            Text {
                                visible: root.inspectSubtitle.length > 0
                                Layout.fillWidth: true
                                text: root.inspectSubtitle
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                                wrapMode: Text.Wrap
                            }

                            Repeater {
                                model: root.inspectFacts

                                delegate: RowLayout {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    SectionLabel {
                                        text: modelData.label
                                        Layout.preferredWidth: 72
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: modelData.value && String(modelData.value).length > 0
                                              ? String(modelData.value)
                                              : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeCaption
                                        color: Theme.textSecondary
                                        wrapMode: Text.WrapAnywhere
                                    }
                                }
                            }
                        }
                    }

                    ToolBanner {
                        Layout.fillWidth: true
                        tone: "error"
                        text: client.inspectError
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        inset: true
                        padding: Theme.sp2

                        Item {
                            anchors.fill: parent

                            Text {
                                anchors.centerIn: parent
                                visible: client.inspectBusy
                                text: qsTr("Loading inspect output…")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textSecondary
                            }

                            PierScrollView {
                                anchors.fill: parent
                                clip: true
                                visible: !client.inspectBusy

                                PierTextArea {
                                    readOnly: true
                                    frameVisible: false
                                    mono: true
                                    wrapMode: TextArea.NoWrap
                                    text: client.inspectJson.length > 0
                                          ? client.inspectJson
                                          : qsTr("// No inspect data yet")
                                    font.pixelSize: Theme.sizeCaption
                                    color: client.inspectJson.length > 0 ? Theme.textPrimary : Theme.textTertiary
                                    selectByMouse: true
                                }
                            }
                        }
                    }
                }
            }
        }

        ToolBanner {
            Layout.fillWidth: true
            tone: "neutral"
            text: root._footerText()
        }
    }

    Component {
        id: containersPanel

        ColumnLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Containers")
                subtitle: client.showStopped
                          ? qsTr("%1 total containers").arg(client.containerCount)
                          : qsTr("Running containers only")

                StatusPill {
                    text: client.showStopped ? qsTr("All") : qsTr("Running")
                    tone: "neutral"
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: client.showStopped ? qsTr("Hide stopped") : qsTr("Show stopped")
                    onClicked: client.showStopped = !client.showStopped
                }
            }

            ListView {
                id: containerList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: client
                spacing: Theme.sp1
                reuseItems: true

                delegate: Rectangle {
                    id: row
                    required property string containerId
                    required property string image
                    required property string names
                    required property string statusText
                    required property string state
                    required property bool isRunning
                    required property string ports
                    required property string created

                    readonly property bool confirming: root.pendingDeleteId === row.containerId

                    width: ListView.view.width
                    implicitHeight: row.confirming ? 72 : 54
                    radius: Theme.radiusSm
                    color: row.confirming
                           ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.08)
                           : (rowMouse.containsMouse ? Theme.bgHover : "transparent")

                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1
                        visible: !row.confirming

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: row.isRunning ? Theme.statusSuccess : Theme.textTertiary
                            }

                            Text {
                                Layout.preferredWidth: 170
                                text: row.names.length > 0 ? row.names : row.containerId.slice(0, 12)
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.fillWidth: true
                                text: row.image
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textSecondary
                                elide: Text.ElideRight
                            }

                            StatusPill {
                                text: row.isRunning ? qsTr("Running") : qsTr("Stopped")
                                tone: row.isRunning ? "success" : "neutral"
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: row.statusText.length > 0 ? row.statusText : row.created
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.preferredWidth: 190
                                text: row.ports.length > 0 ? row.ports : "—"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                                horizontalAlignment: Text.AlignRight
                            }

                            DockerRowButton {
                                glyph: "{}"
                                tooltip: qsTr("Inspect")
                                onClicked: root._openInspect(
                                               "container",
                                               row.containerId,
                                               row.names.length > 0 ? row.names : row.containerId.slice(0, 12),
                                               row.image,
                                               [
                                                   { "label": qsTr("ID"), "value": row.containerId },
                                                   { "label": qsTr("State"), "value": row.statusText },
                                                   { "label": qsTr("Image"), "value": row.image },
                                                   { "label": qsTr("Ports"), "value": row.ports }
                                               ])
                            }

                            DockerRowButton {
                                glyph: ">"
                                tooltip: qsTr("Start")
                                visible: !row.isRunning
                                onClicked: client.start(row.containerId)
                            }

                            DockerRowButton {
                                glyph: "[]"
                                tooltip: qsTr("Stop")
                                visible: row.isRunning
                                onClicked: client.stopContainer(row.containerId)
                            }

                            DockerRowButton {
                                glyph: "R"
                                tooltip: qsTr("Restart")
                                visible: row.isRunning
                                onClicked: client.restart(row.containerId)
                            }

                            DockerRowButton {
                                glyph: "L"
                                tooltip: qsTr("Live logs")
                                onClicked: root._openLogsFor(row.containerId, row.names)
                            }

                            DockerRowButton {
                                glyph: "X"
                                tooltip: qsTr("Remove")
                                danger: true
                                onClicked: root.pendingDeleteId = row.containerId
                            }
                        }
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp2
                        visible: row.confirming

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Remove '%1'?")
                                      .arg(row.names.length > 0 ? row.names : row.containerId.slice(0, 12))
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: Theme.statusError
                            elide: Text.ElideRight
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Cancel")
                            onClicked: root.pendingDeleteId = ""
                        }

                        PrimaryButton {
                            compact: true
                            minimumWidth: 0
                            text: row.isRunning ? qsTr("Force remove") : qsTr("Remove")
                            onClicked: {
                                client.remove(row.containerId, row.isRunning)
                                root.pendingDeleteId = ""
                            }
                        }
                    }
                }

                Text {
                    anchors.centerIn: parent
                    visible: client.busy && containerList.count === 0
                    text: qsTr("Querying docker…")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textSecondary
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: client.status === PierDockerClient.Connected
                             && !client.busy
                             && containerList.count === 0
                    icon: "container"
                    title: client.showStopped ? qsTr("No containers") : qsTr("No running containers")
                    description: qsTr("Containers for this host will appear here when Docker returns results.")
                }
            }
        }
    }

    Component {
        id: imagesPanel

        ColumnLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Images")
                subtitle: qsTr("%1 images").arg(client.images.length)
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                PierTextField {
                    Layout.fillWidth: true
                    text: root.pullImageName
                    placeholder: qsTr("Pull image, e.g. redis:7-alpine")
                    onTextChanged: root.pullImageName = text
                }

                PrimaryButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Pull")
                    enabled: root.pullImageName.trim().length > 0
                    onClicked: {
                        client.pullImage(root.pullImageName)
                        root.pullImageName = ""
                    }
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Prune")
                    onClicked: client.pruneImages()
                }
            }

            ListView {
                id: imagesList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: client.images
                spacing: Theme.sp1

                delegate: Rectangle {
                    id: imageRow
                    required property var modelData

                    readonly property string imageId: String(modelData.id || "")
                    readonly property string repository: String(modelData.repository || "<none>")
                    readonly property string tag: String(modelData.tag || "<none>")
                    readonly property string size: String(modelData.size || "")
                    readonly property string created: String(modelData.created || "")

                    width: ListView.view.width
                    implicitHeight: 58
                    radius: Theme.radiusSm
                    color: rowMouse.containsMouse ? Theme.bgHover : "transparent"

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: imageRow.repository
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            StatusPill {
                                text: imageRow.tag
                                tone: "info"
                            }

                            Text {
                                text: imageRow.size.length > 0 ? imageRow.size : "—"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: imageRow.created.length > 0 ? imageRow.created : imageRow.imageId
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                            }

                            DockerRowButton {
                                glyph: "{}"
                                tooltip: qsTr("Inspect")
                                onClicked: root._openInspect(
                                               "image",
                                               imageRow.imageId,
                                               imageRow.repository,
                                               imageRow.tag,
                                               [
                                                   { "label": qsTr("ID"), "value": imageRow.imageId },
                                                   { "label": qsTr("Tag"), "value": imageRow.tag },
                                                   { "label": qsTr("Size"), "value": imageRow.size },
                                                   { "label": qsTr("Created"), "value": imageRow.created }
                                               ])
                            }

                            DockerRowButton {
                                glyph: ">"
                                tooltip: qsTr("Run container")
                                onClicked: root._openRunDialog(
                                               imageRow.repository === "<none>"
                                               ? imageRow.imageId
                                               : imageRow.repository + ":" + imageRow.tag)
                            }

                            DockerRowButton {
                                glyph: "X"
                                tooltip: qsTr("Remove")
                                danger: true
                                onClicked: client.removeImage(imageRow.imageId, false)
                            }

                            DockerRowButton {
                                glyph: "!"
                                tooltip: qsTr("Force remove")
                                danger: true
                                onClicked: client.removeImage(imageRow.imageId, true)
                            }
                        }
                    }
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: client.status === PierDockerClient.Connected
                             && !client.busy
                             && imagesList.count === 0
                    icon: "container"
                    title: qsTr("No images")
                    description: qsTr("Pulled and built images will appear here.")
                }
            }
        }
    }

    Component {
        id: volumesPanel

        ColumnLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Volumes")
                subtitle: qsTr("%1 volumes").arg(client.volumes.length)

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Prune")
                    onClicked: client.pruneVolumes()
                }
            }

            ListView {
                id: volumesList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: client.volumes
                spacing: Theme.sp1

                delegate: Rectangle {
                    id: volumeRow
                    required property var modelData

                    readonly property string volumeName: String(modelData.name || "")
                    readonly property string driver: String(modelData.driver || "")
                    readonly property string mountpoint: String(modelData.mountpoint || "")

                    width: ListView.view.width
                    implicitHeight: 58
                    radius: Theme.radiusSm
                    color: rowMouse.containsMouse ? Theme.bgHover : "transparent"

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: volumeRow.volumeName
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            StatusPill {
                                text: volumeRow.driver.length > 0 ? volumeRow.driver : qsTr("Volume")
                                tone: "warning"
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: volumeRow.mountpoint.length > 0 ? volumeRow.mountpoint : qsTr("Inspect to view mountpoint")
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                elide: Text.ElideMiddle
                            }

                            DockerRowButton {
                                glyph: "{}"
                                tooltip: qsTr("Inspect")
                                onClicked: root._openInspect(
                                               "volume",
                                               volumeRow.volumeName,
                                               volumeRow.volumeName,
                                               volumeRow.driver,
                                               [
                                                   { "label": qsTr("Name"), "value": volumeRow.volumeName },
                                                   { "label": qsTr("Driver"), "value": volumeRow.driver },
                                                   { "label": qsTr("Mount"), "value": volumeRow.mountpoint }
                                               ])
                            }

                            DockerRowButton {
                                glyph: "X"
                                tooltip: qsTr("Remove")
                                danger: true
                                onClicked: client.removeVolume(volumeRow.volumeName)
                            }
                        }
                    }
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: client.status === PierDockerClient.Connected
                             && !client.busy
                             && volumesList.count === 0
                    icon: "container"
                    title: qsTr("No volumes")
                    description: qsTr("Named volumes will appear here when Docker reports them.")
                }
            }
        }
    }

    Component {
        id: networksPanel

        ColumnLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Networks")
                subtitle: qsTr("%1 networks").arg(client.networks.length)
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                PierTextField {
                    Layout.fillWidth: true
                    text: root.networkName
                    placeholder: qsTr("Create network, e.g. pier-app-net")
                    onTextChanged: root.networkName = text
                }

                PierComboBox {
                    Layout.preferredWidth: 140
                    options: root.networkDrivers
                    currentIndex: root.networkDriverIndex
                    onActivated: (index) => root.networkDriverIndex = index
                }

                PrimaryButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Create")
                    enabled: root.networkName.trim().length > 0
                    onClicked: {
                        client.createNetwork(root.networkName, root.networkDrivers[root.networkDriverIndex])
                        root.networkName = ""
                    }
                }
            }

            ListView {
                id: networksList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: client.networks
                spacing: Theme.sp1

                delegate: Rectangle {
                    id: networkRow
                    required property var modelData

                    readonly property string networkId: String(modelData.id || "")
                    readonly property string networkName: String(modelData.name || "")
                    readonly property string driver: String(modelData.driver || "")
                    readonly property string scope: String(modelData.scope || "")
                    readonly property bool removable: ["bridge", "host", "none"].indexOf(networkName) < 0

                    width: ListView.view.width
                    implicitHeight: 58
                    radius: Theme.radiusSm
                    color: rowMouse.containsMouse ? Theme.bgHover : "transparent"

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: networkRow.networkName
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            StatusPill {
                                text: networkRow.driver.length > 0 ? networkRow.driver : qsTr("Network")
                                tone: "info"
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: networkRow.scope.length > 0 ? networkRow.scope : networkRow.networkId
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                            }

                            DockerRowButton {
                                glyph: "{}"
                                tooltip: qsTr("Inspect")
                                onClicked: root._openInspect(
                                               "network",
                                               networkRow.networkName,
                                               networkRow.networkName,
                                               networkRow.networkId,
                                               [
                                                   { "label": qsTr("ID"), "value": networkRow.networkId },
                                                   { "label": qsTr("Driver"), "value": networkRow.driver },
                                                   { "label": qsTr("Scope"), "value": networkRow.scope }
                                               ])
                            }

                            DockerRowButton {
                                glyph: "X"
                                tooltip: qsTr("Remove")
                                danger: true
                                visible: networkRow.removable
                                onClicked: client.removeNetwork(networkRow.networkName)
                            }
                        }
                    }
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: client.status === PierDockerClient.Connected
                             && !client.busy
                             && networksList.count === 0
                    icon: "container"
                    title: qsTr("No networks")
                    description: qsTr("Docker networks for this host will appear here.")
                }
            }
        }
    }

    Component {
        id: composePanel

        ColumnLayout {
            anchors.fill: parent
            spacing: Theme.sp2

            ToolSectionHeader {
                Layout.fillWidth: true
                title: qsTr("Compose")
                subtitle: root.composeFilePath.trim().length > 0
                          ? root.composeFilePath
                          : qsTr("Manage a compose stack by explicit file path")
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                inset: true
                padding: Theme.sp3

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Use an absolute compose file path so local and remote hosts behave predictably.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        wrapMode: Text.Wrap
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        PierTextField {
                            Layout.fillWidth: true
                            text: root.composeFilePath
                            placeholder: qsTr("/path/to/docker-compose.yml")
                            onTextChanged: root.composeFilePath = text
                        }

                        PrimaryButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Load")
                            enabled: root.composeFilePath.trim().length > 0
                            onClicked: client.refreshCompose(root.composeFilePath)
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Up")
                            enabled: root.composeFilePath.trim().length > 0
                            onClicked: client.composeUp(root.composeFilePath)
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Down")
                            enabled: root.composeFilePath.trim().length > 0
                            onClicked: client.composeDown(root.composeFilePath)
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Restart all")
                            enabled: root.composeFilePath.trim().length > 0
                            onClicked: client.composeRestart(root.composeFilePath, "")
                        }
                    }
                }
            }

            ListView {
                id: composeList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: client.composeServices
                spacing: Theme.sp1

                delegate: Rectangle {
                    id: composeRow
                    required property var modelData

                    readonly property string service: String(modelData.service || modelData.name || "")
                    readonly property string status: String(modelData.status || "")
                    readonly property string state: String(modelData.state || "")
                    readonly property bool isRunning: Boolean(modelData.isRunning)
                    readonly property string image: String(modelData.image || "")
                    readonly property string ports: String(modelData.ports || "")

                    width: ListView.view.width
                    implicitHeight: 58
                    radius: Theme.radiusSm
                    color: rowMouse.containsMouse ? Theme.bgHover : "transparent"

                    MouseArea {
                        id: rowMouse
                        anchors.fill: parent
                        hoverEnabled: true
                        acceptedButtons: Qt.NoButton
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp1

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: composeRow.isRunning ? Theme.statusSuccess : Theme.textTertiary
                            }

                            Text {
                                Layout.preferredWidth: 180
                                text: composeRow.service
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.fillWidth: true
                                text: composeRow.image
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textSecondary
                                elide: Text.ElideRight
                            }

                            StatusPill {
                                text: composeRow.isRunning ? qsTr("Running") : qsTr("Stopped")
                                tone: composeRow.isRunning ? "success" : "neutral"
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: composeRow.status.length > 0 ? composeRow.status : composeRow.state
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                            }

                            Text {
                                Layout.preferredWidth: 180
                                text: composeRow.ports.length > 0 ? composeRow.ports : "—"
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                elide: Text.ElideRight
                                horizontalAlignment: Text.AlignRight
                            }

                            DockerRowButton {
                                glyph: "R"
                                tooltip: qsTr("Restart service")
                                onClicked: client.composeRestart(root.composeFilePath, composeRow.service)
                            }

                            DockerRowButton {
                                glyph: "L"
                                tooltip: qsTr("Service logs")
                                onClicked: root._openComposeLogsFor(composeRow.service)
                            }
                        }
                    }
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: !client.busy
                             && composeList.count === 0
                             && root.composeFilePath.trim().length > 0
                    icon: "container"
                    title: qsTr("No compose services")
                    description: qsTr("Load a compose file and its services will appear here.")
                }

                ToolEmptyState {
                    anchors.centerIn: parent
                    visible: !client.busy
                             && root.composeFilePath.trim().length === 0
                    icon: "container"
                    title: qsTr("Compose path required")
                    description: qsTr("Enter a compose file path to manage a stack on this host.")
                }
            }
        }
    }

    Rectangle {
        id: overlay
        anchors.fill: parent
        visible: client.status === PierDockerClient.Connecting
              || client.status === PierDockerClient.Failed
        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.88)

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            preventStealing: true
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        Rectangle {
            anchors.centerIn: parent
            width: Math.min(420, parent.width - Theme.sp8 * 2)
            implicitHeight: overlayColumn.implicitHeight + Theme.sp5 * 2
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg

            ColumnLayout {
                id: overlayColumn
                anchors.fill: parent
                anchors.margins: Theme.sp5
                spacing: Theme.sp3

                SectionLabel {
                    Layout.alignment: Qt.AlignHCenter
                    text: client.status === PierDockerClient.Connecting
                          ? qsTr("Connecting to Docker")
                          : qsTr("Failed")
                }

                Text {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.maximumWidth: parent.width - Theme.sp5 * 2
                    text: client.target.length > 0 ? client.target : qsTr("Docker")
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeH3
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                }

                Text {
                    visible: client.status === PierDockerClient.Failed
                    Layout.fillWidth: true
                    text: client.errorMessage.length > 0 ? client.errorMessage : qsTr("Unknown error")
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.statusError
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Item { Layout.fillWidth: true }

                    GhostButton {
                        compact: true
                        minimumWidth: 0
                        text: qsTr("Cancel")
                        onClicked: client.stop()
                    }

                    PrimaryButton {
                        compact: true
                        minimumWidth: 0
                        visible: client.status === PierDockerClient.Failed
                        text: qsTr("Retry")
                        onClicked: root._dispatchConnect()
                    }
                }
            }
        }
    }

    ModalDialogShell {
        id: runDialog
        open: root.runDialogOpen
        title: qsTr("Run Image")
        subtitle: root.runImageRef
        dialogWidth: 720
        dialogHeight: 620
        bodyPadding: Theme.sp4
        onRequestClose: root.runDialogOpen = false

        body: PierScrollView {
            id: runDialogScroll
            anchors.fill: parent
            clip: true
            contentWidth: width

            Item {
                width: runDialogScroll.width
                implicitHeight: runDialogBody.implicitHeight

                ColumnLayout {
                    id: runDialogBody
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    spacing: Theme.sp3

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Container name") }
                            PierTextField {
                                Layout.fillWidth: true
                                text: root.runContainerName
                                placeholder: qsTr("Optional")
                                onTextChanged: root.runContainerName = text
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Port mappings") }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("One per line, format HOST:CONTAINER")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }
                            PierTextArea {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 88
                                mono: true
                                text: root.runPortsText
                                wrapMode: TextArea.Wrap
                                font.pixelSize: Theme.sizeCaption
                                selectByMouse: true
                                onTextChanged: root.runPortsText = text
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Environment variables") }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("One per line, format KEY=value")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }
                            PierTextArea {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 88
                                mono: true
                                text: root.runEnvText
                                wrapMode: TextArea.Wrap
                                font.pixelSize: Theme.sizeCaption
                                selectByMouse: true
                                onTextChanged: root.runEnvText = text
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Volume mounts") }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("One per line, format HOST_PATH:CONTAINER_PATH")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }
                            PierTextArea {
                                Layout.fillWidth: true
                                Layout.preferredHeight: 88
                                mono: true
                                text: root.runVolumesText
                                wrapMode: TextArea.Wrap
                                font.pixelSize: Theme.sizeCaption
                                selectByMouse: true
                                onTextChanged: root.runVolumesText = text
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Restart policy") }
                            PierComboBox {
                                Layout.fillWidth: true
                                options: root.restartLabels
                                currentIndex: root.runRestartIndex
                                onActivated: (index) => root.runRestartIndex = index
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp3

                        ColumnLayout {
                            anchors.fill: parent
                            spacing: Theme.sp2

                            SectionLabel { text: qsTr("Command override") }
                            PierTextField {
                                Layout.fillWidth: true
                                text: root.runCommand
                                placeholder: qsTr("Optional, e.g. /bin/sh")
                                onTextChanged: root.runCommand = text
                            }
                        }
                    }
                }
            }
        }

        footer: Item {
            implicitHeight: runDialogFooter.implicitHeight

            RowLayout {
                id: runDialogFooter
                width: parent.width
                spacing: Theme.sp2

                GhostButton {
                    text: qsTr("Cancel")
                    onClicked: root.runDialogOpen = false
                }

                Item { Layout.fillWidth: true }

                PrimaryButton {
                    text: qsTr("Create and run")
                    enabled: root.runImageRef.length > 0
                    onClicked: root._submitRunDialog()
                }
            }
        }
    }

    component DockerRowButton : Rectangle {
        id: rowBtn
        required property string glyph
        required property string tooltip
        property bool danger: false
        property bool active: false
        signal clicked()

        implicitWidth: 22
        implicitHeight: 22
        radius: Theme.radiusSm
        color: rowBtn.active
               ? Theme.accentSubtle
               : (btnMouse.containsMouse
                  ? (rowBtn.danger
                     ? Qt.rgba(Theme.statusError.r, Theme.statusError.g, Theme.statusError.b, 0.12)
                     : Theme.accentSubtle)
                  : "transparent")
        border.color: rowBtn.active
                      ? Theme.accent
                      : (btnMouse.containsMouse
                         ? (rowBtn.danger ? Theme.statusError : Theme.borderDefault)
                         : "transparent")
        border.width: (rowBtn.active || btnMouse.containsMouse) ? 1 : 0

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

        Text {
            anchors.centerIn: parent
            text: rowBtn.glyph
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeCaption
            color: rowBtn.active
                   ? Theme.accent
                   : (btnMouse.containsMouse
                      ? (rowBtn.danger ? Theme.statusError : Theme.textPrimary)
                      : Theme.textSecondary)
        }

        MouseArea {
            id: btnMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: rowBtn.clicked()
        }

        PierToolTip {
            text: rowBtn.tooltip
            visible: btnMouse.containsMouse
        }
    }
}
