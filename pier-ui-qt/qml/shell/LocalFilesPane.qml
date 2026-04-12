import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Effects
import QtQuick.Layouts
import Qt.labs.folderlistmodel
import Pier
import "../components"

// Local file browser for the left panel. Reworked toward the original
// Pier file pane: clear header, breadcrumb path, search, and a compact
// multi-column file list for fast scanning.
Item {
    id: root

    signal markdownRequested(string filePath)
    signal openTerminalRequested(string path)
    signal contextPathChanged(string path)

    readonly property string homePath: StandardPaths.writableLocation(StandardPaths.HomeLocation)
    readonly property int modifiedColumnWidth: 56
    readonly property int kindColumnWidth: 50
    readonly property int sizeColumnWidth: 58
    readonly property bool showModifiedColumn: width >= 214
    readonly property bool showSizeColumn: width >= 288
    readonly property bool showKindColumn: width >= 356
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
        root.contextPathChanged(root.currentPath)
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
            implicitHeight: 32
            color: "transparent"
            border.width: 0

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp1
                anchors.rightMargin: Theme.sp1
                spacing: Theme.sp1

                IconButton {
                    compact: true
                    icon: "arrow-left"
                    tooltip: qsTr("Up")
                    enabled: root.currentPath.length > 1
                    onClicked: root._goUp()
                }

                Rectangle {
                    Layout.preferredWidth: 14
                    Layout.preferredHeight: 14
                    color: "transparent"

                    Image {
                        anchors.centerIn: parent
                        source: "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                        sourceSize: Qt.size(13, 13)
                        layer.enabled: true
                        layer.effect: MultiEffect {
                            colorizationColor: Theme.accent
                            colorization: 1.0
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    text: root.currentFolderName
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeBody
                    font.weight: Theme.weightMedium
                    color: Theme.textPrimary
                    elide: Text.ElideRight
                }

                IconButton {
                    id: placesButton
                    compact: true
                    glyph: "\u22ef"
                    tooltip: qsTr("Places")
                    onClicked: {
                        const pos = placesButton.mapToItem(root, 0, placesButton.height + Theme.sp1)
                        placesMenu.x = Math.max(Theme.sp2,
                                                Math.min(root.width - placesMenu.width - Theme.sp2, pos.x))
                        placesMenu.y = Math.max(Theme.sp2, pos.y)
                        placesMenu.open()
                    }
                }

                IconButton {
                    compact: true
                    icon: "refresh-cw"
                    tooltip: qsTr("Refresh")
                    onClicked: root._refreshFolder()
                }
            }

            PopoverPanel {
                id: placesMenu
                width: 208

                PierMenuItem {
                    text: qsTr("Home")
                    onClicked: {
                        placesMenu.close()
                        root.currentPath = root.homePath
                    }
                }

                PierMenuItem {
                    text: qsTr("Desktop")
                    onClicked: {
                        placesMenu.close()
                        root.currentPath = StandardPaths.writableLocation(StandardPaths.DesktopLocation)
                    }
                }

                PierMenuItem {
                    text: qsTr("Documents")
                    onClicked: {
                        placesMenu.close()
                        root.currentPath = StandardPaths.writableLocation(StandardPaths.DocumentsLocation)
                    }
                }

                PierMenuItem {
                    text: qsTr("Downloads")
                    onClicked: {
                        placesMenu.close()
                        root.currentPath = StandardPaths.writableLocation(StandardPaths.DownloadLocation)
                    }
                }

                Rectangle {
                    width: placesMenu.width - placesMenu.leftPadding - placesMenu.rightPadding
                    height: 1
                    color: Theme.borderSubtle
                }

                PierMenuItem {
                    text: qsTr("Choose folder…")
                    onClicked: {
                        placesMenu.close()
                        folderDialog.open()
                    }
                }

                Rectangle {
                    width: placesMenu.width - placesMenu.leftPadding - placesMenu.rightPadding
                    height: 1
                    color: Theme.borderSubtle
                }

                PierMenuItem {
                    text: qsTr("Open terminal here")
                    onClicked: {
                        placesMenu.close()
                        root.openTerminalRequested(root.currentPath)
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 24
            color: "transparent"
            border.width: 0

            Flickable {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
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
                                color: crumbArea.containsMouse ? Theme.bgHover : "transparent"
                                implicitHeight: 16
                                implicitWidth: crumbText.implicitWidth + Theme.sp1 * 2

                                Text {
                                    id: crumbText
                                    anchors.centerIn: parent
                                    text: modelData.name
                                    font.family: Theme.fontUi
                                    font.pixelSize: 10
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

        PierSearchField {
            id: searchInput
            Layout.fillWidth: true
            Layout.preferredHeight: 24
            text: root.searchQuery
            placeholder: qsTr("Search files…")
            clearable: true
            compact: true
            onTextChanged: root.searchQuery = text
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 22
            color: Theme.bgPanel

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp2
                anchors.rightMargin: Theme.sp2
                spacing: Theme.sp2

                Text {
                    Layout.fillWidth: true
                    text: qsTr("Name")
                    font.family: Theme.fontUi
                    font.pixelSize: 10
                    font.weight: Theme.weightMedium
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.modifiedColumnWidth
                    visible: root.showModifiedColumn
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Modified")
                    font.family: Theme.fontUi
                    font.pixelSize: 10
                    font.weight: Theme.weightMedium
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.sizeColumnWidth
                    visible: root.showSizeColumn
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Size")
                    font.family: Theme.fontUi
                    font.pixelSize: 10
                    font.weight: Theme.weightMedium
                    color: Theme.textTertiary
                }

                Text {
                    Layout.preferredWidth: root.kindColumnWidth
                    visible: root.showKindColumn
                    horizontalAlignment: Text.AlignRight
                    text: qsTr("Kind")
                    font.family: Theme.fontUi
                    font.pixelSize: 10
                    font.weight: Theme.weightMedium
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
                implicitHeight: matchesSearch ? 28 : 0
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
                    anchors.leftMargin: Theme.sp2
                    anchors.rightMargin: Theme.sp2
                    spacing: Theme.sp1_5

                    Rectangle {
                        Layout.preferredWidth: 13
                        Layout.preferredHeight: 13
                        color: "transparent"

                        Image {
                            anchors.centerIn: parent
                            source: fileRow.fileIsDir
                                    ? "qrc:/qt/qml/Pier/resources/icons/lucide/folder.svg"
                                    : "qrc:/qt/qml/Pier/resources/icons/lucide/file-text.svg"
                            sourceSize: Qt.size(12, 12)
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
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeSmall
                        font.weight: fileRow.fileIsDir ? Theme.weightMedium : Theme.weightRegular
                        color: Theme.textPrimary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.modifiedColumnWidth
                        visible: root.showModifiedColumn
                        horizontalAlignment: Text.AlignRight
                        text: root._formatModified(fileRow.fileModified)
                        font.family: Theme.fontUi
                        font.pixelSize: 10
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.sizeColumnWidth
                        visible: root.showSizeColumn
                        horizontalAlignment: Text.AlignRight
                        text: root._formatSize(fileRow.fileSize, fileRow.fileIsDir)
                        font.family: Theme.fontMono
                        font.pixelSize: 10
                        color: Theme.textTertiary
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.preferredWidth: root.kindColumnWidth
                        visible: root.showKindColumn
                        horizontalAlignment: Text.AlignRight
                        text: root._formatKind(fileRow.fileSuffix, fileRow.fileIsDir)
                        font.family: Theme.fontUi
                        font.pixelSize: 10
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
                height: 116
                color: Theme.bgInset
                radius: Theme.radiusMd
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

    PopoverPanel {
        id: fileContextMenu
        width: 208
        cornerRadius: Theme.radiusMd

        contentItem: Column {
            spacing: Theme.sp0_5

            PierMenuItem {
                text: root._contextOpenLabel()
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    root._openFileItem(root.contextFilePath,
                                       root.contextFileIsDir,
                                       root.contextFilePreviewable)
                }
            }

            PierMenuItem {
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

            PierMenuItem {
                text: qsTr("Reveal in file manager")
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.revealPath(root.contextFilePath)
                }
            }

            PierMenuItem {
                text: qsTr("Copy path")
                enabled: root.contextFilePath.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.copyText(root.contextFilePath)
                }
            }

            PierMenuItem {
                text: qsTr("Copy name")
                enabled: root.contextFileName.length > 0
                onClicked: {
                    fileContextMenu.close()
                    PierLocalSystem.copyText(root.contextFileName)
                }
            }
        }
    }

}
