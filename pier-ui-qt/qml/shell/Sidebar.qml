import QtQuick
import QtQuick.Effects
import QtQuick.Layouts
import QtQuick.Window
import Pier
import "../components"

Rectangle {
    id: root

    property var connectionsModel: null
    property int selectedSection: 0
    property string serverSearch: ""
    property int connectionsRevision: 0
    property string groupDialogMode: ""
    property string groupDialogText: ""
    property string groupDialogTarget: ""
    property int draggingConnectionIndex: -1
    property var collapsedGroups: ({})
    clip: true

    signal addConnectionRequested
    signal connectionActivated(int index)
    signal connectionDeleted(int index)
    signal connectionSftpRequested(int index)
    signal connectionDuplicated(int index)
    signal openLocalTerminalRequested(string path)
    signal openMarkdownRequested(string filePath)
    signal fileContextChanged(string path)

    readonly property bool hasSavedGroups: connectionsModel
                                          && connectionsModel.groups
                                          && connectionsModel.groups.length > 0
    readonly property bool hasSavedConnections: connectionsModel && connectionsModel.count > 0
    readonly property bool hasServerContent: hasSavedConnections || hasSavedGroups
    readonly property var serverSections: root._buildServerSections()

    function _refreshConnectionsRevision() {
        root.connectionsRevision += 1
    }

    function _normalizedGroupName(name) {
        return String(name || "").trim()
    }

    function _primaryGroup(tags) {
        if (!tags || typeof tags.length !== "number")
            return ""
        for (let i = 0; i < tags.length; ++i) {
            const value = root._normalizedGroupName(tags[i])
            if (value.length > 0)
                return value
        }
        return ""
    }

    function _matchConnection(conn) {
        const query = String(root.serverSearch || "").trim().toLowerCase()
        if (query.length === 0)
            return true
        const haystack = [
            conn.name || "",
            conn.username || "",
            conn.host || "",
            root._primaryGroup(conn.tags || [])
        ].join(" ").toLowerCase()
        return haystack.indexOf(query) >= 0
    }

    function _setGroupCollapsed(groupKey, collapsed) {
        const nextState = Object.assign({}, root.collapsedGroups)
        nextState[groupKey] = collapsed
        root.collapsedGroups = nextState
    }

    function _isGroupCollapsed(groupKey) {
        return !!root.collapsedGroups[groupKey]
    }

    function _openGroupDialog(mode, groupName) {
        root.groupDialogMode = mode
        root.groupDialogTarget = groupName || ""
        root.groupDialogText = groupName || ""
        groupDialog.open = true
        Qt.callLater(() => groupNameField.forceActiveFocus())
    }

    function _submitGroupDialog() {
        const value = root._normalizedGroupName(root.groupDialogText)
        if (value.length === 0 || !root.connectionsModel)
            return
        if (root.groupDialogMode === "rename")
            root.connectionsModel.renameGroup(root.groupDialogTarget, value)
        else
            root.connectionsModel.createGroup(value)
        groupDialog.open = false
    }

    function _moveConnectionToGroup(index, groupKey) {
        if (!root.connectionsModel || index < 0)
            return
        if (String(groupKey || "").length > 0)
            root.connectionsModel.setPrimaryTag(index, groupKey)
        else
            root.connectionsModel.clearPrimaryTag(index)
        root.draggingConnectionIndex = -1
    }

    function _buildServerSections() {
        root.connectionsRevision
        const sections = []
        if (!root.connectionsModel)
            return sections

        const knownGroups = []
        if (root.connectionsModel.groups) {
            for (let i = 0; i < root.connectionsModel.groups.length; ++i) {
                const groupName = root._normalizedGroupName(root.connectionsModel.groups[i])
                if (groupName.length > 0 && knownGroups.indexOf(groupName) < 0)
                    knownGroups.push(groupName)
            }
        }
        knownGroups.sort(function(a, b) { return a.localeCompare(b) })

        const grouped = {}
        for (let i = 0; i < knownGroups.length; ++i)
            grouped[knownGroups[i]] = []

        const ungrouped = []
        for (let i = 0; i < root.connectionsModel.count; ++i) {
            const conn = root.connectionsModel.get(i)
            if (!conn || !root._matchConnection(conn))
                continue
            const groupName = root._primaryGroup(conn.tags || [])
            const item = {
                index: i,
                name: conn.name || "",
                username: conn.username || "",
                host: conn.host || "",
                port: conn.port || 22,
                keyPath: conn.keyPath || "",
                usesAgent: conn.usesAgent === true,
                tags: conn.tags || []
            }
            if (groupName.length > 0) {
                if (!grouped[groupName])
                    grouped[groupName] = []
                grouped[groupName].push(item)
            } else {
                ungrouped.push(item)
            }
        }

        const hasNamedGroups = Object.keys(grouped).length > 0
        const searching = String(root.serverSearch || "").trim().length > 0

        const names = Object.keys(grouped).sort(function(a, b) { return a.localeCompare(b) })
        for (let i = 0; i < names.length; ++i) {
            const key = names[i]
            const items = grouped[key]
            if (searching && items.length === 0)
                continue
            sections.push({
                key: key,
                title: key,
                ungrouped: false,
                showHeader: true,
                items: items
            })
        }

        if (ungrouped.length > 0 || !hasNamedGroups) {
            sections.push({
                key: "",
                title: hasNamedGroups ? qsTr("Ungrouped") : qsTr("Connections"),
                ungrouped: true,
                showHeader: hasNamedGroups || searching,
                items: ungrouped
            })
        }

        return sections
    }

    Connections {
        target: root.connectionsModel
        function onCountChanged() { root._refreshConnectionsRevision() }
        function onGroupsChanged() { root._refreshConnectionsRevision() }
        function onDataChanged() { root._refreshConnectionsRevision() }
        function onModelReset() { root._refreshConnectionsRevision() }
        function onRowsInserted() { root._refreshConnectionsRevision() }
        function onRowsRemoved() { root._refreshConnectionsRevision() }
    }

    implicitWidth: Theme.sidebarWidth
    color: Theme.bgPanel

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp2
        spacing: Theme.sp2

        Item {
            Layout.fillWidth: true
            implicitHeight: 34

            Rectangle {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(parent.width - Theme.sp1 * 2, tabsRow.implicitWidth + Theme.sp1)
                height: 30
                radius: Theme.radiusMd
                color: Theme.bgInset
                border.color: Theme.borderSubtle
                border.width: 1

                RowLayout {
                    id: tabsRow
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp0_5
                    anchors.rightMargin: Theme.sp0_5
                    spacing: Theme.sp0_5

                    SidebarTabButton {
                        title: qsTr("Files")
                        icon: "folder"
                        active: root.selectedSection === 0
                        onClicked: root.selectedSection = 0
                    }

                    SidebarTabButton {
                        title: qsTr("Servers")
                        icon: "server"
                        active: root.selectedSection === 1
                        onClicked: root.selectedSection = 1
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.borderSubtle
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: root.selectedSection

            LocalFilesPane {
                Layout.fillWidth: true
                Layout.fillHeight: true
                onContextPathChanged: (path) => root.fileContextChanged(path)
                onMarkdownRequested: (filePath) => root.openMarkdownRequested(filePath)
                onOpenTerminalRequested: (path) => root.openLocalTerminalRequested(path)
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: Theme.sp2

                    Item {
                        Layout.fillWidth: true
                        implicitHeight: 28

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Theme.sp1
                            anchors.rightMargin: Theme.sp1
                            spacing: Theme.sp1

                            Image {
                                Layout.alignment: Qt.AlignVCenter
                                source: "qrc:/qt/qml/Pier/resources/icons/lucide/server.svg"
                                sourceSize: Qt.size(Theme.iconXs, Theme.iconXs)
                                layer.enabled: true
                                layer.effect: MultiEffect {
                                    colorization: 1.0
                                    colorizationColor: Theme.accent
                                }
                            }

                            Text {
                                text: qsTr("Saved Connections")
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                font.weight: Theme.weightMedium
                                color: Theme.textPrimary
                                elide: Text.ElideRight
                            }

                            Text {
                                text: qsTr("%1 ready").arg(root.connectionsModel ? root.connectionsModel.count : 0)
                                font.family: Theme.fontUi
                                font.pixelSize: 10
                                color: Theme.textTertiary
                            }

                            Item { Layout.fillWidth: true }

                            IconButton {
                                compact: true
                                icon: "folder"
                                tooltip: qsTr("New group")
                                onClicked: root._openGroupDialog("create", "")
                            }

                            IconButton {
                                compact: true
                                icon: "plus"
                                tooltip: qsTr("Add connection")
                                onClicked: root.addConnectionRequested()
                            }
                        }
                    }

                    PierSearchField {
                        Layout.fillWidth: true
                        text: root.serverSearch
                        placeholder: qsTr("Search servers")
                        clearable: true
                        compact: true
                        onTextChanged: root.serverSearch = text
                    }

                    Item {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true

                        Text {
                            anchors.centerIn: parent
                            visible: !root.hasServerContent
                            text: qsTr("No saved connections yet.")
                            horizontalAlignment: Text.AlignHCenter
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            color: Theme.textTertiary
                        }

                        PierScrollView {
                            anchors.fill: parent
                            anchors.rightMargin: Theme.sp0_5
                            visible: root.hasServerContent

                            contentWidth: availableWidth

                            Column {
                                width: Math.max(0, parent.availableWidth - Theme.sp0_5)
                                spacing: Theme.sp2

                                Repeater {
                                    model: root.serverSections

                                    delegate: Column {
                                        required property var modelData
                                        width: parent.width
                                        spacing: Theme.sp0_5

                                        ServerGroupHeader {
                                            width: parent.width
                                            visible: modelData.showHeader
                                            title: modelData.title
                                            groupKey: modelData.key
                                            itemCount: modelData.items.length
                                            ungrouped: modelData.ungrouped
                                            expanded: !root._isGroupCollapsed(modelData.key)
                                            onToggleRequested: root._setGroupCollapsed(modelData.key, !expanded)
                                            onRenameRequested: root._openGroupDialog("rename", modelData.key)
                                            onDeleteRequested: root.connectionsModel.deleteGroup(modelData.key)
                                            onDropped: (sourceIndex) => root._moveConnectionToGroup(sourceIndex, modelData.key)
                                        }

                                        Rectangle {
                                            width: parent.width
                                            height: 1
                                            color: Theme.borderSubtle
                                            visible: modelData.showHeader
                                        }

                                        Column {
                                            width: parent.width
                                            spacing: Theme.sp0_5
                                            visible: !root._isGroupCollapsed(modelData.key)

                                            Repeater {
                                                model: modelData.items

                                                delegate: ServerConnectionRow {
                                                    required property var modelData
                                                    width: parent.width
                                                    itemIndex: modelData.index
                                                    name: modelData.name
                                                    username: modelData.username
                                                    host: modelData.host
                                                    port: modelData.port
                                                    keyPath: modelData.keyPath
                                                    usesAgent: modelData.usesAgent
                                                }
                                            }

                                            Text {
                                                width: parent.width
                                                visible: modelData.items.length === 0
                                                leftPadding: Theme.sp3
                                                text: qsTr("Drop connections here")
                                                font.family: Theme.fontUi
                                                font.pixelSize: 10
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
        }
    }

    PopoverPanel {
        id: contextMenu

        property int targetIndex: -1

        width: 176
        panelPadding: Theme.sp1
        itemSpacing: Theme.sp0_5

        Column {
            id: ctxCol
            spacing: Theme.sp0_5

            Repeater {
                model: [
                    { label: qsTr("Connect"), action: "connect" },
                    { label: qsTr("SFTP"), action: "sftp" },
                    { label: qsTr("Duplicate"), action: "duplicate" },
                    { label: qsTr("Ungroup"), action: "ungroup" },
                    { label: qsTr("Delete"), action: "delete" }
                ]

                delegate: PierMenuItem {
                    width: contextMenu.width - contextMenu.leftPadding - contextMenu.rightPadding
                    text: modelData.label
                    destructive: modelData.action === "delete"
                    onClicked: {
                        const idx = contextMenu.targetIndex
                        contextMenu.close()
                        if (modelData.action === "connect")
                            root.connectionActivated(idx)
                        else if (modelData.action === "sftp")
                            root.connectionSftpRequested(idx)
                        else if (modelData.action === "duplicate")
                            root.connectionDuplicated(idx)
                        else if (modelData.action === "ungroup")
                            root.connectionsModel.clearPrimaryTag(idx)
                        else if (modelData.action === "delete")
                            root.connectionDeleted(idx)
                    }
                }
            }
        }
    }

    ModalDialogShell {
        id: groupDialog
        parent: root.Window.window ? root.Window.window.contentItem : root
        anchors.fill: parent
        open: false
        title: root.groupDialogMode === "rename" ? qsTr("Rename Group") : qsTr("New Group")
        subtitle: root.groupDialogMode === "rename"
                  ? qsTr("Update the section name for these saved connections.")
                  : qsTr("Create a persistent server group for organizing saved connections.")
        dialogWidth: 420
        dialogHeight: 280
        bodyPadding: 0
        onRequestClose: open = false

        body: Item {
            anchors.fill: parent

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.sp5
                spacing: Theme.sp3

                Card {
                    id: groupCard
                    Layout.fillWidth: true
                    inset: true
                    padding: Theme.sp4
                    implicitHeight: groupCardColumn.implicitHeight + padding * 2

                    ColumnLayout {
                        id: groupCardColumn
                        anchors.fill: parent
                        spacing: Theme.sp2

                        Text {
                            text: qsTr("Group Name")
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeBody
                            font.weight: Theme.weightMedium
                            color: Theme.textPrimary
                        }

                        PierTextField {
                            id: groupNameField
                            Layout.fillWidth: true
                            text: root.groupDialogText
                            placeholder: qsTr("Production")
                            onTextChanged: root.groupDialogText = text
                        }

                        Text {
                            Layout.fillWidth: true
                            text: qsTr("Connections dropped onto this group will inherit the tag automatically.")
                            wrapMode: Text.WordWrap
                            font.family: Theme.fontUi
                            font.pixelSize: Theme.sizeSmall
                            color: Theme.textTertiary
                        }
                    }
                }
            }
        }

        footer: Item {
            implicitHeight: footerRow.implicitHeight

            RowLayout {
                id: footerRow
                width: parent.width
                spacing: Theme.sp2

                Item { Layout.fillWidth: true }

                GhostButton {
                    text: qsTr("Cancel")
                    onClicked: groupDialog.open = false
                }

                PrimaryButton {
                    text: root.groupDialogMode === "rename" ? qsTr("Rename") : qsTr("Create")
                    onClicked: root._submitGroupDialog()
                }
            }
        }
    }

    Rectangle {
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        width: 1
        color: Theme.borderSubtle
    }

    component SidebarTabButton: Rectangle {
        property string title: ""
        property string icon: ""
        property bool active: false
        signal clicked

        implicitHeight: 25
        implicitWidth: row.implicitWidth + Theme.sp2 * 2
        radius: Theme.radiusSm
        color: active ? Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, Theme.dark ? 0.14 : 0.11)
                      : tabMouse.containsMouse ? Theme.bgHover : "transparent"
        border.width: 0

        RowLayout {
            id: row
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp2
            spacing: Theme.sp1

            Item {
                Layout.alignment: Qt.AlignVCenter
                Layout.preferredWidth: 12
                Layout.preferredHeight: 12

                Image {
                    anchors.centerIn: parent
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + icon + ".svg"
                    sourceSize: Qt.size(12, 12)
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        colorization: 1.0
                        colorizationColor: active ? Theme.accent : Theme.textSecondary
                    }
                }
            }

            Text {
                Layout.alignment: Qt.AlignVCenter
                text: title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: active ? Theme.weightMedium : Theme.weightRegular
                color: active ? Theme.accent : Theme.textSecondary
                wrapMode: Text.NoWrap
                elide: Text.ElideRight
            }
        }

        MouseArea {
            id: tabMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: parent.clicked()
        }
    }

    component ServerGroupHeader: Rectangle {
        property string title: ""
        property string groupKey: ""
        property int itemCount: 0
        property bool expanded: true
        property bool ungrouped: false
        signal toggleRequested()
        signal renameRequested()
        signal deleteRequested()
        signal dropped(int sourceIndex)

        implicitHeight: 22
        radius: Theme.radiusSm
        readonly property bool hovered: groupHover.hovered
        color: dropArea.containsDrag ? Theme.accentSubtle : hovered ? Theme.bgHover : "transparent"

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp1
            anchors.rightMargin: Theme.sp1
            spacing: Theme.sp1

            Item {
                Layout.preferredWidth: 11
                Layout.preferredHeight: 11
                visible: itemCount > 0 || !ungrouped

                Image {
                    anchors.centerIn: parent
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/chevron-down.svg"
                    sourceSize: Qt.size(11, 11)
                    rotation: expanded ? 0 : -90
                    transformOrigin: Item.Center
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        colorization: 1.0
                        colorizationColor: Theme.textTertiary
                    }
                    Behavior on rotation { NumberAnimation { duration: Theme.durFast } }
                }
            }

            Image {
                source: "qrc:/qt/qml/Pier/resources/icons/lucide/" + (ungrouped ? "server" : "folder") + ".svg"
                sourceSize: Qt.size(11, 11)
                layer.enabled: true
                layer.effect: MultiEffect {
                    colorization: 1.0
                    colorizationColor: Theme.textSecondary
                }
            }

            Text {
                text: title
                font.family: Theme.fontUi
                font.pixelSize: Theme.sizeCaption
                font.weight: Theme.weightMedium
                color: Theme.textPrimary
                elide: Text.ElideRight
            }

            Text {
                text: "(" + itemCount + ")"
                font.family: Theme.fontUi
                font.pixelSize: 10
                color: Theme.textTertiary
            }

            Item { Layout.fillWidth: true }

            IconButton {
                compact: true
                glyph: "\u22ef"
                visible: !ungrouped
                opacity: hovered ? 1.0 : 0.0
                enabled: opacity > 0.01
                tooltip: qsTr("Group actions")
                onClicked: {
                    const pos = mapToItem(root, 0, height + Theme.sp1)
                    groupContextMenu.targetGroup = groupKey
                    groupContextMenu.x = Math.max(Theme.sp2,
                                                  Math.min(root.width - groupContextMenu.width - Theme.sp2, pos.x))
                    groupContextMenu.y = Math.max(Theme.sp2, pos.y)
                    groupContextMenu.open()
                }
                Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
            }
        }

        DropArea {
            id: dropArea
            anchors.fill: parent
            enabled: root.draggingConnectionIndex >= 0
            onDropped: root.dropped(root.draggingConnectionIndex)
        }

        HoverHandler {
            id: groupHover
            cursorShape: Qt.PointingHandCursor
        }

        TapHandler {
            acceptedButtons: Qt.LeftButton
            gesturePolicy: TapHandler.ReleaseWithinBounds
            onTapped: parent.toggleRequested()
        }
    }

    component ServerConnectionRow: Rectangle {
        required property int itemIndex
        required property string name
        required property string username
        required property string host
        required property int port
        required property string keyPath
        required property bool usesAgent

        readonly property string authLabel: usesAgent
                                            ? qsTr("Agent")
                                            : keyPath.length > 0 ? qsTr("Key") : qsTr("Password")
        readonly property color authTint: usesAgent
                                          ? Theme.accent
                                          : keyPath.length > 0 ? Theme.statusSuccess : Theme.statusWarning

        readonly property bool hovered: rowHover.hovered
        implicitHeight: 34
        radius: Theme.radiusMd
        color: hovered || dragHandler.active ? Theme.bgHover : "transparent"

        Behavior on color { ColorAnimation { duration: Theme.durFast } }

        Drag.active: dragHandler.active
        Drag.source: this
        Drag.hotSpot.x: width / 2
        Drag.hotSpot.y: height / 2

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp2
            anchors.rightMargin: Theme.sp1
            spacing: Theme.sp1_5

            Rectangle {
                width: 14
                height: 14
                radius: Theme.radiusSm
                color: Qt.rgba(authTint.r, authTint.g, authTint.b, Theme.dark ? 0.12 : 0.08)
                border.color: Qt.rgba(authTint.r, authTint.g, authTint.b, Theme.dark ? 0.20 : 0.14)
                border.width: 1
                Layout.alignment: Qt.AlignVCenter

                Rectangle {
                    anchors.centerIn: parent
                    width: 4
                    height: 4
                    radius: 2
                    color: authTint
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: 0

                Text {
                    text: name
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                Text {
                    text: username + "@" + host + ":" + port
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    font.family: Theme.fontMono
                    font.pixelSize: Theme.sizeSmall
                    color: Theme.textTertiary
                    elide: Text.ElideMiddle
                }
            }

            Rectangle {
                implicitHeight: 15
                implicitWidth: authText.implicitWidth + Theme.sp1 * 2
                radius: Theme.radiusPill
                color: Qt.rgba(authTint.r, authTint.g, authTint.b, Theme.dark ? 0.12 : 0.08)

                Text {
                    id: authText
                    anchors.centerIn: parent
                    text: authLabel
                    font.family: Theme.fontUi
                    font.pixelSize: 8
                    font.weight: Theme.weightMedium
                    color: authTint
                }
            }

            Item {
                Layout.preferredWidth: 48
                Layout.preferredHeight: 24

                RowLayout {
                    anchors.centerIn: parent
                    spacing: Theme.sp0_5

                    IconButton {
                        compact: true
                        icon: "terminal"
                        tooltip: qsTr("Open SSH session")
                        opacity: hovered ? 1.0 : 0.0
                        enabled: opacity > 0.01
                        onClicked: root.connectionActivated(itemIndex)
                        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
                    }

                    IconButton {
                        compact: true
                        icon: "x"
                        iconSize: Theme.iconXs
                        tooltip: qsTr("Delete")
                        opacity: hovered ? 1.0 : 0.0
                        enabled: opacity > 0.01
                        onClicked: root.connectionDeleted(itemIndex)
                        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
                    }
                }
            }
        }

        DragHandler {
            id: dragHandler
            target: null
            onActiveChanged: {
                if (active)
                    root.draggingConnectionIndex = itemIndex
                else if (root.draggingConnectionIndex === itemIndex)
                    root.draggingConnectionIndex = -1
            }
        }

        HoverHandler {
            id: rowHover
            cursorShape: Qt.PointingHandCursor
        }

        TapHandler {
            acceptedButtons: Qt.LeftButton
            gesturePolicy: TapHandler.ReleaseWithinBounds
            onTapped: {
                if (!dragHandler.active)
                    root.connectionActivated(itemIndex)
            }
        }

        TapHandler {
            acceptedButtons: Qt.RightButton
            gesturePolicy: TapHandler.ReleaseWithinBounds
            onTapped: (eventPoint, button) => {
                const pos = parent.mapToItem(root, eventPoint.position.x, eventPoint.position.y + parent.height)
                contextMenu.targetIndex = itemIndex
                contextMenu.x = Math.max(Theme.sp2,
                                         Math.min(pos.x, root.width - contextMenu.width - Theme.sp2))
                contextMenu.y = Math.max(Theme.sp2,
                                         Math.min(pos.y, root.height - contextMenu.implicitHeight - Theme.sp2))
                contextMenu.open()
            }
        }
    }

    PopoverPanel {
        id: groupContextMenu

        property string targetGroup: ""

        width: 176
        panelPadding: Theme.sp1
        itemSpacing: Theme.sp0_5

        contentItem: Column {
            spacing: Theme.sp0_5

            PierMenuItem {
                text: qsTr("Rename group")
                onClicked: {
                    const groupName = groupContextMenu.targetGroup
                    groupContextMenu.close()
                    root._openGroupDialog("rename", groupName)
                }
            }

            PierMenuItem {
                text: qsTr("Ungroup connections")
                destructive: true
                onClicked: {
                    root.connectionsModel.deleteGroup(groupContextMenu.targetGroup)
                    groupContextMenu.close()
                }
            }
        }
    }
}
