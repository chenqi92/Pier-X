import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Effects
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Pier

// Local file browser for the left panel. Reworked toward the original
// Pier file pane: clear header, breadcrumb path, search, and a compact
// multi-column file list for fast scanning.
Item {
    id: root

    signal markdownRequested(string filePath)
    signal openTerminalRequested(string path)

    readonly property string homePath: StandardPaths.writableLocation(StandardPaths.HomeLocation)
    readonly property int modifiedColumnWidth: 56
    readonly property int kindColumnWidth: 54
    readonly property int sizeColumnWidth: 58
    property string currentPath: homePath
    property string searchQuery: ""
    property string folderUrl: root._toFolderUrl(root.currentPath)
    property string selectedPath: ""
    property string contextFilePath: ""
    property string contextFileName: ""
    property bool contextFileIsDir: false
    property bool contextFilePreviewable: false
    readonly property string currentFolderName: {
        const normalized = root._normalizePath(root.currentPath)
        if (!normalized || normalized === "/")
            return qsTr("Files")
        const parts = normalized.split(/[\\\/]+/).filter(function(part) { return part.length > 0 })
        return parts.length > 0 ? parts[parts.length - 1] : qsTr("Files")
    }

    onCurrentPathChanged: {
        searchQuery = ""
        folderUrl = root._toFolderUrl(root.currentPath)
    }

    function _refreshFolder() {
        root.folderUrl = ""
        Qt.callLater(function() {
            root.folderUrl = root._toFolderUrl(root.currentPath)
        })
    }

    function _toFolderUrl(path) {
        if (!path || path.length === 0)
            return ""
        if (String(path).indexOf("file://") === 0)
            return String(path)
        if (Qt.platform.os === "windows")
            return "file:///" + String(path).replace(/\\/g, "/")
        return "file://" + path
    }

    function _normalizePath(path) {
        var value = String(path || "")
        if (value.indexOf("file://") === 0)
            value = value.replace(/^file:\/\//, "")
        return decodeURIComponent(value)
    }

    function _displayPath(path) {
        var normalized = _normalizePath(path)
        if (normalized.indexOf(root.homePath) === 0)
            return "~" + normalized.slice(root.homePath.length)
        return normalized
    }

    function _goUp() {
        var trimmed = root.currentPath.replace(/[\\\/]+$/, "")
        if (trimmed.length === 0 || trimmed === "/" || /^[A-Za-z]:$/.test(trimmed))
            return
        var slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
        if (slash <= 0) {
            if (Qt.platform.os === "windows" && trimmed.length >= 2)
                root.currentPath = trimmed.slice(0, 2)
            else
                root.currentPath = "/"
            return
        }
        root.currentPath = trimmed.slice(0, slash)
    }

    function _isPreviewable(path) {
        var lower = String(path || "").toLowerCase()
        return lower.endsWith(".md")
                || lower.endsWith(".markdown")
                || lower.endsWith(".mdx")
                || lower.endsWith(".txt")
                || lower.endsWith(".log")
    }

    function _directoryForPath(path, isDir) {
        const normalized = root._normalizePath(path)
        if (isDir)
            return normalized
        const slash = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"))
        if (slash <= 0) {
            if (Qt.platform.os === "windows" && normalized.length >= 2)
                return normalized.slice(0, 2)
            return "/"
        }
        return normalized.slice(0, slash)
    }

    function _openFileItem(path, isDir, isPreviewable) {
        if (isDir) {
            root.currentPath = path
            return
        }
        if (isPreviewable) {
            root.markdownRequested(path)
            return
        }
        PierLocalSystem.openPath(path)
    }

    function _openContextMenu(path, name, isDir, isPreviewable, item, x, y) {
        root.contextFilePath = path
        root.contextFileName = name
        root.contextFileIsDir = isDir
        root.contextFilePreviewable = isPreviewable
        const pos = item.mapToItem(root, x, y)
        fileContextMenu.x = Math.max(Theme.sp2,
                                     Math.min(root.width - fileContextMenu.width - Theme.sp2, pos.x))
        fileContextMenu.y = Math.max(Theme.sp2,
                                     Math.min(root.height - fileContextMenu.implicitHeight - Theme.sp2, pos.y))
        fileContextMenu.open()
    }

    function _contextOpenLabel() {
        if (root.contextFileIsDir)
            return qsTr("Open folder")
        if (root.contextFilePreviewable)
            return qsTr("Open preview")
        return qsTr("Open with default app")
    }

    function _matchesQuery(name) {
        const query = String(root.searchQuery || "").trim().toLowerCase()
        if (query.length === 0)
            return true
        return String(name || "").toLowerCase().indexOf(query) >= 0
    }

    function _pathSegments(path) {
        const normalized = root._normalizePath(path)
        if (normalized.length === 0)
            return []

        const segments = []
        const home = root._normalizePath(root.homePath)
        if (normalized.indexOf(home) === 0) {
            segments.push({ name: "~", path: home })
            const relative = normalized.slice(home.length)
            const parts = relative.split(/[\\\/]+/).filter(function(part) { return part.length > 0 })
            var accumulated = home
            for (var i = 0; i < parts.length; ++i) {
                accumulated += "/" + parts[i]
                segments.push({ name: parts[i], path: accumulated })
            }
            return segments
        }

        if (normalized === "/")
            return [{ name: "/", path: "/" }]

        segments.push({ name: "/", path: "/" })
        const absoluteParts = normalized.split(/[\\\/]+/).filter(function(part) { return part.length > 0 })
        var full = ""
        for (var j = 0; j < absoluteParts.length; ++j) {
            full += "/" + absoluteParts[j]
            segments.push({ name: absoluteParts[j], path: full })
        }
        return segments
    }

    function _formatModified(dateValue) {
        if (!dateValue)
            return ""
        return Qt.formatDateTime(dateValue, "MM-dd")
    }

    function _formatKind(suffixValue, isDir) {
        if (isDir)
            return qsTr("Folder")
        const suffix = String(suffixValue || "").trim()
        if (suffix.length === 0)
            return qsTr("File")
        return suffix.length > 4 ? suffix.slice(0, 4).toUpperCase() : suffix.toUpperCase()
    }

    function _formatSize(bytesValue, isDir) {
        if (isDir)
            return "--"
        const size = Number(bytesValue || 0)
        if (size < 1024)
            return size + " B"
        if (size < 1024 * 1024)
            return (size / 1024).toFixed(size < 10 * 1024 ? 1 : 0) + " KB"
        if (size < 1024 * 1024 * 1024)
            return (size / (1024 * 1024)).toFixed(size < 10 * 1024 * 1024 ? 1 : 0) + " MB"
        return (size / (1024 * 1024 * 1024)).toFixed(1) + " GB"
    }

    FolderListModel {
        id: folderModel
        folder: root.folderUrl
        showDirs: true
        showFiles: true
        showHidden: false
        showDotAndDotDot: false
        showDirsFirst: true
        sortField: FolderListModel.Name
        sortReversed: false
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: headerBar
            Layout.fillWidth: true
            implicitHeight: 52
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1
            radius: Theme.radiusMd

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                IconButton {
                    icon: "arrow-left"
                    tooltip: qsTr("Up")
                    enabled: root.currentPath.length > 1
                    onClicked: root._goUp()
                }

                Rectangle {
                    Layout.preferredWidth: 18
                    Layout.preferredHeight: 18
                    radius: Theme.radiusSm
                    color: Theme.accentSubtle

                    Image {
                        anchors.centerIn: parent
                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                        sourceSize: Qt.size(14, 14)
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorizationColor: Theme.accent
                            colorization: 1.0
                        }
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0

                    Text {
                        Layout.fillWidth: true
                        text: root.currentFolderName
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        font.weight: Theme.weightSemibold
                        color: Theme.textPrimary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root._displayPath(root.currentPath) + "  ·  " + qsTr("%1 items").arg(folderModel.count)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideMiddle
                    }
                }

                GhostButton {
                    id: placesButton
                    text: qsTr("Places")
                    onClicked: placesMenu.open()
                }

                IconButton {
                    icon: "refresh-cw"
                    tooltip: qsTr("Refresh")
                    onClicked: root._refreshFolder()
                }

                IconButton {
                    icon: "terminal"
                    tooltip: qsTr("Terminal")
                    onClicked: root.openTerminalRequested(root.currentPath)
                }
            }

            Menu {
                id: placesMenu
                x: placesButton.x
                y: headerBar.height - 1

                MenuItem {
                    text: qsTr("Home")
                    onTriggered: root.currentPath = root.homePath
                }

                MenuItem {
                    text: qsTr("Desktop")
                    onTriggered: root.currentPath = StandardPaths.writableLocation(StandardPaths.DesktopLocation)
                }

                MenuItem {
                    text: qsTr("Documents")
                    onTriggered: root.currentPath = StandardPaths.writableLocation(StandardPaths.DocumentsLocation)
                }

                MenuItem {
                    text: qsTr("Downloads")
                    onTriggered: root.currentPath = StandardPaths.writableLocation(StandardPaths.DownloadLocation)
                }

                MenuSeparator {}

                MenuItem {
                    text: qsTr("Choose folder…")
                    onTriggered: folderDialog.open()
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 32
            color: Theme.bgInset
            border.color: Theme.borderSubtle
            border.width: 1
            border.pixelAligned: true
            radius: Theme.radiusMd

            Flickable {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                contentWidth: breadcrumbRow.width
                contentHeight: height
                clip: true
                flickableDirection: Flickable.HorizontalFlick
                boundsBehavior: Flickable.StopAtBounds

                Row {
                    id: breadcrumbRow
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Theme.sp1

                    Repeater {
                        id: breadcrumbRepeater
                        model: root._pathSegments(root.currentPath)

                        delegate: Row {
                            required property var modelData
                            required property int index

                            spacing: Theme.sp1

                            Text {
                                visible: index > 0
                                text: "›"
                                anchors.verticalCenter: parent.verticalCenter
                                font.family: Theme.fontUi
                                font.pixelSize: Theme.sizeSmall
                                color: Theme.textTertiary
                            }

                            Rectangle {
                                radius: Theme.radiusSm
                                color: crumbArea.containsMouse ? Theme.bgHover : index === breadcrumbRepeater.count - 1 ? Theme.bgSurface : "transparent"
                                implicitHeight: 20
                                implicitWidth: crumbText.implicitWidth + Theme.sp2 * 2

                                Text {
                                    id: crumbText
                                    anchors.centerIn: parent
                                    text: modelData.name
                                    font.family: Theme.fontUi
                                    font.pixelSize: Theme.sizeSmall
                                    color: index === breadcrumbRepeater.count - 1
                                           ? Theme.textPrimary
                                           : Theme.accent
                                }

                                MouseArea {
                                    id: crumbArea
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: root.currentPath = modelData.path
                                }
                            }
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 38
            color: Theme.bgSurface
            border.color: Theme.borderSubtle
            border.width: 1
            border.pixelAligned: true
            radius: Theme.radiusMd

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                spacing: Theme.sp2

                Image {
                    source: "qrc:/qt/qml/Pier/resources/icons/lucide/search.svg"
                    sourceSize: Qt.size(14, 14)
                    Layout.alignment: Qt.AlignVCenter
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        colorizationColor: Theme.textTertiary
                        colorization: 1.0
                    }
                }

                TextInput {
                    id: searchInput
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    text: root.searchQuery
                    onTextChanged: root.searchQuery = text
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    color: Theme.textPrimary
                    clip: true
                    selectByMouse: true
                    selectionColor: Theme.accentMuted
                    selectedTextColor: Theme.textPrimary

                    Text {
                        anchors.fill: parent
                        verticalAlignment: Text.AlignVCenter
                        text: qsTr("Search files…")
                        visible: searchInput.text.length === 0
                        font: searchInput.font
                        color: Theme.textTertiary
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 26
            color: Theme.bgPanel

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp3
                anchors.rightMargin: Theme.sp3
                spacing: Theme.sp2

                Text {
                    Layout.fillWidth: true
                    text: qsTr("Name").toUpperCase()
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    font.weight: Theme.weightSemibold
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.modifiedColumnWidth
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Modified").toUpperCase()
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    font.weight: Theme.weightSemibold
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.kindColumnWidth
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Kind").toUpperCase()
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    font.weight: Theme.weightSemibold
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.sizeColumnWidth
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Size").toUpperCase()
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeSmall
                    font.weight: Theme.weightSemibold
                    color: Theme.textTertiary
                }
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: 1
                color: Theme.borderSubtle
            }
        }

        ListView {
            id: filesView
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 0
            model: folderModel

            delegate: Rectangle {
                id: fileRow

                required property string fileName
                required property string filePath
                required property bool fileIsDir
                required property string fileSuffix
                required property date fileModified
                required property var fileSize

                readonly property bool matchesSearch: root._matchesQuery(fileName)
                readonly property bool selected: root.selectedPath === filePath

                width: ListView.view.width
                implicitHeight: matchesSearch ? Theme.listRowHeight : 0
                height: implicitHeight
                visible: matchesSearch
                color: selected ? Theme.bgSelected : fileMouse.containsMouse ? Theme.bgHover : "transparent"
                radius: Theme.radiusSm

                Behavior on color { ColorAnimation { duration: Theme.durFast } }

                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 2
                    radius: 1
                    color: Theme.accent
                    visible: fileRow.selected
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp3
                    anchors.rightMargin: Theme.sp3
                    spacing: Theme.sp2

                    Rectangle {
                        Layout.preferredWidth: 18
                        Layout.preferredHeight: 18
                        radius: Theme.radiusSm
                        color: fileRow.fileIsDir ? Theme.accentSubtle : Theme.bgInset

                        Image {
                            anchors.centerIn: parent
                            source: fileRow.fileIsDir
                                    ? "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                                    : "qrc:/qt/qml/Pier/resources/icons/lucide/file-text.svg"
                            sourceSize: Qt.size(14, 14)
                            layer.enabled: true
                            layer.effect: MultiEffect {
                                colorizationColor: fileRow.fileIsDir ? Theme.accent : Theme.textTertiary
                                colorization: 1.0
                            }
                        }
                    }

                    Text {
                        Layout.fillWidth: true
                        text: fileRow.fileName
                        font.family: fileRow.fileIsDir ? Theme.fontUi : Theme.fontMono
                        font.pixelSize: Theme.sizeBody
                        font.weight: fileRow.fileIsDir ? Theme.weightMedium : Theme.weightRegular
                        color: Theme.textPrimary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.modifiedColumnWidth
                        horizontalAlignment: Text.AlignRight
                        text: root._formatModified(fileRow.fileModified)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.kindColumnWidth
                        horizontalAlignment: Text.AlignRight
                        text: root._formatKind(fileRow.fileSuffix, fileRow.fileIsDir)
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.sizeColumnWidth
                        horizontalAlignment: Text.AlignRight
                        text: root._formatSize(fileRow.fileSize, fileRow.fileIsDir)
                        font.family: Theme.fontMono
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }
                }

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: Theme.borderSubtle
                    visible: fileRow.matchesSearch
                }

                MouseArea {
                    id: fileMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        if (mouse.button === Qt.RightButton)
                            return
                        root.selectedPath = fileRow.filePath
                        if (fileRow.fileIsDir) {
                            root.currentPath = fileRow.filePath
                        } else if (root._isPreviewable(fileRow.filePath)) {
                            root.markdownRequested(fileRow.filePath)
                        } else {
                            PierLocalSystem.openPath(fileRow.filePath)
                        }
                    }
                    onPressed: (mouse) => {
                        if (mouse.button !== Qt.RightButton)
                            return
                        root.selectedPath = fileRow.filePath
                        root._openContextMenu(fileRow.filePath,
                                              fileRow.fileName,
                                              fileRow.fileIsDir,
                                              root._isPreviewable(fileRow.filePath),
                                              fileRow,
                                              mouse.x,
                                              mouse.y)
                        mouse.accepted = true
                    }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp6, 300)
                height: 132
                color: Theme.bgInset
                radius: Theme.radiusLg
                border.color: Theme.borderSubtle
                border.width: 1
                visible: folderModel.count === 0

                Column {
                    anchors.centerIn: parent
                    width: parent.width - Theme.sp6 * 2
                    spacing: Theme.sp1

                    Text {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("This folder is empty.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textSecondary
                    }

                    Text {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("Choose another directory or open a local terminal.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp6, 300)
                height: 132
                color: Theme.bgInset
                radius: Theme.radiusLg
                border.color: Theme.borderSubtle
                border.width: 1
                visible: folderModel.count > 0
                         && root.searchQuery.trim().length > 0
                         && filesView.contentHeight === 0

                Column {
                    anchors.centerIn: parent
                    width: parent.width - Theme.sp6 * 2
                    spacing: Theme.sp1

                    Text {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("No matching files.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeBody
                        color: Theme.textSecondary
                    }

                    Text {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: qsTr("Try a different keyword or clear the search.")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        color: Theme.textTertiary
                    }
                }
            }
        }
    }

    FolderDialog {
        id: folderDialog
        title: qsTr("Choose folder…")
        onAccepted: root.currentPath = root._normalizePath(selectedFolder.toString())
    }

    Popup {
        id: fileContextMenu
        width: 208
        modal: false
        focus: true
        padding: Theme.sp1
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusMd

            Behavior on color { ColorAnimation { duration: Theme.durNormal } }
            Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }
        }

        contentItem: Column {
            spacing: Theme.sp0_5

            FileMenuItem {
                text: root._contextOpenLabel()
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    root._openFileItem(root.contextFilePath,
                                       root.contextFileIsDir,
                                       root.contextFilePreviewable)
                }
            }

            FileMenuItem {
                text: qsTr("Open in terminal here")
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    root.openTerminalRequested(root._directoryForPath(root.contextFilePath,
                                                                      root.contextFileIsDir))
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                color: Theme.borderSubtle
            }

            FileMenuItem {
                text: qsTr("Reveal in file manager")
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.revealPath(root.contextFilePath)
                }
            }

            FileMenuItem {
                text: qsTr("Copy path")
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.copyText(root.contextFilePath)
                }
            }

            FileMenuItem {
                text: qsTr("Copy name")
                enabled: root.contextFileName.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.copyText(root.contextFileName)
                }
            }
        }
    }

    component FileMenuItem: Rectangle {
        id: menuItem

        property string text: ""
        signal clicked()

        implicitWidth: 208
        implicitHeight: 28
        radius: Theme.radiusSm
        color: menuArea.containsMouse ? Theme.bgHover : "transparent"
        opacity: enabled ? 1.0 : 0.48

        Behavior on color { ColorAnimation { duration: Theme.durFast } }
        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        Text {
            anchors.fill: parent
            anchors.leftMargin: Theme.sp3
            anchors.rightMargin: Theme.sp3
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
            text: menuItem.text
            font.family: Theme.fontUi
            font.pixelSize: Theme.sizeBody
            color: Theme.textSecondary
        }

        MouseArea {
            id: menuArea
            anchors.fill: parent
            hoverEnabled: true
            enabled: menuItem.enabled
            cursorShape: menuItem.enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: menuItem.clicked()
        }
    }
}
