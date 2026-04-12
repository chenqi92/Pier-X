import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier

// MySQL client panel — M5d per-service tool. Final M5 slice.
//
// Layout
// ──────
//   ┌───────────────────────────────────────────────────┐
//   │ root@127.0.0.1:13306    [db:testdb ▾] [↻ Refresh] │  top bar
//   ├───────────────────────────────────────────────────┤
//   │ SELECT * FROM users WHERE id < 10;               │  SQL editor
//   │                                          [Run ▸] │
//   ├───────────────────────────────────────────────────┤
//   │ id │ name  │ email                                │  result grid
//   │ 1  │ alice │ alice@example.com                    │  (TableView)
//   │ 2  │ null  │ bob@example.com                      │
//   │ …                                                 │
//   ├───────────────────────────────────────────────────┤
//   │ 2 rows · 12 ms                                    │  footer
//   └───────────────────────────────────────────────────┘
//
// Before connect: a centered connect form overlays the whole
// panel so the user can fill in host/user/password/db and
// click Connect.
Rectangle {
    id: root

    // Pre-filled by Main.qml when the tab is created (e.g.
    // from the palette entry which defaults to the tunnel
    // convention 127.0.0.1:13306).
    property string mysqlHost: "127.0.0.1"
    property int    mysqlPort: 13306
    property string mysqlUser: "root"
    property string mysqlDatabase: ""

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierMySqlClient {
        id: client
    }

    // Connect form state (not sent until user clicks Connect).
    property string formHost: root.mysqlHost
    property int    formPort: root.mysqlPort
    property string formUser: root.mysqlUser
    property string formPassword: ""
    property string formDatabase: root.mysqlDatabase

    function _submit() {
        client.connectTo(root.formHost, root.formPort,
                         root.formUser, root.formPassword,
                         root.formDatabase)
    }

    function _rowLabel(n) {
        return n === 1 ? qsTr("1 row") : qsTr("%1 rows").arg(n)
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
                      : qsTr("MySQL")
                font.family: Theme.fontMono
                font.pixelSize: Theme.sizeBody
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideMiddle
                Layout.minimumWidth: 180
                Layout.maximumWidth: 320

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }

            Item { Layout.fillWidth: true }

            GhostButton {
                compact: true
                minimumWidth: 0
                text: qsTr("↻ Databases")
                enabled: client.status === PierMySqlClient.Connected
                onClicked: client.refreshDatabases()
            }

            GhostButton {
                compact: true
                minimumWidth: 0
                text: qsTr("Disconnect")
                enabled: client.status === PierMySqlClient.Connected
                onClicked: client.stop()
            }
        }

        // ─── SQL editor ─────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(100, root.height * 0.28)
            color: Theme.bgPanel
            border.color: sqlEditor.activeFocus
                          ? Theme.borderFocus
                          : Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durFast } }

            ScrollView {
                anchors.fill: parent
                anchors.margins: Theme.sp2
                clip: true

                TextArea {
                    id: sqlEditor
                    placeholderText: qsTr("SELECT * FROM … ")
                    wrapMode: TextArea.WrapAtWordBoundaryOrAnywhere
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textPrimary
                    background: Rectangle { color: "transparent" }
                    selectByMouse: true

                    // Ctrl/Cmd+Enter runs the query. Fall back
                    // to Keys rather than a Shortcut so the
                    // binding is scoped to the editor.
                    Keys.onPressed: (event) => {
                        if ((event.modifiers & Qt.ControlModifier)
                            && (event.key === Qt.Key_Return
                                || event.key === Qt.Key_Enter)) {
                            event.accepted = true
                            client.execute(sqlEditor.text)
                        }
                    }

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            // Run button — anchored bottom-right inside the
            // editor card so it sits above the scrollable area
            // without consuming layout height.
            PrimaryButton {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.rightMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                text: qsTr("Run ▸")
                enabled: client.status === PierMySqlClient.Connected
                         && sqlEditor.text.trim().length > 0
                         && !client.busy
                onClicked: client.execute(sqlEditor.text)
            }
        }

        // ─── Result area ────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.bgPanel
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm

            Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            // Error view: shown when the last execute returned
            // a server error.
            Rectangle {
                anchors.fill: parent
                anchors.margins: Theme.sp3
                color: "transparent"
                visible: client.lastError.length > 0

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp2

                    SectionLabel { text: qsTr("Error") }

                    Text {
                        Layout.fillWidth: true
                        text: client.lastError
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeBody
                        color: Theme.statusError
                        wrapMode: Text.Wrap
                    }
                }
            }

            // Result grid: HorizontalHeaderView + TableView
            // backed by client.resultModel. Shown when we have
            // a successful execute AND the last op wasn't DML.
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp1
                spacing: 0
                visible: client.lastError.length === 0
                         && client.resultModel.columnCount > 0

                HorizontalHeaderView {
                    id: headerRow
                    Layout.fillWidth: true
                    syncView: resultTable
                    clip: true

                    delegate: Rectangle {
                        implicitHeight: 22
                        implicitWidth: 120
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1

                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2
                            anchors.rightMargin: Theme.sp2
                            verticalAlignment: Text.AlignVCenter
                            horizontalAlignment: Text.AlignLeft
                            text: display
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            font.weight: Theme.weightMedium
                            color: Theme.textSecondary
                            elide: Text.ElideRight
                        }
                    }
                }

                TableView {
                    id: resultTable
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: client.resultModel
                    columnSpacing: 0
                    rowSpacing: 0
                    reuseItems: true

                    delegate: Rectangle {
                        implicitHeight: 22
                        implicitWidth: 120
                        required property var display
                        required property bool isNull
                        color: "transparent"
                        border.color: Theme.borderSubtle
                        border.width: 1

                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2
                            anchors.rightMargin: Theme.sp2
                            verticalAlignment: Text.AlignVCenter
                            horizontalAlignment: Text.AlignLeft
                            text: parent.isNull
                                  ? "NULL"
                                  : (parent.display !== undefined ? parent.display : "")
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: parent.isNull
                                   ? Theme.textTertiary
                                   : Theme.textPrimary
                            font.italic: parent.isNull
                            elide: Text.ElideRight

                            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                        }
                    }
                }
            }

            // DML success view: no columns but affected_rows > 0.
            Text {
                anchors.centerIn: parent
                visible: client.lastError.length === 0
                         && client.resultModel.columnCount === 0
                         && client.lastAffectedRows > 0
                text: qsTr("%1 rows affected").arg(client.lastAffectedRows)
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.statusSuccess
            }

            // Empty placeholder (connected, no query run yet).
            Text {
                anchors.centerIn: parent
                visible: client.status === PierMySqlClient.Connected
                         && !client.busy
                         && client.lastError.length === 0
                         && client.resultModel.columnCount === 0
                         && client.lastAffectedRows === 0
                text: qsTr("Type SQL above and press Ctrl+Enter to run")
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeBody
                color: Theme.textTertiary

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            }
        }

        // ─── Footer ──────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 20
            color: "transparent"

            RowLayout {
                anchors.fill: parent
                spacing: Theme.sp3

                Text {
                    text: client.resultModel.rowCount > 0
                          ? _rowLabel(client.resultModel.rowCount)
                              + (client.lastTruncated ? qsTr(" (truncated)") : "")
                          : (client.lastAffectedRows > 0
                             ? qsTr("%1 affected").arg(client.lastAffectedRows)
                             : "")
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: client.lastTruncated
                           ? Theme.statusWarning
                           : Theme.textTertiary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Item { Layout.fillWidth: true }

                Text {
                    visible: client.lastElapsedMs > 0
                    text: qsTr("%1 ms").arg(client.lastElapsedMs)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }

                Text {
                    visible: client.databases.length > 0
                    text: qsTr("%1 dbs").arg(client.databases.length)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }
        }
    }

    // ─── Connect form overlay ────────────────────────────
    // Shown when the panel is Idle or Failed. Covers the
    // whole view so the SQL editor / grid aren't usable
    // until we have a live connection.
    Rectangle {
        id: connectOverlay
        anchors.fill: parent
        visible: client.status === PierMySqlClient.Idle
              || client.status === PierMySqlClient.Connecting
              || client.status === PierMySqlClient.Failed
        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.92)

        Behavior on opacity { NumberAnimation { duration: Theme.durNormal } }

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            preventStealing: true
            onPressed: (mouse) => mouse.accepted = true
        }

        Rectangle {
            id: card
            anchors.centerIn: parent
            width: Math.min(440, parent.width - Theme.sp8 * 2)
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
                    text: client.status === PierMySqlClient.Connecting
                          ? qsTr("Connecting to MySQL")
                          : (client.status === PierMySqlClient.Failed
                             ? qsTr("Connection failed")
                             : qsTr("MySQL connection"))
                    Layout.alignment: Qt.AlignHCenter
                }

                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    rowSpacing: Theme.sp2
                    columnSpacing: Theme.sp3

                    Text {
                        text: qsTr("Host")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        Layout.fillWidth: true
                        text: root.formHost
                        onTextChanged: root.formHost = text
                    }

                    Text {
                        text: qsTr("Port")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        Layout.fillWidth: true
                        text: root.formPort + ""
                        onTextChanged: {
                            const n = parseInt(text, 10)
                            if (!isNaN(n) && n > 0 && n <= 65535)
                                root.formPort = n
                        }
                    }

                    Text {
                        text: qsTr("User")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        Layout.fillWidth: true
                        text: root.formUser
                        onTextChanged: root.formUser = text
                    }

                    Text {
                        text: qsTr("Password")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        Layout.fillWidth: true
                        password: true
                        text: root.formPassword
                        onTextChanged: root.formPassword = text
                    }

                    Text {
                        text: qsTr("Database")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        Layout.fillWidth: true
                        placeholder: qsTr("(optional)")
                        text: root.formDatabase
                        onTextChanged: root.formDatabase = text
                    }
                }

                Text {
                    visible: client.status === PierMySqlClient.Failed
                             && client.errorMessage.length > 0
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.sp2
                    text: client.errorMessage
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

                    PrimaryButton {
                        text: client.status === PierMySqlClient.Connecting
                              ? qsTr("Connecting…")
                              : qsTr("Connect")
                        enabled: client.status !== PierMySqlClient.Connecting
                                 && root.formHost.length > 0
                                 && root.formUser.length > 0
                        onClicked: root._submit()
                    }
                }
            }
        }
    }
}
