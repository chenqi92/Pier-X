import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier

// MySQL browser panel — tunnel-first service tool.
//
// Intended flow:
//   1. User opens a local SSH tunnel from the service strip.
//   2. This panel targets the local side of that tunnel
//      (typically 127.0.0.1:13306).
//   3. User supplies SQL credentials, then browses schemas or
//      runs ad-hoc queries against the forwarded endpoint.
Rectangle {
    id: root

    property string mysqlHost: "127.0.0.1"
    property int    mysqlPort: 0
    property string mysqlUser: ""
    property string mysqlPassword: ""
    property string mysqlDatabase: ""

    property string formHost: mysqlHost.length > 0 ? mysqlHost : "127.0.0.1"
    property string formPortText: mysqlPort > 0 ? String(mysqlPort) : "3306"
    property string formUser: mysqlUser
    property string formPassword: mysqlPassword
    property string formDatabase: mysqlDatabase
    property string selectedDatabase: mysqlDatabase
    property string selectedTable: ""
    property string sqlText: mysqlDatabase.length > 0
                             ? ("USE `" + mysqlDatabase + "`;\nSHOW TABLES;")
                             : "SELECT NOW() AS now;"

    readonly property bool hasResult: client.lastError.length > 0
                                      || client.resultColumnCount > 0
                                      || client.resultRowCount > 0
                                      || client.lastAffectedRows > 0
                                      || client.lastElapsedMs > 0

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierMySqlClient {
        id: client

        onDatabasesChanged: {
            if (root.selectedDatabase.length === 0 && databases.length > 0) {
                root.selectedDatabase = databases[0]
                root.formDatabase = databases[0]
                refreshTables(root.selectedDatabase)
            } else if (root.selectedDatabase.length > 0) {
                refreshTables(root.selectedDatabase)
            }
        }
    }

    Component.onCompleted: {
        if (root.mysqlHost.length > 0
            && root.mysqlPort > 0
            && root.mysqlUser.length > 0) {
            _connect()
        }
    }

    function _portValue() {
        var parsed = parseInt(root.formPortText, 10)
        return isNaN(parsed) ? 0 : parsed
    }

    function _connect() {
        var port = _portValue()
        if (root.formHost.length === 0 || root.formUser.length === 0 || port <= 0)
            return
        client.stop()
        root.selectedTable = ""
        root.selectedDatabase = root.formDatabase
        client.connectTo(root.formHost, port,
                         root.formUser, root.formPassword,
                         root.formDatabase)
    }

    function _refreshSchema() {
        client.refreshDatabases()
        if (root.selectedDatabase.length > 0)
            client.refreshTables(root.selectedDatabase)
    }

    function _previewTable(tableName) {
        if (tableName.length === 0)
            return
        root.selectedTable = tableName
        root.sqlText = "SELECT * FROM `" + tableName + "` LIMIT 200;"
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // Connection / toolbar row.
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Rectangle {
                implicitWidth: 168
                implicitHeight: 28
                color: Theme.bgPanel
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                Text {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2
                    verticalAlignment: Text.AlignVCenter
                    text: client.target.length > 0
                          ? client.target
                          : (root.formHost + ":" + root.formPortText)
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                }
            }

            PierTextField {
                implicitWidth: 150
                placeholder: qsTr("Host")
                text: root.formHost
                onTextChanged: root.formHost = text
            }

            PierTextField {
                implicitWidth: 78
                placeholder: qsTr("Port")
                text: root.formPortText
                onTextChanged: root.formPortText = text
            }

            PierTextField {
                implicitWidth: 118
                placeholder: qsTr("User")
                text: root.formUser
                onTextChanged: root.formUser = text
            }

            PierTextField {
                implicitWidth: 132
                placeholder: qsTr("Password")
                password: true
                text: root.formPassword
                onTextChanged: root.formPassword = text
            }

            PierTextField {
                implicitWidth: 140
                placeholder: qsTr("Default DB")
                text: root.formDatabase
                onTextChanged: root.formDatabase = text
            }

            PrimaryButton {
                text: client.status === PierMySqlClient.Connected
                      ? qsTr("Reconnect")
                      : qsTr("Connect")
                enabled: !client.busy
                onClicked: root._connect()
            }

            GhostButton {
                text: qsTr("Refresh")
                enabled: client.status === PierMySqlClient.Connected && !client.busy
                onClicked: root._refreshSchema()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 24
            color: client.lastError.length > 0
                   ? Theme.accentSubtle
                   : Theme.bgSurface
            border.color: client.lastError.length > 0
                          ? Theme.statusError
                          : Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusSm
            visible: client.errorMessage.length > 0 || client.lastError.length > 0

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

            Text {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                verticalAlignment: Text.AlignVCenter
                text: client.lastError.length > 0 ? client.lastError : client.errorMessage
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                color: client.lastError.length > 0 ? Theme.statusError : Theme.textSecondary
                elide: Text.ElideRight
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp2

            // Schema sidebar.
            Rectangle {
                Layout.preferredWidth: 260
                Layout.minimumWidth: 220
                Layout.fillHeight: true
                color: Theme.bgPanel
                border.color: Theme.borderSubtle
                border.width: 1
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: Theme.sp2
                    spacing: Theme.sp2

                    Text {
                        text: qsTr("Databases")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }

                    ListView {
                        id: databasesView
                        Layout.fillWidth: true
                        Layout.preferredHeight: 180
                        clip: true
                        spacing: 0
                        model: client.databases
                        currentIndex: -1

                        delegate: Rectangle {
                            id: dbRow
                            required property int index
                            required property string modelData

                            width: ListView.view.width
                            implicitHeight: 24
                            radius: Theme.radiusSm
                            color: root.selectedDatabase === dbRow.modelData
                                   ? Theme.accentSubtle
                                   : (dbMouse.containsMouse ? Theme.bgHover : "transparent")

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            Text {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                verticalAlignment: Text.AlignVCenter
                                text: dbRow.modelData
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            MouseArea {
                                id: dbMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    root.selectedDatabase = dbRow.modelData
                                    root.formDatabase = dbRow.modelData
                                    root.selectedTable = ""
                                    client.refreshTables(dbRow.modelData)
                                }
                            }
                        }
                    }

                    Text {
                        text: qsTr("Tables")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }

                    ListView {
                        id: tablesView
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: 0
                        model: client.tables
                        currentIndex: -1

                        delegate: Rectangle {
                            id: tableRow
                            required property int index
                            required property string modelData

                            width: ListView.view.width
                            implicitHeight: 24
                            radius: Theme.radiusSm
                            color: root.selectedTable === tableRow.modelData
                                   ? Theme.accentSubtle
                                   : (tableMouse.containsMouse ? Theme.bgHover : "transparent")

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            Text {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                verticalAlignment: Text.AlignVCenter
                                text: tableRow.modelData
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeBody
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            MouseArea {
                                id: tableMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root._previewTable(tableRow.modelData)
                            }
                        }
                    }
                }
            }

            // Query editor + result grid.
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.sp2

                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 188
                    color: Theme.bgPanel
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp2
                        spacing: Theme.sp2

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                text: qsTr("Query")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeCaption
                                font.weight: Theme.weightMedium
                                color: Theme.textSecondary
                            }

                            Item { Layout.fillWidth: true }

                            GhostButton {
                                text: qsTr("USE %1").arg(root.selectedDatabase)
                                visible: root.selectedDatabase.length > 0
                                enabled: client.status === PierMySqlClient.Connected
                                onClicked: {
                                    root.sqlText = "USE `" + root.selectedDatabase + "`;\n"
                                    sqlEditor.text = root.sqlText
                                }
                            }

                            PrimaryButton {
                                text: client.busy ? qsTr("Running…") : qsTr("Run")
                                enabled: client.status === PierMySqlClient.Connected
                                         && !client.busy
                                         && root.sqlText.trim().length > 0
                                onClicked: client.execute(root.sqlText)
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            color: Theme.bgCanvas
                            border.color: Theme.borderSubtle
                            border.width: 1
                            radius: Theme.radiusSm

                            Flickable {
                                anchors.fill: parent
                                anchors.margins: Theme.sp1
                                contentWidth: sqlEditor.contentWidth
                                contentHeight: sqlEditor.contentHeight
                                clip: true

                                TextEdit {
                                    id: sqlEditor
                                    width: Math.max(parent.width, contentWidth + Theme.sp4)
                                    height: Math.max(parent.height, contentHeight + Theme.sp4)
                                    text: root.sqlText
                                    wrapMode: TextEdit.NoWrap
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textPrimary
                                    selectionColor: Theme.accentMuted
                                    selectedTextColor: Theme.textPrimary
                                    persistentSelection: true
                                    onTextChanged: root.sqlText = text
                                }
                            }
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 24
                    color: Theme.bgSurface
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    Text {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp2
                        anchors.rightMargin: Theme.sp2
                        verticalAlignment: Text.AlignVCenter
                        text: client.lastError.length > 0
                              ? client.lastError
                              : (!root.hasResult
                                 ? qsTr("Open a tunnel, connect with SQL credentials, then run a query.")
                                 : (client.resultColumnCount > 0
                                    ? qsTr("%1 rows · %2 columns · %3 ms%4")
                                        .arg(client.resultRowCount)
                                        .arg(client.resultColumnCount)
                                        .arg(client.lastElapsedMs)
                                        .arg(client.lastTruncated ? qsTr(" · truncated") : "")
                                    : qsTr("%1 rows affected · %2 ms")
                                        .arg(client.lastAffectedRows)
                                        .arg(client.lastElapsedMs)))
                        font.family: client.lastError.length > 0 ? Theme.fontUi : Theme.fontMono
                        font.pixelSize: Theme.sizeCaption
                        color: client.lastError.length > 0 ? Theme.statusError : Theme.textSecondary
                        elide: Text.ElideRight
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.bgPanel
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusSm

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: Theme.sp1
                        spacing: 1

                        HorizontalHeaderView {
                            id: headerView
                            Layout.fillWidth: true
                            implicitHeight: 28
                            syncView: resultTable
                            visible: client.resultColumnCount > 0

                            delegate: Rectangle {
                                required property string display

                                implicitWidth: 180
                                implicitHeight: 28
                                color: Theme.bgSurface
                                border.color: Theme.borderSubtle
                                border.width: 1

                                Text {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp2
                                    anchors.rightMargin: Theme.sp2
                                    verticalAlignment: Text.AlignVCenter
                                    text: display
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeCaption
                                    font.weight: Theme.weightMedium
                                    color: Theme.textSecondary
                                    elide: Text.ElideRight
                                }
                            }
                        }

                        Item {
                            Layout.fillWidth: true
                            Layout.fillHeight: true

                            TableView {
                                id: resultTable
                                anchors.fill: parent
                                clip: true
                                boundsBehavior: Flickable.StopAtBounds
                                columnSpacing: 1
                                rowSpacing: 1
                                reuseItems: true
                                model: client.resultModel
                                visible: client.resultColumnCount > 0

                                columnWidthProvider: function(column) {
                                    return 180
                                }
                                rowHeightProvider: function(row) {
                                    return 28
                                }

                                delegate: Rectangle {
                                    required property var display
                                    required property bool isNull
                                    required property int row

                                    implicitWidth: 180
                                    implicitHeight: 28
                                    color: row % 2 === 0 ? "transparent" : Theme.bgHover

                                    Text {
                                        anchors.fill: parent
                                        anchors.leftMargin: Theme.sp2
                                        anchors.rightMargin: Theme.sp2
                                        verticalAlignment: Text.AlignVCenter
                                        text: isNull ? qsTr("NULL")
                                                     : (display === undefined || display === null
                                                        ? ""
                                                        : display)
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeCaption
                                        font.italic: isNull
                                        color: isNull ? Theme.textTertiary : Theme.textPrimary
                                        elide: Text.ElideRight
                                    }
                                }
                            }

                            Column {
                                anchors.centerIn: parent
                                spacing: Theme.sp1
                                visible: client.resultColumnCount === 0

                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: root.selectedTable.length > 0
                                          ? qsTr("Preview prepared for %1").arg(root.selectedTable)
                                          : qsTr("No result set yet")
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeBody
                                    font.weight: Theme.weightMedium
                                    color: Theme.textPrimary
                                }

                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: qsTr("Use the schema list to draft a query, then run it here.")
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeCaption
                                    color: Theme.textTertiary
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
