import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier
import "../components"

// Redis browser panel — M5a per-service tool.
//
// Layout
// ──────
//   ┌───────────────────────────────────────────────────┐
//   │ 127.0.0.1:16379 / db 0        [pattern] [Scan]    │  top bar
//   ├─────────────────────┬─────────────────────────────┤
//   │ user:1              │ Key    user:1               │  details
//   │ user:2              │ Type   hash       TTL  ∞    │
//   │ session:abc         │ Length 5          Enc  lp   │
//   │ ...                 │ ───────────────────────────  │
//   │                     │ name = alice                │  preview
//   │                     │ age  = 30                   │
//   │                     │ ...                         │
//   └─────────────────────┴─────────────────────────────┘
//
// The view is 100% driven by PierRedisClient properties and
// signals — this file contains no business logic, only layout
// and bindings. Scan pattern editing updates a local state and
// fires a new scan on Enter / Scan button click.
Rectangle {
    id: root

    clip: true
    // Bound from Main.qml's tab model. M5a always targets a
    // localhost port (the tunnel), so no auth — just host/port/db.
    property string redisHost: "127.0.0.1"
    property int    redisPort: 0
    property int    redisDb: 0

    // Local view state. The pattern lives here instead of on
    // the backend so typing doesn't re-fire SCANs on every key.
    property string scanPattern: "*"
    property int    scanLimit: 500

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierRedisClient {
        id: client
    }

    Component.onCompleted: _dispatchConnect()

    function _dispatchConnect() {
        if (root.redisHost.length === 0 || root.redisPort <= 0) {
            console.warn("RedisBrowserView: missing host/port")
            return
        }
        client.connectTo(root.redisHost, root.redisPort, root.redisDb)
    }

    function _formatTtl(ttl) {
        if (ttl === -1) return qsTr("∞")
        if (ttl === -2) return qsTr("—")
        if (ttl < 60) return ttl + " s"
        if (ttl < 3600) return Math.round(ttl / 60) + " m"
        if (ttl < 86400) return Math.round(ttl / 3600) + " h"
        return Math.round(ttl / 86400) + " d"
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp2
        spacing: Theme.sp2

        ToolHeroPanel {
            Layout.fillWidth: true
            compact: true
            accentColor: Theme.accent

            ColumnLayout {
                id: redisHeader
                anchors.fill: parent
                spacing: Theme.sp2

                ToolSectionHeader {
                    Layout.fillWidth: true
                    compact: true
                    prominent: true
                    icon: "database"
                    title: qsTr("Redis")
                    subtitle: client.target.length > 0
                              ? client.target
                              : (root.redisHost + ":" + root.redisPort)

                    GhostButton {
                        compact: true
                        minimumWidth: 0
                        text: qsTr("Retry")
                        visible: client.status === PierRedisClient.Failed
                        onClicked: _dispatchConnect()
                    }

                    PrimaryButton {
                        compact: true
                        text: qsTr("Scan")
                        enabled: client.status === PierRedisClient.Connected
                        onClicked: client.scanKeys(root.scanPattern, root.scanLimit)
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    StatusPill {
                        text: client.status === PierRedisClient.Connected
                              ? qsTr("Connected")
                              : (client.status === PierRedisClient.Connecting
                                 ? qsTr("Connecting")
                                 : qsTr("Idle"))
                        tone: client.status === PierRedisClient.Connected ? "info" : "neutral"
                    }

                    StatusPill {
                        text: qsTr("DB %1").arg(root.redisDb)
                        tone: "neutral"
                    }

                    StatusPill {
                        visible: client.selectedKey.length > 0
                        text: client.selectedKind.length > 0
                              ? client.selectedKind
                              : qsTr("Inspector")
                        tone: "neutral"
                    }

                    StatusPill {
                        text: qsTr("%1 matches").arg(client.keys.length)
                        tone: client.keysTruncated ? "warning" : "neutral"
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    ToolFactChip {
                        label: qsTr("Database")
                        value: String(root.redisDb)
                        monoValue: true
                    }

                    ToolFactChip {
                        label: qsTr("Pattern")
                        value: root.scanPattern
                        monoValue: true
                    }

                    ToolFactChip {
                        label: qsTr("Matches")
                        value: String(client.keys.length)
                        monoValue: true
                    }
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    inset: true
                    padding: Theme.sp2
                    implicitHeight: scanControls.implicitHeight + Theme.sp2 * 2

                    Flow {
                        id: scanControls
                        anchors.fill: parent
                        spacing: Theme.sp2

                        PierTextField {
                            id: patternField
                            width: Math.max(220, Math.min(320, root.width * 0.4))
                            placeholder: qsTr("SCAN pattern (e.g. user:*)")
                            text: root.scanPattern
                            onTextChanged: root.scanPattern = text
                        }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Retry")
                            enabled: client.status === PierRedisClient.Failed
                            visible: client.status === PierRedisClient.Failed
                            onClicked: _dispatchConnect()
                        }

                        PrimaryButton {
                            text: qsTr("Scan")
                            enabled: client.status === PierRedisClient.Connected
                            onClicked: client.scanKeys(root.scanPattern, root.scanLimit)
                        }
                    }
                }
            }
        }

        // ─── Split: key list | inspector ────────────────
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            RowLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                // Key list.
                ToolPanelSurface {
                    Layout.preferredWidth: 260
                    Layout.minimumWidth: 200
                    Layout.fillHeight: true

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: 0

                        ToolSectionHeader {
                            Layout.fillWidth: true
                            compact: true
                            icon: "database"
                            title: qsTr("Keys")
                            subtitle: root.scanPattern.length > 0
                                      ? qsTr("%1 matches · %2").arg(client.keys.length).arg(root.scanPattern)
                                      : qsTr("%1 matches").arg(client.keys.length)
                        }

                        ToolPanelSurface {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            inset: true
                            padding: Theme.sp1_5

                            ListView {
                                id: keyList
                                anchors.fill: parent
                                clip: true
                                boundsBehavior: Flickable.StopAtBounds
                                model: client.keys
                                currentIndex: -1

                                delegate: Rectangle {
                                    id: row
                                    required property int index
                                    required property string modelData

                                    width: ListView.view.width
                                    implicitHeight: 24
                                    color: ListView.isCurrentItem
                                           ? Theme.accentSubtle
                                           : (mouseArea.containsMouse
                                              ? Theme.bgHover
                                              : "transparent")
                                    radius: Theme.radiusSm

                                    Behavior on color { ColorAnimation { duration: Theme.durFast } }

                                    Text {
                                        anchors.left: parent.left
                                        anchors.right: parent.right
                                        anchors.verticalCenter: parent.verticalCenter
                                        anchors.leftMargin: Theme.sp2
                                        anchors.rightMargin: Theme.sp2
                                        text: row.modelData
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: row.ListView.isCurrentItem
                                                     ? Theme.weightMedium
                                                     : Theme.weightRegular
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight

                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }

                                    MouseArea {
                                        id: mouseArea
                                        anchors.fill: parent
                                        hoverEnabled: true
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: {
                                            keyList.currentIndex = row.index
                                            client.inspect(row.modelData)
                                        }
                                    }
                                }
                            }
                        }

                        // Footer: count + truncated hint.
                        ToolBanner {
                            Layout.fillWidth: true
                            tone: client.keysTruncated ? "warning" : "neutral"
                            text: client.keys.length + " "
                                  + (client.keys.length === 1 ? qsTr("key") : qsTr("keys"))
                                  + (client.keysTruncated ? " (truncated)" : "")
                        }
                    }

                    // Busy spinner overlay for the key list.
                    Text {
                        anchors.centerIn: parent
                        visible: client.busy && client.keys.length === 0
                        text: qsTr("Scanning…")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textSecondary
                    }
                }

                // Inspector.
                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumWidth: 360

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp3
                        spacing: Theme.sp2

                        ToolSectionHeader {
                            Layout.fillWidth: true
                            compact: true
                            title: client.selectedKey.length > 0 ? client.selectedKey : qsTr("Inspector")
                            subtitle: client.selectedKey.length > 0
                                      ? (client.selectedKind.length > 0 ? client.selectedKind : qsTr("Redis key"))
                                      : qsTr("Select a key from the list to inspect its payload.")
                        }

                        ToolPanelSurface {
                            visible: client.selectedKey.length === 0
                            Layout.fillWidth: true
                            inset: true
                            padding: Theme.sp3
                            implicitHeight: serverSummary.implicitHeight + Theme.sp3 * 2

                            ColumnLayout {
                                id: serverSummary
                                anchors.fill: parent
                                spacing: Theme.sp2

                                ToolSectionHeader {
                                    Layout.fillWidth: true
                                    compact: true
                                    icon: "activity"
                                    title: client.serverInfo.redis_version
                                           ? qsTr("Redis %1").arg(client.serverInfo.redis_version)
                                           : qsTr("Redis server")
                                    subtitle: client.serverInfo.redis_mode
                                              ? qsTr("%1 mode").arg(client.serverInfo.redis_mode)
                                              : qsTr("Select a key to inspect values and metadata.")
                                }

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 360 ? 2 : 1
                                    rowSpacing: Theme.sp2
                                    columnSpacing: Theme.sp2

                                    ToolMetricTile {
                                        Layout.fillWidth: true
                                        compact: true
                                        title: qsTr("Clients")
                                        valueText: client.serverInfo.connected_clients || "—"
                                        subtitle: qsTr("Connected sessions")
                                        accentColor: Theme.statusSuccess
                                    }

                                    ToolMetricTile {
                                        Layout.fillWidth: true
                                        compact: true
                                        title: qsTr("Memory")
                                        valueText: client.serverInfo.used_memory_human || "—"
                                        subtitle: qsTr("Used memory")
                                        accentColor: Theme.accent
                                    }

                                    ToolMetricTile {
                                        Layout.fillWidth: true
                                        compact: true
                                        title: qsTr("Uptime")
                                        valueText: client.serverInfo.uptime_in_days
                                                   ? qsTr("%1 days").arg(client.serverInfo.uptime_in_days)
                                                   : "—"
                                        subtitle: qsTr("Server uptime")
                                        accentColor: Theme.statusInfo
                                    }

                                    ToolMetricTile {
                                        Layout.fillWidth: true
                                        compact: true
                                        title: qsTr("Database")
                                        valueText: qsTr("DB %1").arg(root.redisDb)
                                        subtitle: root.scanPattern.length > 0
                                                  ? root.scanPattern
                                                  : qsTr("Active pattern")
                                        accentColor: Theme.statusWarning
                                    }
                                }

                                ToolEmptyState {
                                    compact: true
                                    Layout.fillWidth: true
                                    icon: "database"
                                    title: client.status === PierRedisClient.Connected
                                           ? qsTr("Select a key to inspect")
                                           : qsTr("Connecting…")
                                    description: qsTr("Preview, type, TTL, encoding, and sampled values will appear here.")
                                }
                            }
                        }

                        ToolPanelSurface {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            inset: true
                            padding: Theme.sp3
                            implicitHeight: overviewColumn.implicitHeight + Theme.sp3 * 2

                            ColumnLayout {
                                id: overviewColumn
                                anchors.fill: parent
                                spacing: Theme.sp2

                                ToolSectionHeader {
                                    Layout.fillWidth: true
                                    compact: true
                                    icon: "activity"
                                    title: qsTr("Overview")
                                    subtitle: client.selectedKey
                                }

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 360 ? 4 : 2
                                    rowSpacing: Theme.sp1
                                    columnSpacing: Theme.sp3

                                    SectionLabel { text: qsTr("Type") }
                                    Text {
                                        text: client.selectedKind.length > 0 ? client.selectedKind : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    SectionLabel { text: qsTr("TTL") }
                                    Text {
                                        text: _formatTtl(client.selectedTtl)
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }

                                    SectionLabel { text: qsTr("Length") }
                                    Text {
                                        text: client.selectedLength + ""
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                    SectionLabel { text: qsTr("Encoding") }
                                    Text {
                                        text: client.selectedEncoding.length > 0
                                              ? client.selectedEncoding
                                              : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                                    }
                                }
                            }
                        }

                        ToolPanelSurface {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            inset: true
                            padding: Theme.sp3

                            ColumnLayout {
                                anchors.fill: parent
                                spacing: Theme.sp2

                                ToolSectionHeader {
                                    Layout.fillWidth: true
                                    compact: true
                                    icon: "file-text"
                                    title: qsTr("Preview")
                                    subtitle: client.selectedPreviewTruncated
                                              ? qsTr("Preview, type, TTL, encoding, and sampled values will appear here.")
                                              : ""
                                }

                                ToolPanelSurface {
                                    Layout.fillWidth: true
                                    Layout.fillHeight: true
                                    inset: true
                                    padding: Theme.sp0

                                    PierScrollView {
                                        anchors.fill: parent
                                        clip: true

                                        PierTextArea {
                                            readOnly: true
                                            frameVisible: false
                                            mono: true
                                            wrapMode: TextArea.NoWrap
                                            text: client.selectedPreview.join("\n")
                                                  + (client.selectedPreviewTruncated
                                                     ? "\n\n" + qsTr("… preview truncated")
                                                     : "")
                                            selectByMouse: true
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ─── Server info footer ──────────────────────────
        ToolBanner {
            Layout.fillWidth: true
            tone: "neutral"
            text: {
                var version = client.serverInfo.redis_version
                var mode = client.serverInfo.redis_mode
                if (version && mode) return qsTr("Redis %1 · %2 mode").arg(version).arg(mode)
                if (version) return qsTr("Redis %1").arg(version)
                return ""
            }
        }
    }

    // ─── Connecting / Failed overlay ───────────────────
    Rectangle {
        id: overlay

        anchors.fill: parent
        visible: client.status === PierRedisClient.Connecting
              || client.status === PierRedisClient.Failed

        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.88)

        Behavior on opacity { NumberAnimation { duration: Theme.durNormal } }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            preventStealing: true
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

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

                SectionLabel {
                    text: client.status === PierRedisClient.Connecting
                          ? qsTr("Opening Redis")
                          : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }

                Text {
                    text: client.target.length > 0 ? client.target : qsTr("Redis")
                    Layout.alignment: Qt.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeH3
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                    Layout.maximumWidth: card.width - Theme.sp5 * 2

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Text {
                    visible: client.status === PierRedisClient.Failed
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp2
                    text: client.errorMessage.length > 0
                          ? client.errorMessage
                          : qsTr("Unknown error")
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.statusError
                }

                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp3
                    spacing: Theme.sp2

                    Item { Layout.fillWidth: true }

                    GhostButton {
                        text: qsTr("Cancel")
                        onClicked: client.stop()
                    }
                    PrimaryButton {
                        text: qsTr("Retry")
                        visible: client.status === PierRedisClient.Failed
                        onClicked: _dispatchConnect()
                    }
                }
            }
        }
    }
}
