import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Pier
import "../components"

// SQLite inspector — open a local .db file, browse tables,
// view columns, and execute arbitrary SQL queries.
Rectangle {
    id: root

    color: Theme.bgCanvas

    PierSqliteClient { id: client }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // ── Header ──────────────────────────────────────
        ToolPanelSurface {
            Layout.fillWidth: true
            padding: Theme.sp2
            implicitHeight: headerRow.implicitHeight + Theme.sp2 * 2

            RowLayout {
                id: headerRow
                anchors.fill: parent
                spacing: Theme.sp2

                Text {
                    text: client.dbPath.length > 0
                          ? client.dbPath.split("/").pop()
                          : qsTr("SQLite")
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightSemibold
                    color: Theme.textPrimary
                    elide: Text.ElideMiddle
                    Layout.fillWidth: true
                }

                StatusPill {
                    text: client.status === PierSqliteClient.Ready
                          ? qsTr("Connected")
                          : (client.status === PierSqliteClient.Loading
                             ? qsTr("Opening…")
                             : qsTr("No database"))
                    tone: client.status === PierSqliteClient.Ready ? "success" : "neutral"
                }

                GhostButton {
                    compact: true
                    minimumWidth: 0
                    text: qsTr("Open…")
                    onClicked: fileDialog.open()
                }
            }
        }

        // ── Main content ────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp2

            // Left: table list + columns
            ToolPanelSurface {
                Layout.preferredWidth: 200
                Layout.fillHeight: true
                padding: Theme.sp1

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp1

                    ToolSectionHeader {
                        Layout.fillWidth: true
                        title: qsTr("Tables")
                        subtitle: client.tables.length > 0
                                  ? qsTr("%1 tables").arg(client.tables.length)
                                  : qsTr("Open a database file")
                    }

                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: client.tables

                        delegate: Rectangle {
                            required property string modelData
                            required property int index
                            width: ListView.view.width
                            height: 28
                            color: tableMouse.containsMouse ? Theme.bgHover : "transparent"
                            radius: Theme.radiusSm

                            MouseArea {
                                id: tableMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    client.loadColumns(modelData)
                                    client.execute("SELECT * FROM \"" + modelData + "\" LIMIT 200")
                                }
                            }

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                anchors.rightMargin: Theme.sp2
                                spacing: Theme.sp1

                                Text {
                                    text: modelData
                                    font.family: Theme.fontMono
                                    font.pixelSize: Theme.sizeSmall
                                    color: Theme.textPrimary
                                    elide: Text.ElideRight
                                    Layout.fillWidth: true
                                }
                            }
                        }
                    }

                    // Column info
                    ToolSectionHeader {
                        Layout.fillWidth: true
                        visible: client.columns.length > 0
                        title: qsTr("Columns")
                    }

                    ListView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.min(contentHeight, 150)
                        clip: true
                        visible: client.columns.length > 0
                        model: client.columns

                        delegate: RowLayout {
                            required property var modelData
                            width: ListView.view.width
                            height: 22
                            spacing: Theme.sp1

                            Rectangle {
                                width: 6; height: 6; radius: 3
                                color: modelData.primary_key ? Theme.accent : Theme.textTertiary
                                Layout.leftMargin: Theme.sp2
                            }

                            Text {
                                text: modelData.name
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textPrimary
                                Layout.fillWidth: true
                            }

                            Text {
                                text: modelData.col_type
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                                Layout.rightMargin: Theme.sp2
                            }
                        }
                    }
                }
            }

            // Right: SQL editor + results
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.sp2

                // SQL editor
                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 100
                    padding: Theme.sp2

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: Theme.sp1

                        PierTextArea {
                            id: sqlEditor
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            mono: true
                            inset: true
                            placeholderText: qsTr("Enter SQL query…")
                            font.pixelSize: Theme.sizeSmall
                            wrapMode: TextEdit.NoWrap

                            Keys.onReturnPressed: (event) => {
                                if (event.modifiers & Qt.ControlModifier) {
                                    client.execute(sqlEditor.text)
                                    event.accepted = true
                                }
                            }
                        }

                        RowLayout {
                            spacing: Theme.sp1

                            PrimaryButton {
                                text: qsTr("Execute")
                                enabled: sqlEditor.text.length > 0
                                         && client.status === PierSqliteClient.Ready
                                         && !client.busy
                                onClicked: client.execute(sqlEditor.text)
                            }

                            Text {
                                visible: client.lastElapsedMs > 0
                                text: qsTr("%1 ms").arg(client.lastElapsedMs)
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }

                            Text {
                                visible: client.lastError.length > 0
                                text: client.lastError
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.statusError
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            Item { Layout.fillWidth: true }

                            Text {
                                visible: client.resultRows.length > 0
                                text: qsTr("%1 rows").arg(client.resultRows.length)
                                font.family: Theme.fontMono
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }
                        }
                    }
                }

                // Results table
                ToolPanelSurface {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    padding: Theme.sp1

                    ColumnLayout {
                        anchors.fill: parent
                        spacing: 0

                        // Column headers
                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 24
                            color: Theme.bgHover
                            visible: client.resultColumns.length > 0

                            Row {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp2
                                spacing: 0

                                Repeater {
                                    model: client.resultColumns
                                    Text {
                                        width: Math.max(80, (parent.width - Theme.sp2) / Math.max(1, client.resultColumns.length))
                                        height: 24
                                        text: modelData
                                        font.family: Theme.fontMono
                                        font.pixelSize: Theme.sizeSmall
                                        font.weight: Theme.weightSemibold
                                        color: Theme.textSecondary
                                        elide: Text.ElideRight
                                        verticalAlignment: Text.AlignVCenter
                                        leftPadding: Theme.sp1
                                    }
                                }
                            }
                        }

                        // Data rows
                        ListView {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: client.resultRows

                            delegate: Rectangle {
                                required property var modelData
                                required property int index
                                width: ListView.view.width
                                height: 24
                                color: index % 2 === 0 ? "transparent" : Theme.bgHover

                                Row {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.sp2
                                    spacing: 0

                                    Repeater {
                                        model: modelData
                                        Text {
                                            width: Math.max(80, (parent.width - Theme.sp2) / Math.max(1, client.resultColumns.length))
                                            height: 24
                                            text: modelData || ""
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.sizeSmall
                                            color: Theme.textPrimary
                                            elide: Text.ElideRight
                                            verticalAlignment: Text.AlignVCenter
                                            leftPadding: Theme.sp1
                                        }
                                    }
                                }
                            }
                        }

                        // Empty state
                        ToolEmptyState {
                            Layout.alignment: Qt.AlignCenter
                            visible: client.resultColumns.length === 0 && client.status === PierSqliteClient.Ready
                            icon: "database"
                            title: qsTr("Ready")
                            description: qsTr("Click a table or enter a SQL query to view results.")
                        }

                        ToolEmptyState {
                            Layout.alignment: Qt.AlignCenter
                            visible: client.status !== PierSqliteClient.Ready
                            icon: "database"
                            title: qsTr("No Database Open")
                            description: qsTr("Click 'Open…' to select a .db or .sqlite file.")
                        }
                    }
                }
            }
        }
    }

    FileDialog {
        id: fileDialog
        title: qsTr("Open SQLite database")
        nameFilters: [
            qsTr("SQLite files (*.db *.sqlite *.sqlite3 *.s3db)"),
            qsTr("All files (*)")
        ]
        fileMode: FileDialog.OpenFile
        onAccepted: {
            var path = selectedFile.toString().replace(/^file:\/\//, "")
            client.open(path)
        }
    }
}
