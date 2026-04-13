import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier
import "../components"

// Server resource monitor — M7b per-service tool.
//
// Layout:
//   ┌──────────────────────────────────────────────────┐
//   │ user@host                  up 5 days, 3:42       │
//   ├──────────┬──────────┬──────────┬─────────────────┤
//   │ CPU      │ Memory   │ Swap     │ Disk /          │
//   │ ██████░░ │ ██████░░ │ ░░░░░░░░ │ ████████░░░░░░░ │
//   │ 23.5%    │ 8.0 / 16 │ 0.1 / 2  │ 40G / 100G 42% │
//   ├──────────┴──────────┴──────────┴─────────────────┤
//   │ Load: 0.12  0.34  0.56                           │
//   └──────────────────────────────────────────────────┘
Rectangle {
    id: root

    clip: true
    property var sharedSession: null
    property string sshHost: ""
    property int    sshPort: 22
    property string sshUser: ""
    property string sshPassword: ""
    property string sshCredentialId: ""
    property string sshKeyPath: ""
    property string sshPassphraseCredentialId: ""
    property bool   sshUsesAgent: false

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierServerMonitor { id: monitor }

    Component.onCompleted: Qt.callLater(_dispatchConnect)

    Connections {
        target: root.sharedSession
        function onConnectedChanged() {
            if (root.sharedSession && root.sharedSession.connected)
                root._dispatchConnect()
        }
    }

    function _dispatchConnect() {
        if (monitor.status === PierServerMonitor.Connecting
                || monitor.status === PierServerMonitor.Connected)
            return
        if (root.sharedSession && root.sharedSession.connected) {
            monitor.connectToSession(root.sharedSession)
            return
        }
        if (root.sshHost.length === 0 || root.sshUser.length === 0) {
            // No SSH context — monitor local machine
            monitor.connectLocal()
            return
        }
        var kind = 0, secret = "", extra = ""
        if (root.sshUsesAgent) {
            kind = 3
        } else if (root.sshKeyPath.length > 0) {
            kind = 2; secret = root.sshKeyPath; extra = root.sshPassphraseCredentialId
        } else if (root.sshCredentialId.length > 0) {
            kind = 1; secret = root.sshCredentialId
        } else {
            kind = 0; secret = root.sshPassword
        }
        monitor.connectTo(root.sshHost, root.sshPort, root.sshUser, kind, secret, extra)
    }

    function _fmtMb(val) {
        if (val < 0) return "—"
        if (val < 1024) return val.toFixed(0) + " MB"
        return (val / 1024).toFixed(1) + " GB"
    }

    function _pctBar(pct) {
        if (pct < 0) return "—"
        return pct.toFixed(1) + "%"
    }

    function _barColor(pct) {
        if (pct < 0) return Theme.textTertiary
        if (pct < 60) return Theme.statusSuccess
        if (pct < 85) return Theme.statusWarning
        return Theme.statusError
    }

    PierScrollView {
        anchors.fill: parent
        anchors.margins: Theme.sp2
        clip: true

        ColumnLayout {
            width: Math.max(root.width - Theme.sp6, 280)
            spacing: Theme.sp2

            ToolHeroPanel {
                Layout.fillWidth: true
                compact: true
                accentColor: Theme.statusInfo

                ColumnLayout {
                    id: monitorHeader
                    anchors.fill: parent
                    spacing: Theme.sp2

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        compact: true
                        prominent: true
                        icon: "server"
                        title: monitor.hostname.length > 0
                               ? monitor.hostname
                               : qsTr("Server Monitor")
                        subtitle: monitor.target.length > 0
                                  ? monitor.target
                                  : (monitor.os.length > 0 ? monitor.os : qsTr("Waiting for system snapshot."))

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("Probe")
                            enabled: monitor.status === PierServerMonitor.Connected
                            onClicked: monitor.probeOnce()
                        }
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        StatusPill {
                            text: monitor.status === PierServerMonitor.Connected
                                  ? qsTr("Connected")
                                  : (monitor.status === PierServerMonitor.Connecting
                                     ? qsTr("Connecting")
                                     : qsTr("Idle"))
                            tone: monitor.status === PierServerMonitor.Connected
                                  ? "success"
                                  : (monitor.status === PierServerMonitor.Connecting ? "info" : "neutral")
                        }

                        StatusPill {
                            visible: monitor.cpuPct >= 0
                            text: qsTr("CPU %1").arg(_pctBar(monitor.cpuPct))
                            tone: "neutral"
                        }

                        StatusPill {
                            visible: monitor.memTotalMb > 0
                            text: qsTr("Memory %1").arg(_fmtMb(monitor.memUsedMb))
                            tone: "neutral"
                        }

                        StatusPill {
                            visible: monitor.diskTotal.length > 0
                            text: qsTr("Disk %1").arg(_pctBar(monitor.diskUsePct))
                            tone: "neutral"
                        }
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        ToolFactChip {
                            label: qsTr("Uptime")
                            value: monitor.uptime
                            monoValue: true
                        }

                        ToolFactChip {
                            label: qsTr("Load")
                            value: monitor.load1 >= 0
                                   ? (monitor.load1.toFixed(2) + " / "
                                      + monitor.load5.toFixed(2) + " / "
                                      + monitor.load15.toFixed(2))
                                   : ""
                            monoValue: true
                        }

                        ToolFactChip {
                            label: qsTr("Memory")
                            value: monitor.memTotalMb > 0
                                   ? (_fmtMb(monitor.memUsedMb) + " / " + _fmtMb(monitor.memTotalMb))
                                   : ""
                            monoValue: true
                        }

                        ToolFactChip {
                            label: qsTr("Disk")
                            value: monitor.diskUsed.length > 0 && monitor.diskTotal.length > 0
                                   ? (monitor.diskUsed + " / " + monitor.diskTotal)
                                   : ""
                            monoValue: true
                        }
                    }

                    ToolBanner {
                        Layout.fillWidth: true
                        tone: monitor.status === PierServerMonitor.Connected ? "neutral" : "info"
                        text: monitor.os.length > 0
                              ? monitor.os
                              : qsTr("Waiting for host details")
                    }
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                implicitHeight: overviewSections.implicitHeight + Theme.sp2 * 2
                padding: Theme.sp2

                GridLayout {
                    id: overviewSections
                    anchors.fill: parent
                    columns: width >= 640 ? 2 : 1
                    rowSpacing: Theme.sp2
                    columnSpacing: Theme.sp2

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp2
                        implicitHeight: overviewColumn.implicitHeight + Theme.sp2 * 2

                        ColumnLayout {
                            id: overviewColumn
                            anchors.fill: parent
                            spacing: Theme.sp2

                            ToolSectionHeader {
                                Layout.fillWidth: true
                                compact: true
                                icon: "server"
                                title: qsTr("Overview")
                                subtitle: monitor.os.length > 0 ? monitor.os : qsTr("Waiting for host details")
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp0_5

                                    Text {
                                        text: qsTr("Host")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        text: monitor.hostname.length > 0 ? monitor.hostname : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp0_5

                                    Text {
                                        text: qsTr("Uptime")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        text: monitor.uptime.length > 0 ? monitor.uptime : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }
                    }

                    ToolPanelSurface {
                        Layout.fillWidth: true
                        inset: true
                        padding: Theme.sp2
                        implicitHeight: capacityColumn.implicitHeight + Theme.sp2 * 2

                        ColumnLayout {
                            id: capacityColumn
                            anchors.fill: parent
                            spacing: Theme.sp2

                            ToolSectionHeader {
                                Layout.fillWidth: true
                                compact: true
                                icon: "hard-drive"
                                title: qsTr("Capacity")
                                subtitle: qsTr("Available memory, swap, and storage")
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp0_5

                                    Text {
                                        text: qsTr("Free memory")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        text: monitor.memTotalMb > 0 ? _fmtMb(monitor.memFreeMb) : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp0_5

                                    Text {
                                        text: qsTr("Swap")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        text: monitor.swapTotalMb > 0
                                              ? _fmtMb(monitor.swapUsedMb) + " / " + _fmtMb(monitor.swapTotalMb)
                                              : qsTr("Not configured")
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp0_5

                                    Text {
                                        text: qsTr("Storage available")
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeSmall
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        text: monitor.diskAvail.length > 0 ? monitor.diskAvail : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }
                    }
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                implicitHeight: resourceSection.implicitHeight + Theme.sp2 * 2
                padding: Theme.sp2

                ColumnLayout {
                    id: resourceSection
                    anchors.fill: parent
                    spacing: Theme.sp2

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        compact: true
                        icon: "activity"
                        title: qsTr("Resources")
                        subtitle: qsTr("Processor, memory, swap, and storage pressure")
                    }

                    GridLayout {
                        Layout.fillWidth: true
                        columns: width >= 560 ? 2 : 1
                        rowSpacing: Theme.sp2
                        columnSpacing: Theme.sp2

                        ToolMetricTile {
                            Layout.fillWidth: true
                            compact: true
                            title: qsTr("CPU")
                            valueText: _pctBar(monitor.cpuPct)
                            subtitle: monitor.busy ? qsTr("Refreshing snapshot") : qsTr("Processor usage")
                            progress: monitor.cpuPct
                            accentColor: _barColor(monitor.cpuPct)
                        }

                        ToolMetricTile {
                            Layout.fillWidth: true
                            compact: true
                            title: qsTr("Memory")
                            valueText: monitor.memTotalMb > 0
                                       ? _fmtMb(monitor.memUsedMb)
                                       : "—"
                            subtitle: monitor.memTotalMb > 0
                                      ? qsTr("%1 free of %2")
                                            .arg(_fmtMb(monitor.memFreeMb))
                                            .arg(_fmtMb(monitor.memTotalMb))
                                      : qsTr("Waiting for memory stats")
                            footerText: monitor.memTotalMb > 0
                                        ? _pctBar(monitor.memUsedMb / monitor.memTotalMb * 100)
                                        : ""
                            progress: monitor.memTotalMb > 0
                                      ? (monitor.memUsedMb / monitor.memTotalMb * 100)
                                      : -1
                            accentColor: _barColor(monitor.memTotalMb > 0
                                                    ? (monitor.memUsedMb / monitor.memTotalMb * 100)
                                                    : -1)
                        }

                        ToolMetricTile {
                            Layout.fillWidth: true
                            compact: true
                            title: qsTr("Swap")
                            valueText: monitor.swapTotalMb > 0
                                       ? _fmtMb(monitor.swapUsedMb)
                                       : qsTr("Not configured")
                            subtitle: monitor.swapTotalMb > 0
                                      ? qsTr("%1 total").arg(_fmtMb(monitor.swapTotalMb))
                                      : qsTr("No swap partition detected")
                            footerText: monitor.swapTotalMb > 0
                                        ? _pctBar(monitor.swapUsedMb / monitor.swapTotalMb * 100)
                                        : ""
                            progress: monitor.swapTotalMb > 0
                                      ? (monitor.swapUsedMb / monitor.swapTotalMb * 100)
                                      : -1
                            accentColor: _barColor(monitor.swapTotalMb > 0
                                                    ? (monitor.swapUsedMb / monitor.swapTotalMb * 100)
                                                    : -1)
                        }

                        ToolMetricTile {
                            Layout.fillWidth: true
                            compact: true
                            title: qsTr("Disk")
                            valueText: monitor.diskUsed.length > 0 ? monitor.diskUsed : "—"
                            subtitle: monitor.diskTotal.length > 0
                                      ? qsTr("%1 free").arg(monitor.diskAvail)
                                      : qsTr("Waiting for filesystem stats")
                            footerText: monitor.diskTotal.length > 0
                                        ? qsTr("%1 total").arg(monitor.diskTotal)
                                        : ""
                            progress: monitor.diskUsePct
                            accentColor: _barColor(monitor.diskUsePct)
                        }
                    }
                }
            }

            ToolPanelSurface {
                Layout.fillWidth: true
                implicitHeight: snapshotSection.implicitHeight + Theme.sp2 * 2
                padding: Theme.sp2

                ColumnLayout {
                    id: snapshotSection
                    anchors.fill: parent
                    spacing: Theme.sp2

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        compact: true
                        icon: "loader"
                        title: qsTr("Snapshot")
                        subtitle: qsTr("Host details and load averages")
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: [
                                { lbl: "1m", val: monitor.load1 },
                                { lbl: "5m", val: monitor.load5 },
                                { lbl: "15m", val: monitor.load15 }
                            ]

                            Rectangle {
                                required property var modelData

                                implicitWidth: loadValue.implicitWidth + loadLabel.implicitWidth + Theme.sp4
                                implicitHeight: 28
                                radius: Theme.radiusSm
                                color: Theme.bgInset
                                border.color: Theme.borderSubtle
                                border.width: 1

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp2
                                    anchors.rightMargin: Theme.sp2
                                    spacing: Theme.sp1

                                    Text {
                                        id: loadLabel
                                        text: modelData.lbl
                                        font.family: Theme.fontUi
                                        font.pixelSize: Theme.sizeCaption
                                        color: Theme.textTertiary
                                    }

                                    Text {
                                        id: loadValue
                                        text: modelData.val >= 0 ? modelData.val.toFixed(2) : "—"
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        font.weight: Theme.weightMedium
                                        color: modelData.val >= 0 ? Theme.textPrimary : Theme.textTertiary
                                    }
                                }
                            }
                        }
                    }

                    ToolBanner {
                        Layout.fillWidth: true
                        tone: monitor.busy ? "info" : "neutral"
                        text: monitor.hostname.length > 0
                              ? qsTr("%1 · %2").arg(monitor.hostname).arg(monitor.os)
                              : qsTr("Waiting for host details")
                    }
                }
            }
        }
    }

    // ─── Connecting / Failed overlay ─────────────────
    Rectangle {
        anchors.fill: parent
        visible: monitor.status === PierServerMonitor.Connecting
              || monitor.status === PierServerMonitor.Failed
        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.88)

        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; preventStealing: true; onPressed: (mouse) => mouse.accepted = true }

        Rectangle {
            id: card; anchors.centerIn: parent
            width: Math.min(380, parent.width - Theme.sp8 * 2)
            implicitHeight: cc.implicitHeight + Theme.sp5 * 2
            color: Theme.bgElevated; border.color: Theme.borderDefault; border.width: 1; radius: Theme.radiusLg
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                id: cc; anchors.fill: parent; anchors.margins: Theme.sp5; spacing: Theme.sp3
                SectionLabel {
                    text: monitor.status === PierServerMonitor.Connecting
                          ? qsTr("Connecting") : qsTr("Failed")
                    Layout.alignment: Qt.AlignHCenter
                }
                Text {
                    text: monitor.target.length > 0 ? monitor.target : "—"
                    Layout.alignment: Qt.AlignHCenter
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeH3; font.weight: Theme.weightMedium
                    color: Theme.textPrimary; elide: Text.ElideMiddle
                    Layout.maximumWidth: card.width - Theme.sp5 * 2
                }
                Text {
                    visible: monitor.status === PierServerMonitor.Failed
                    Layout.fillWidth: true; text: monitor.errorMessage.length > 0 ? monitor.errorMessage : qsTr("Unknown error")
                    wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeCaption; color: Theme.statusError
                }
                RowLayout {
                    Layout.fillWidth: true; Layout.topMargin: Theme.sp3; spacing: Theme.sp2
                    Item { Layout.fillWidth: true }
                    GhostButton { text: qsTr("Cancel"); onClicked: monitor.stop() }
                    PrimaryButton { text: qsTr("Retry"); visible: monitor.status === PierServerMonitor.Failed; onClicked: _dispatchConnect() }
                }
            }
        }
    }

}
