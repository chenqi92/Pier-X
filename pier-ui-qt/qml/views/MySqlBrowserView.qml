import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pier
import "../components"

// MySQL browser panel — tunnel-first service tool with a
// persistent workspace. The panel is designed for repeated
// operations: saved profiles, favorite SQL, schema browsing,
// and quick table actions all live in one place.
Rectangle {
    id: root

    clip: true
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
    property string formCredentialId: ""

    property string selectedDatabase: mysqlDatabase
    property string selectedTable: ""
    property string selectedColumn: ""
    property string sqlText: mysqlDatabase.length > 0
                             ? ("USE `" + mysqlDatabase + "`;\nSHOW TABLES;")
                             : "SELECT NOW() AS now;"

    property int selectedProfileIndex: -1
    property int selectedFavoriteIndex: -1
    property string profileDraftName: ""
    property string favoriteDraftName: ""
    property string workspaceNotice: ""
    property string workspaceNoticeKind: "info"
    property string databaseFilter: ""
    property string tableFilter: ""
    property string columnFilter: ""

    readonly property bool hasSavedCredential: formCredentialId.length > 0 && formPassword.length === 0
    readonly property bool hasResult: client.lastError.length > 0
                                      || client.resultColumnCount > 0
                                      || client.resultRowCount > 0
                                      || client.lastAffectedRows > 0
                                      || client.lastElapsedMs > 0
    readonly property string bannerText: client.lastError.length > 0
                                         ? client.lastError
                                         : (client.errorMessage.length > 0
                                            ? client.errorMessage
                                            : workspaceNotice)
    readonly property string bannerKind: client.lastError.length > 0
                                         || client.errorMessage.length > 0
                                         ? "error"
                                         : workspaceNoticeKind
    readonly property var filteredDatabases: root._filterStrings(client.databases, root.databaseFilter)
    readonly property var filteredTables: root._filterStrings(client.tables, root.tableFilter)
    readonly property var filteredColumns: root._filterColumns(client.columns, root.columnFilter)

    color: Theme.bgCanvas
    focus: true
    activeFocusOnTab: true

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    PierMySqlWorkspace {
        id: workspace
    }

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

        onTablesChanged: {
            if (root.selectedTable.length > 0 && tables.indexOf(root.selectedTable) < 0) {
                root.selectedTable = ""
                root.selectedColumn = ""
                refreshColumns("", "")
            }
        }
    }

    Component.onCompleted: {
        root.profileDraftName = root._suggestProfileName()
        root.favoriteDraftName = root._suggestFavoriteName()
        if (root.mysqlHost.length > 0
            && root.mysqlPort > 0
            && root.mysqlUser.length > 0) {
            root._connect()
        }
    }

    function _portValue() {
        var parsed = parseInt(root.formPortText, 10)
        return isNaN(parsed) ? 0 : parsed
    }

    function _setNotice(message, kind) {
        root.workspaceNotice = message || ""
        root.workspaceNoticeKind = kind || "info"
    }

    function _clearNotice() {
        root.workspaceNotice = ""
        root.workspaceNoticeKind = "info"
    }

    function _suggestProfileName() {
        var port = _portValue()
        var label = (root.formUser.length > 0 ? root.formUser : "sql")
                    + "@"
                    + (root.formHost.length > 0 ? root.formHost : "127.0.0.1")
                    + ":"
                    + (port > 0 ? String(port) : "3306")
        if (root.formDatabase.trim().length > 0)
            label += "/" + root.formDatabase.trim()
        return label
    }

    function _suggestFavoriteName() {
        if (root.selectedTable.length > 0)
            return "Preview " + root.selectedTable
        if (root.selectedDatabase.length > 0)
            return root.selectedDatabase + " query"
        return "Query"
    }

    function _applySql(text) {
        root.sqlText = text
        sqlEditor.text = text
    }

    function _normalizedNeedle(text) {
        return (text || "").trim().toLowerCase()
    }

    function _filterStrings(values, query) {
        var needle = root._normalizedNeedle(query)
        var out = []
        for (var i = 0; i < values.length; ++i) {
            var value = String(values[i] || "")
            if (needle.length === 0 || value.toLowerCase().indexOf(needle) >= 0)
                out.push(value)
        }
        return out
    }

    function _columnSearchText(column) {
        var parts = [
            column.name || "",
            column.type || "",
            column.key || "",
            column.extra || ""
        ]
        if (column.defaultValue !== null
                && column.defaultValue !== undefined
                && String(column.defaultValue).length > 0)
            parts.push(String(column.defaultValue))
        return parts.join(" ").toLowerCase()
    }

    function _filterColumns(values, query) {
        var needle = root._normalizedNeedle(query)
        var out = []
        for (var i = 0; i < values.length; ++i) {
            var column = values[i]
            if (needle.length === 0 || root._columnSearchText(column).indexOf(needle) >= 0)
                out.push(column)
        }
        return out
    }

    function _columnReference(columnName) {
        if (columnName.length === 0)
            return ""
        if (root.selectedTable.length > 0)
            return "`" + root.selectedTable + "`.`" + columnName + "`"
        return "`" + columnName + "`"
    }

    function _insertSqlSnippet(snippet) {
        if (!snippet || snippet.length === 0)
            return
        sqlEditor.forceActiveFocus()
        var cursor = sqlEditor.cursorPosition
        sqlEditor.insert(cursor, snippet)
        sqlEditor.cursorPosition = cursor + snippet.length
        root.sqlText = sqlEditor.text
    }

    function _connect() {
        var port = _portValue()
        if (root.formHost.length === 0 || root.formUser.length === 0 || port <= 0) {
            root._setNotice(qsTr("Host, port, and user are required."), "error")
            return
        }
        root._clearNotice()
        client.stop()
        root.selectedTable = ""
        root.selectedColumn = ""
        root.selectedDatabase = root.formDatabase
        if (root.formPassword.length > 0) {
            client.connectTo(root.formHost, port,
                             root.formUser, root.formPassword,
                             root.formDatabase)
        } else if (root.formCredentialId.length > 0) {
            client.connectToWithCredential(root.formHost, port,
                                           root.formUser, root.formCredentialId,
                                           root.formDatabase)
        } else {
            client.connectTo(root.formHost, port,
                             root.formUser, "",
                             root.formDatabase)
        }
    }

    function _refreshSchema() {
        client.refreshDatabases()
        if (root.selectedDatabase.length > 0) {
            client.refreshTables(root.selectedDatabase)
        }
        if (root.selectedDatabase.length > 0 && root.selectedTable.length > 0) {
            client.refreshColumns(root.selectedDatabase, root.selectedTable)
        }
    }

    function _applyProfile(index) {
        var profile = workspace.profileAt(index)
        if (!profile.name)
            return
        root.selectedProfileIndex = index
        root.profileDraftName = profile.name
        root.formHost = profile.host || "127.0.0.1"
        root.formPortText = String(profile.port || 3306)
        root.formUser = profile.user || ""
        root.formDatabase = profile.database || ""
        root.formCredentialId = profile.credentialId || ""
        root.formPassword = ""
        root.selectedDatabase = root.formDatabase
        root.selectedTable = ""
        root.selectedColumn = ""
        root._setNotice(qsTr("Profile %1 applied").arg(profile.name), "info")
    }

    function _saveProfile() {
        var name = root.profileDraftName.trim().length > 0
                   ? root.profileDraftName.trim()
                   : root._suggestProfileName()
        var existingName = root.selectedProfileIndex >= 0
                           ? (workspace.profileAt(root.selectedProfileIndex).name || "")
                           : ""
        var credentialId = root.formCredentialId

        if (root.formPassword.length > 0) {
            if (credentialId.length === 0 || (existingName.length > 0 && existingName !== name)) {
                credentialId = PierCredentials.freshId()
            }
            if (!PierCredentials.setEntry(credentialId, root.formPassword)) {
                root._setNotice(qsTr("Failed to save password to keychain."), "error")
                return
            }
        } else if (existingName.length > 0 && existingName !== name) {
            // Avoid silently sharing a hidden secret across two
            // different profile names when the user duplicated a
            // profile without re-entering the password.
            credentialId = ""
        }

        if (!workspace.upsertProfile(name,
                                     root.formHost,
                                     root._portValue(),
                                     root.formUser,
                                     root.formDatabase,
                                     credentialId)) {
            root._setNotice(qsTr("Failed to save profile."), "error")
            return
        }

        root.formCredentialId = credentialId
        root.profileDraftName = name
        root.selectedProfileIndex = workspace.indexOfProfile(name)
        root._setNotice(qsTr("Profile %1 saved").arg(name), "success")
    }

    function _removeProfile() {
        if (root.selectedProfileIndex < 0)
            return

        var profile = workspace.profileAt(root.selectedProfileIndex)
        var credentialId = profile.credentialId || ""
        var shouldDeleteCredential = credentialId.length > 0
                                     && !workspace.credentialReferencedElsewhere(credentialId, root.selectedProfileIndex)
        if (!workspace.removeProfile(root.selectedProfileIndex)) {
            root._setNotice(qsTr("Failed to remove profile."), "error")
            return
        }
        if (shouldDeleteCredential) {
            PierCredentials.deleteEntry(credentialId)
        }

        root.selectedProfileIndex = -1
        root.profileDraftName = root._suggestProfileName()
        if (root.formCredentialId === credentialId) {
            root.formCredentialId = ""
        }
        root._setNotice(qsTr("Profile removed"), "info")
    }

    function _applyFavorite(index) {
        var favorite = workspace.favoriteAt(index)
        if (!favorite.name)
            return

        root.selectedFavoriteIndex = index
        root.favoriteDraftName = favorite.name
        if (favorite.database && favorite.database.length > 0) {
            root.selectedDatabase = favorite.database
            root.formDatabase = favorite.database
            if (client.status === PierMySqlClient.Connected) {
                client.refreshTables(root.selectedDatabase)
            }
        }
        root._applySql(favorite.sql || "")
        root._setNotice(qsTr("Favorite %1 loaded").arg(favorite.name), "info")
    }

    function _saveFavorite() {
        var name = root.favoriteDraftName.trim().length > 0
                   ? root.favoriteDraftName.trim()
                   : root._suggestFavoriteName()
        if (!workspace.upsertFavorite(name,
                                      root.sqlText,
                                      root.selectedDatabase.length > 0
                                      ? root.selectedDatabase
                                      : root.formDatabase)) {
            root._setNotice(qsTr("Failed to save favorite query."), "error")
            return
        }
        root.favoriteDraftName = name
        root.selectedFavoriteIndex = workspace.indexOfFavorite(name)
        root._setNotice(qsTr("Favorite %1 saved").arg(name), "success")
    }

    function _removeFavorite() {
        if (root.selectedFavoriteIndex < 0)
            return
        if (!workspace.removeFavorite(root.selectedFavoriteIndex)) {
            root._setNotice(qsTr("Failed to remove favorite query."), "error")
            return
        }
        root.selectedFavoriteIndex = -1
        root.favoriteDraftName = root._suggestFavoriteName()
        root._setNotice(qsTr("Favorite removed"), "info")
    }

    function _previewTable(tableName) {
        if (tableName.length === 0)
            return
        root.selectedTable = tableName
        root.selectedColumn = ""
        if (root.selectedDatabase.length > 0) {
            client.refreshColumns(root.selectedDatabase, tableName)
        }
        root._applySql("SELECT * FROM `" + tableName + "` LIMIT 200;")
        root.favoriteDraftName = "Preview " + tableName
    }

    function _showCountFor(tableName) {
        if (tableName.length === 0)
            return
        root._applySql("SELECT COUNT(*) AS total FROM `" + tableName + "`;")
    }

    function _showDescribeFor(tableName) {
        if (tableName.length === 0 || root.selectedDatabase.length === 0)
            return
        root._applySql("SHOW COLUMNS FROM `" + root.selectedDatabase + "`.`" + tableName + "`;")
    }

    function _showCreateFor(tableName) {
        if (tableName.length === 0)
            return
        root._applySql("SHOW CREATE TABLE `" + tableName + "`;")
    }

    function _selectColumn(columnName) {
        root.selectedColumn = columnName
    }

    function _selectOnlyColumn(columnName) {
        if (columnName.length === 0 || root.selectedTable.length === 0)
            return
        root._applySql("SELECT " + root._columnReference(columnName)
                       + " FROM `" + root.selectedTable + "` LIMIT 200;")
    }

    function _insertSelectedColumn() {
        if (root.selectedColumn.length === 0)
            return
        root._insertSqlSnippet(root._columnReference(root.selectedColumn))
    }

    function _insertFilterForColumn() {
        if (root.selectedColumn.length === 0)
            return
        root._insertSqlSnippet(root._columnReference(root.selectedColumn) + " = ")
    }

    function _insertOrderForColumn() {
        if (root.selectedColumn.length === 0)
            return
        root._insertSqlSnippet("ORDER BY " + root._columnReference(root.selectedColumn) + " DESC\n")
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        ToolPanelSurface {
            Layout.fillWidth: true
            padding: Theme.sp2
            implicitHeight: connectFlow.implicitHeight + Theme.sp2 * 2

            Flow {
                id: connectFlow
                anchors.fill: parent
                spacing: Theme.sp2

                Rectangle {
                    implicitWidth: 176
                    implicitHeight: 26
                    color: Theme.bgInset
                    border.color: Theme.borderSubtle
                    border.width: 1
                    radius: Theme.radiusPill

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
                    implicitWidth: 152
                    placeholder: qsTr("Host")
                    text: root.formHost
                    onTextChanged: root.formHost = text
                }

                PierTextField {
                    implicitWidth: 84
                    placeholder: qsTr("Port")
                    text: root.formPortText
                    onTextChanged: root.formPortText = text
                }

                PierTextField {
                    implicitWidth: 132
                    placeholder: qsTr("User")
                    text: root.formUser
                    onTextChanged: root.formUser = text
                }

                PierTextField {
                    implicitWidth: 150
                    placeholder: root.hasSavedCredential
                                 ? qsTr("Password (saved in keychain)")
                                 : qsTr("Password")
                    password: true
                    text: root.formPassword
                    onTextChanged: root.formPassword = text
                }

                PierTextField {
                    implicitWidth: 162
                    placeholder: qsTr("Default DB")
                    text: root.formDatabase
                    onTextChanged: root.formDatabase = text
                }

                Rectangle {
                    visible: root.hasSavedCredential
                    implicitWidth: credentialText.implicitWidth + Theme.sp3 * 2
                    implicitHeight: 24
                    radius: Theme.radiusPill
                    color: Theme.bgSurface
                    border.color: Theme.borderSubtle
                    border.width: 1

                    Text {
                        id: credentialText
                        anchors.centerIn: parent
                        text: qsTr("Keychain")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }
                }

                PrimaryButton {
                    text: client.status === PierMySqlClient.Connected
                          ? qsTr("Reconnect")
                          : qsTr("Connect")
                    enabled: !client.busy
                    onClicked: root._connect()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Refresh")
                    enabled: client.status === PierMySqlClient.Connected && !client.busy
                    onClicked: root._refreshSchema()
                }
            }
        }

        ToolPanelSurface {
            Layout.fillWidth: true
            implicitHeight: profileFlow.implicitHeight + Theme.sp2 * 2
            padding: Theme.sp2

            Flow {
                id: profileFlow
                anchors.fill: parent
                spacing: Theme.sp2

                Text {
                    text: qsTr("Profile")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }

                PierTextField {
                    implicitWidth: 160
                    placeholder: qsTr("Profile name")
                    text: root.profileDraftName
                    onTextChanged: root.profileDraftName = text
                }

                PierComboBox {
                    implicitWidth: 180
                    options: workspace.profileNames
                    currentIndex: root.selectedProfileIndex
                    placeholder: qsTr("Saved profiles")
                    onActivated: (index) => root._applyProfile(index)
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Save")
                    onClicked: root._saveProfile()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Delete")
                    enabled: root.selectedProfileIndex >= 0
                    onClicked: root._removeProfile()
                }

                Rectangle {
                    width: 1
                    height: 18
                    color: Theme.borderSubtle
                }

                Text {
                    text: qsTr("Favorite")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }

                PierTextField {
                    implicitWidth: 160
                    placeholder: qsTr("Favorite name")
                    text: root.favoriteDraftName
                    onTextChanged: root.favoriteDraftName = text
                }

                PierComboBox {
                    implicitWidth: 180
                    options: workspace.favoriteNames
                    currentIndex: root.selectedFavoriteIndex
                    placeholder: qsTr("Saved queries")
                    onActivated: (index) => root._applyFavorite(index)
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Save")
                    onClicked: root._saveFavorite()
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Delete")
                    enabled: root.selectedFavoriteIndex >= 0
                    onClicked: root._removeFavorite()
                }
            }
        }

        ToolBanner {
            Layout.fillWidth: true
            text: bannerText
            tone: bannerKind === "error"
                  ? "error"
                  : (bannerKind === "success" ? "success" : "neutral")
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp2

            ToolPanelSurface {
                Layout.preferredWidth: 336
                Layout.minimumWidth: 300
                Layout.fillHeight: true
                padding: Theme.sp2

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp2

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Text {
                            text: qsTr("Schema")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeCaption
                            font.weight: Theme.weightMedium
                            color: Theme.textSecondary
                        }

                        Item { Layout.fillWidth: true }

                        GhostButton {
                            compact: true
                            minimumWidth: 0
                            text: qsTr("USE %1").arg(root.selectedDatabase)
                            visible: root.selectedDatabase.length > 0
                            enabled: client.status === PierMySqlClient.Connected
                            onClicked: root._applySql("USE `" + root.selectedDatabase + "`;\n")
                        }
                    }

                    Text {
                        text: qsTr("Databases")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        placeholder: qsTr("Filter databases")
                        text: root.databaseFilter
                        onTextChanged: root.databaseFilter = text
                    }

                    ListView {
                        id: databasesView
                        Layout.fillWidth: true
                        Layout.preferredHeight: 112
                        clip: true
                        spacing: 0
                        model: root.filteredDatabases

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
                                    root.selectedColumn = ""
                                    client.refreshTables(dbRow.modelData)
                                    client.refreshColumns("", "")
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

                    PierTextField {
                        Layout.fillWidth: true
                        placeholder: qsTr("Filter tables")
                        text: root.tableFilter
                        onTextChanged: root.tableFilter = text
                    }

                    ListView {
                        id: tablesView
                        Layout.fillWidth: true
                        Layout.preferredHeight: 140
                        clip: true
                        spacing: 0
                        model: root.filteredTables

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

                    Rectangle {
                        Layout.fillWidth: true
                        visible: root.selectedTable.length > 0
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusSm
                        implicitHeight: tableActions.implicitHeight + Theme.sp2 * 2

                        ColumnLayout {
                            id: tableActions
                            anchors.fill: parent
                            anchors.margins: Theme.sp2
                            spacing: Theme.sp2

                            Text {
                                text: root.selectedDatabase.length > 0
                                      ? (root.selectedDatabase + "." + root.selectedTable)
                                      : root.selectedTable
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textPrimary
                                elide: Text.ElideMiddle
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("SELECT 200")
                                    onClicked: root._previewTable(root.selectedTable)
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("COUNT(*)")
                                    onClicked: root._showCountFor(root.selectedTable)
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("DESCRIBE")
                                    onClicked: root._showDescribeFor(root.selectedTable)
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("SHOW CREATE")
                                    onClicked: root._showCreateFor(root.selectedTable)
                                }
                            }
                        }
                    }

                    Text {
                        text: qsTr("Columns")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }

                    PierTextField {
                        Layout.fillWidth: true
                        placeholder: qsTr("Filter columns")
                        text: root.columnFilter
                        onTextChanged: root.columnFilter = text
                    }

                    ListView {
                        id: columnsView
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: Theme.sp1
                        model: root.filteredColumns

                        delegate: Rectangle {
                            id: columnRow
                            required property var modelData

                            width: ListView.view.width
                            implicitHeight: 44
                            radius: Theme.radiusSm
                            color: root.selectedColumn === (modelData.name || "")
                                   ? Theme.accentSubtle
                                   : (columnMouse.containsMouse ? Theme.bgHover : "transparent")

                            Behavior on color { ColorAnimation { duration: Theme.durFast } }

                            Column {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                anchors.topMargin: Theme.sp1
                                anchors.bottomMargin: Theme.sp1
                                spacing: Theme.sp0_5

                                Row {
                                    width: parent.width
                                    spacing: Theme.sp1

                                    Text {
                                        text: modelData.name || ""
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeBody
                                        color: Theme.textPrimary
                                        width: parent.width - (keyBadge.visible ? keyBadge.width + Theme.sp1 : 0)
                                        elide: Text.ElideRight
                                    }

                                    Rectangle {
                                        id: keyBadge
                                        visible: (modelData.key || "").length > 0
                                        implicitWidth: badgeText.implicitWidth + Theme.sp2 * 2
                                        implicitHeight: 18
                                        radius: Theme.radiusPill
                                        color: Theme.accentSubtle
                                        border.color: Theme.borderSubtle
                                        border.width: 1

                                        Text {
                                            id: badgeText
                                            anchors.centerIn: parent
                                            text: modelData.key || ""
                                            font.family: Theme.fontUi
                                            font.pixelSize: Theme.sizeSmall
                                            font.weight: Theme.weightMedium
                                            color: Theme.accent
                                        }
                                    }
                                }

                                Text {
                                    width: parent.width
                                    text: {
                                        var parts = []
                                        if ((modelData.type || "").length > 0)
                                            parts.push(modelData.type)
                                        parts.push(modelData.nullable ? qsTr("nullable") : qsTr("not null"))
                                        if ((modelData.extra || "").length > 0)
                                            parts.push(modelData.extra)
                                        if (modelData.defaultValue !== null
                                                && modelData.defaultValue !== undefined
                                                && String(modelData.defaultValue).length > 0)
                                            parts.push(qsTr("default %1").arg(modelData.defaultValue))
                                        return parts.join(" · ")
                                    }
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textTertiary
                                    elide: Text.ElideRight
                                }
                            }

                            MouseArea {
                                id: columnMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: root._selectColumn(modelData.name || "")
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        visible: root.selectedColumn.length > 0
                        color: Theme.bgSurface
                        border.color: Theme.borderSubtle
                        border.width: 1
                        radius: Theme.radiusSm
                        implicitHeight: selectedColumnCard.implicitHeight + Theme.sp2 * 2

                        ColumnLayout {
                            id: selectedColumnCard
                            anchors.fill: parent
                            anchors.margins: Theme.sp2
                            spacing: Theme.sp2

                            Text {
                                text: qsTr("Column %1").arg(root.selectedColumn)
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeCaption
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            Text {
                                width: parent.width
                                text: {
                                    for (var i = 0; i < client.columns.length; ++i) {
                                        var column = client.columns[i]
                                        if ((column.name || "") !== root.selectedColumn)
                                            continue
                                        var details = []
                                        if ((column.type || "").length > 0)
                                            details.push(column.type)
                                        details.push(column.nullable ? qsTr("nullable") : qsTr("not null"))
                                        if ((column.key || "").length > 0)
                                            details.push(qsTr("key %1").arg(column.key))
                                        if ((column.extra || "").length > 0)
                                            details.push(column.extra)
                                        return details.join(" · ")
                                    }
                                    return ""
                                }
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                wrapMode: Text.WordWrap
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("Only This")
                                    enabled: root.selectedTable.length > 0
                                    onClicked: root._selectOnlyColumn(root.selectedColumn)
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("Insert")
                                    onClicked: root._insertSelectedColumn()
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("WHERE")
                                    onClicked: root._insertFilterForColumn()
                                }

                                GhostButton {
                                    compact: true
                                    minimumWidth: 0
                                    text: qsTr("ORDER BY")
                                    onClicked: root._insertOrderForColumn()
                                }
                            }
                        }
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.sp2

                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 224
                    padding: Theme.sp2

                    ColumnLayout {
                        anchors.fill: parent
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
                                compact: true
                                minimumWidth: 0
                                text: qsTr("USE %1").arg(root.selectedDatabase)
                                visible: root.selectedDatabase.length > 0
                                enabled: client.status === PierMySqlClient.Connected
                                onClicked: root._applySql("USE `" + root.selectedDatabase + "`;\n")
                            }

                            GhostButton {
                                compact: true
                                minimumWidth: 0
                                text: qsTr("SELECT 200")
                                visible: root.selectedTable.length > 0
                                enabled: client.status === PierMySqlClient.Connected
                                onClicked: root._previewTable(root.selectedTable)
                            }

                            GhostButton {
                                compact: true
                                minimumWidth: 0
                                text: qsTr("COUNT(*)")
                                visible: root.selectedTable.length > 0
                                enabled: client.status === PierMySqlClient.Connected
                                onClicked: root._showCountFor(root.selectedTable)
                            }

                            GhostButton {
                                compact: true
                                minimumWidth: 0
                                text: qsTr("SHOW CREATE")
                                visible: root.selectedTable.length > 0
                                enabled: client.status === PierMySqlClient.Connected
                                onClicked: root._showCreateFor(root.selectedTable)
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

                            ScrollView {
                                anchors.fill: parent
                                anchors.margins: Theme.sp1
                                clip: true

                                TextArea {
                                    id: sqlEditor
                                    text: root.sqlText
                                    wrapMode: TextArea.NoWrap
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeBody
                                    color: Theme.textPrimary
                                    selectionColor: Theme.accentMuted
                                    selectedTextColor: Theme.textPrimary
                                    background: Rectangle { color: "transparent" }
                                    selectByMouse: true
                                    onTextChanged: root.sqlText = text

                                    Keys.onPressed: (event) => {
                                        if ((event.modifiers & Qt.ControlModifier)
                                            && (event.key === Qt.Key_Return
                                                || event.key === Qt.Key_Enter)) {
                                            event.accepted = true
                                            client.execute(root.sqlText)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                ToolBanner {
                    Layout.fillWidth: true
                    tone: client.lastError.length > 0 ? "error" : "neutral"
                    text: client.lastError.length > 0
                          ? client.lastError
                          : (!root.hasResult
                             ? qsTr("Saved profiles and favorite queries persist across launches.")
                             : (client.resultColumnCount > 0
                                ? qsTr("%1 rows · %2 columns · %3 ms%4")
                                    .arg(client.resultRowCount)
                                    .arg(client.resultColumnCount)
                                    .arg(client.lastElapsedMs)
                                    .arg(client.lastTruncated ? qsTr(" · truncated") : "")
                                : qsTr("%1 rows affected · %2 ms")
                                    .arg(client.lastAffectedRows)
                                    .arg(client.lastElapsedMs)))
                }

                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    padding: Theme.sp1

                    ColumnLayout {
                        anchors.fill: parent
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

                            ToolEmptyState {
                                anchors.centerIn: parent
                                visible: client.resultColumnCount === 0
                                icon: "database"
                                title: root.selectedTable.length > 0
                                       ? qsTr("Schema ready for %1").arg(root.selectedTable)
                                       : qsTr("No result set yet")
                                description: root.selectedTable.length > 0
                                             ? qsTr("Use quick actions or a saved query to keep working.")
                                             : qsTr("Save a profile, browse a table, or load a favorite query to start.")
                            }
                        }
                    }
                }
            }
        }
    }
}
