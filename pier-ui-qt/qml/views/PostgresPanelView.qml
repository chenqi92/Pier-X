import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier

// PostgreSQL client panel — M7a per-service tool.
// Mirrors MySqlPanelView in layout + flow: connect form overlay
// → SQL editor (top) + result grid (bottom).
Rectangle {
    id: root

    property string pgHost: "127.0.0.1"
    property int    pgPort: 15432
    property string pgUser: "postgres"
    property string pgDatabase: ""

    property string formHost: root.pgHost
    property int    formPort: root.pgPort
    property string formUser: root.pgUser
    property string formPassword: ""
    property string formDatabase: root.pgDatabase

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierPostgresClient { id: client }

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
                text: client.target.length > 0 ? client.target : qsTr("PostgreSQL")
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
                text: qsTr("↻ Databases")
                enabled: client.status === PierPostgresClient.Connected
                onClicked: client.refreshDatabases()
            }
            GhostButton {
                text: qsTr("Disconnect")
                enabled: client.status === PierPostgresClient.Connected
                onClicked: client.stop()
            }
        }

        // ─── SQL editor ─────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(100, root.height * 0.28)
            color: Theme.bgPanel
            border.color: sqlEditor.activeFocus ? Theme.borderFocus : Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
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
                    Keys.onPressed: (event) => {
                        if ((event.modifiers & Qt.ControlModifier)
                            && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) {
                            event.accepted = true
                            client.execute(sqlEditor.text)
                        }
                    }
                }
            }
            PrimaryButton {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.rightMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                text: qsTr("Run ▸")
                enabled: client.status === PierPostgresClient.Connected
                         && sqlEditor.text.trim().length > 0 && !client.busy
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
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            // Error
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp3
                spacing: Theme.sp2
                visible: client.lastError.length > 0
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

            // Grid
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp1
                spacing: 0
                visible: client.lastError.length === 0 && client.resultColumnCount > 0

                HorizontalHeaderView {
                    Layout.fillWidth: true
                    syncView: resultTable
                    clip: true
                    delegate: Rectangle {
                        implicitHeight: 22; implicitWidth: 120
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle; border.width: 1
                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2; anchors.rightMargin: Theme.sp2
                            verticalAlignment: Text.AlignVCenter
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
                    Layout.fillWidth: true; Layout.fillHeight: true
                    clip: true; model: client.resultModel
                    columnSpacing: 0; rowSpacing: 0; reuseItems: true
                    delegate: Rectangle {
                        implicitHeight: 22; implicitWidth: 120
                        required property var display
                        required property bool isNull
                        color: "transparent"
                        border.color: Theme.borderSubtle; border.width: 1
                        Text {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp2; anchors.rightMargin: Theme.sp2
                            verticalAlignment: Text.AlignVCenter
                            text: parent.isNull ? "NULL" : (parent.display !== undefined ? parent.display : "")
                            font.family: Theme.fontMono
                            font.pixelSize: Theme.sizeCaption
                            color: parent.isNull ? Theme.textTertiary : Theme.textPrimary
                            font.italic: parent.isNull
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            // DML success
            Text {
                anchors.centerIn: parent
                visible: client.lastError.length === 0
                         && client.resultColumnCount === 0
                         && client.lastAffectedRows > 0
                text: qsTr("%1 rows affected").arg(client.lastAffectedRows)
                font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody
                color: Theme.statusSuccess
            }

            // Empty placeholder
            Text {
                anchors.centerIn: parent
                visible: client.status === PierPostgresClient.Connected
                         && !client.busy
                         && client.lastError.length === 0
                         && client.resultColumnCount === 0
                         && client.lastAffectedRows === 0
                text: qsTr("Type SQL above and press Ctrl+Enter to run")
                font.family: Theme.fontUi; font.pixelSize: Theme.sizeBody
                color: Theme.textTertiary
            }
        }

        // ─── Footer ──────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 20; color: "transparent"
            RowLayout {
                anchors.fill: parent; spacing: Theme.sp3
                Text {
                    text: client.resultRowCount > 0
                          ? _rowLabel(client.resultRowCount) + (client.lastTruncated ? qsTr(" (truncated)") : "")
                          : (client.lastAffectedRows > 0 ? qsTr("%1 affected").arg(client.lastAffectedRows) : "")
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeCaption
                    color: client.lastTruncated ? Theme.statusWarning : Theme.textTertiary
                }
                Item { Layout.fillWidth: true }
                Text {
                    visible: client.lastElapsedMs > 0
                    text: qsTr("%1 ms").arg(client.lastElapsedMs)
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                }
                Text {
                    visible: client.databases.length > 0
                    text: qsTr("%1 dbs").arg(client.databases.length)
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary
                }
            }
        }
    }

    // ─── Connect form overlay ────────────────────────────
    Rectangle {
        anchors.fill: parent
        visible: client.status === PierPostgresClient.Idle
              || client.status === PierPostgresClient.Connecting
              || client.status === PierPostgresClient.Failed
        color: Qt.rgba(Theme.bgCanvas.r, Theme.bgCanvas.g, Theme.bgCanvas.b, 0.92)

        MouseArea {
            anchors.fill: parent; acceptedButtons: Qt.AllButtons
            preventStealing: true
            onPressed: (mouse) => mouse.accepted = true
        }

        Rectangle {
            id: card
            anchors.centerIn: parent
            width: Math.min(440, parent.width - Theme.sp8 * 2)
            implicitHeight: cardCol.implicitHeight + Theme.sp5 * 2
            color: Theme.bgElevated
            border.color: Theme.borderDefault; border.width: 1
            radius: Theme.radiusLg
            Behavior on color { ColorAnimation { duration: Theme.durNormal } }

            ColumnLayout {
                id: cardCol
                anchors.fill: parent; anchors.margins: Theme.sp5
                spacing: Theme.sp3

                SectionLabel {
                    text: client.status === PierPostgresClient.Connecting
                          ? qsTr("Connecting to PostgreSQL")
                          : (client.status === PierPostgresClient.Failed
                             ? qsTr("Connection failed")
                             : qsTr("PostgreSQL connection"))
                    Layout.alignment: Qt.AlignHCenter
                }

                GridLayout {
                    Layout.fillWidth: true; columns: 2
                    rowSpacing: Theme.sp2; columnSpacing: Theme.sp3

                    Text { text: qsTr("Host"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeCaption; color: Theme.textSecondary }
                    PierTextField { Layout.fillWidth: true; text: root.formHost; onTextChanged: root.formHost = text }

                    Text { text: qsTr("Port"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeCaption; color: Theme.textSecondary }
                    PierTextField { Layout.fillWidth: true; text: root.formPort + ""; onTextChanged: { const n = parseInt(text, 10); if (!isNaN(n) && n > 0 && n <= 65535) root.formPort = n } }

                    Text { text: qsTr("User"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeCaption; color: Theme.textSecondary }
                    PierTextField { Layout.fillWidth: true; text: root.formUser; onTextChanged: root.formUser = text }

                    Text { text: qsTr("Password"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeCaption; color: Theme.textSecondary }
                    PierTextField { Layout.fillWidth: true; password: true; text: root.formPassword; onTextChanged: root.formPassword = text }

                    Text { text: qsTr("Database"); font.family: Theme.fontUi; font.pixelSize: Theme.sizeCaption; color: Theme.textSecondary }
                    PierTextField { Layout.fillWidth: true; placeholder: qsTr("(optional)"); text: root.formDatabase; onTextChanged: root.formDatabase = text }
                }

                Text {
                    visible: client.status === PierPostgresClient.Failed && client.errorMessage.length > 0
                    Layout.fillWidth: true; Layout.topMargin: Theme.sp2
                    text: client.errorMessage
                    wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
                    font.family: Theme.fontMono; font.pixelSize: Theme.sizeCaption
                    color: Theme.statusError
                }

                RowLayout {
                    Layout.fillWidth: true; Layout.topMargin: Theme.sp3
                    spacing: Theme.sp2
                    Item { Layout.fillWidth: true }
                    PrimaryButton {
                        text: client.status === PierPostgresClient.Connecting
                              ? qsTr("Connecting…") : qsTr("Connect")
                        enabled: client.status !== PierPostgresClient.Connecting
                                 && root.formHost.length > 0 && root.formUser.length > 0
                        onClicked: root._submit()
                    }
                }
            }
        }
    }
}
