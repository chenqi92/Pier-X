import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier

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
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // ─── Top bar ─────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Text {
                text: client.target.length > 0
                      ? client.target
                      : qsTr("Redis")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 180
                Layout.maximumWidth: 260

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            Item { Layout.fillWidth: true }

            PierTextField {
                id: patternField
                implicitWidth: 220
                placeholder: qsTr("SCAN pattern (e.g. user:*)")
                text: root.scanPattern
                onTextChanged: root.scanPattern = text
            }

            PrimaryButton {
                text: qsTr("Scan")
                enabled: client.status === PierRedisClient.Connected
                onClicked: client.scanKeys(root.scanPattern, root.scanLimit)
            }
        }

        // ─── Split: key list | inspector ────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: "transparent"

            RowLayout {
                anchors.fill: parent
                spacing: Theme.sp2

                // Key list.
                Rectangle {
                    Layout.preferredWidth: 260
                    Layout.minimumWidth: 200
                    Layout.fillHeight: true
                    color: Theme.bgPanel
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp1
                        spacing: 0

                        ListView {
                            id: keyList
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
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

                        // Footer: count + truncated hint.
                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 22
                            color: Theme.bgSurface
                            radius: Theme.radiusSm

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.left: parent.left
                                anchors.leftMargin: Theme.sp2
                                text: client.keys.length + " "
                                      + (client.keys.length === 1 ? qsTr("key") : qsTr("keys"))
                                      + (client.keysTruncated ? " (truncated)" : "")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                color: client.keysTruncated
                                       ? Theme.statusWarning
                                       : Theme.textTertiary

                                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                            }
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
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.bgPanel
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp3
                        spacing: Theme.sp2

                        // Empty state: no key selected.
                        Text {
                            visible: client.selectedKey.length === 0
                            Layout.alignment: Qt.AlignHCenter
                            Layout.topMargin: Theme.sp8
                            text: client.status === PierRedisClient.Connected
                                  ? qsTr("Select a key to inspect")
                                  : qsTr("Connecting…")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            color: Theme.textTertiary

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        // Header: key name.
                        Text {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            text: client.selectedKey
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeH3
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                            elide: Text.ElideMiddle

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }

                        // Metadata row: type, ttl, length, encoding.
                        GridLayout {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            columns: 4
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

                        Separator {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            Layout.topMargin: Theme.sp2
                        }

                        // Preview list.
                        ScrollView {
                            visible: client.selectedKey.length > 0
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true

                            TextArea {
                                readOnly: true
                                wrapMode: TextArea.NoWrap
                                text: client.selectedPreview.join("\n")
                                      + (client.selectedPreviewTruncated
                                         ? "\n\n" + qsTr("… preview truncated")
                                         : "")
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textPrimary
                                background: Rectangle { color: "transparent" }
                                selectByMouse: true

                                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                            }
                        }
                    }
                }
            }
        }

        // ─── Server info footer ──────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 22
            color: "transparent"

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                text: {
                    var v = client.serverInfo.redis_version
                    var m = client.serverInfo.redis_mode
                    if (v && m) return qsTr("Redis %1 (%2)").arg(v).arg(m)
                    if (v)      return qsTr("Redis %1").arg(v)
                    return ""
                }
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeCaption
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
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
