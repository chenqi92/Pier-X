import QtQuick
import QtQuick.Layouts
import Pier

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

    Component.onCompleted: _dispatchConnect()

    function _dispatchConnect() {
        if (root.sshHost.length === 0 || root.sshUser.length === 0) return
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

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp3

        // ─── Top bar ─────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Text {
                text: monitor.target.length > 0 ? monitor.target : qsTr("Server Monitor")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 200
                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
            Item { Layout.fillWidth: true }
            Text {
                text: monitor.uptime.length > 0 ? monitor.uptime : "—"
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                color: Theme.textSecondary
                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
            GhostButton {
                text: qsTr("↻ Probe")
                enabled: monitor.status === PierServerMonitor.Connected
                onClicked: monitor.probeOnce()
            }
        }

        // ─── Gauge cards ─────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            // CPU
            GaugeCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                title: qsTr("CPU")
                value: monitor.cpuPct
                label: _pctBar(monitor.cpuPct)
                barColor: _barColor(monitor.cpuPct)
            }

            // Memory
            GaugeCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                title: qsTr("Memory")
                value: monitor.memTotalMb > 0
                       ? (monitor.memUsedMb / monitor.memTotalMb * 100) : -1
                label: monitor.memTotalMb > 0
                       ? _fmtMb(monitor.memUsedMb) + " / " + _fmtMb(monitor.memTotalMb)
                       : "—"
                barColor: _barColor(monitor.memTotalMb > 0
                                    ? monitor.memUsedMb / monitor.memTotalMb * 100 : -1)
            }

            // Swap
            GaugeCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                title: qsTr("Swap")
                value: monitor.swapTotalMb > 0
                       ? (monitor.swapUsedMb / monitor.swapTotalMb * 100) : -1
                label: monitor.swapTotalMb > 0
                       ? _fmtMb(monitor.swapUsedMb) + " / " + _fmtMb(monitor.swapTotalMb)
                       : "—"
                barColor: _barColor(monitor.swapTotalMb > 0
                                    ? monitor.swapUsedMb / monitor.swapTotalMb * 100 : -1)
            }

            // Disk
            GaugeCard {
                Layout.fillWidth: true
                Layout.fillHeight: true
                title: qsTr("Disk /")
                value: monitor.diskUsePct
                label: monitor.diskTotal.length > 0
                       ? monitor.diskUsed + " / " + monitor.diskTotal
                       : "—"
                barColor: _barColor(monitor.diskUsePct)
            }
        }

        // ─── Load averages ───────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 36
            color: Theme.bgPanel
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                spacing: Theme.sp3

                SectionLabel { text: qsTr("Load avg") }

                Repeater {
                    model: [
                        { lbl: "1m", val: monitor.load1 },
                        { lbl: "5m", val: monitor.load5 },
                        { lbl: "15m", val: monitor.load15 }
                    ]
                    RowLayout {
                        required property var modelData
                        spacing: Theme.sp1
                        Text {
                            text: modelData.lbl
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            color: Theme.textTertiary
                        }
                        Text {
                            text: modelData.val >= 0 ? modelData.val.toFixed(2) : "—"
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: modelData.val >= 0 ? Theme.textPrimary : Theme.textTertiary
                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }
                    }
                }

                Item { Layout.fillWidth: true }

                Text {
                    visible: monitor.busy
                    text: "…"
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.accent
                }
            }
        }

        // Spacer to push everything up.
        Item { Layout.fillHeight: true }
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

    // ─── Inline gauge card component ─────────────────
    component GaugeCard : Rectangle {
        id: gc
        property string title: ""
        property double value: -1    // 0-100 or -1
        property string label: ""
        property color barColor: Theme.accent

        implicitHeight: 100
        color: Theme.bgPanel
        border.color: Theme.borderSubtle
        border.width: 1
        radius: Theme.radiusSm
        Behavior on color { ColorAnimation { duration: Theme.durNormal } }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp2

            Text {
                text: gc.title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textSecondary
            }

            // Horizontal bar.
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: 8
                radius: 4
                color: Theme.bgSurface
                Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                Rectangle {
                    width: gc.value >= 0
                           ? Math.max(parent.width * gc.value / 100, 2)
                           : 0
                    height: parent.height
                    radius: parent.radius
                    color: gc.barColor
                    Behavior on width { NumberAnimation { duration: Theme.durNormal } }
                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            Text {
                text: gc.label
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }
    }
}
