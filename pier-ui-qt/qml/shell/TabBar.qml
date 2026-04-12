import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Pier

// Horizontal tab bar — TerminalTab repeater + new tab button.
// Supports drag-to-reorder: each tab has a DragHandler that allows
// the user to grab and drag it to a new position. A visual drop
// indicator shows where the tab will land.
Rectangle {
    id: root

    property var model: null
    property int currentIndex: 0
    readonly property bool hasOverflow: tabContainer.contentWidth > tabContainer.width + 1
    readonly property bool canScrollLeft: tabContainer.contentX > 0
    readonly property bool canScrollRight: tabContainer.contentX < tabContainer.contentWidth - tabContainer.width - 1
    signal tabClicked(int index)
    signal tabClosed(int index)
    signal closeOtherTabsRequested(int index)
    signal closeTabsToLeftRequested(int index)
    signal closeTabsToRightRequested(int index)
    signal newTabClicked
    signal tabMoved(int from, int to)

    property int contextTabIndex: -1

    implicitHeight: 32
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    function scrollTabs(delta) {
        const maxContentX = Math.max(0, tabContainer.contentWidth - tabContainer.width)
        tabContainer.contentX = Math.max(0, Math.min(maxContentX, tabContainer.contentX + delta))
    }

    function ensureTabVisible(index) {
        const tab = tabRepeater.itemAt(index)
        if (!tab)
            return
        const tabLeft = tab.x
        const tabRight = tabLeft + tab.width
        if (tabLeft < tabContainer.contentX) {
            tabContainer.contentX = tabLeft
        } else if (tabRight > tabContainer.contentX + tabContainer.width) {
            tabContainer.contentX = tabRight - tabContainer.width
        }
    }

    function openContextMenu(index, x, y) {
        contextTabIndex = index
        tabContextMenu.x = Math.max(Theme.sp2,
                                    Math.min(root.width - tabContextMenu.width - Theme.sp2,
                                             x))
        tabContextMenu.y = root.height - 1
        tabContextMenu.open()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp1
        anchors.rightMargin: Theme.sp1
        spacing: Theme.sp1

        IconButton {
            Layout.alignment: Qt.AlignVCenter
            visible: root.hasOverflow
            enabled: root.canScrollLeft
            icon: "arrow-left"
            tooltip: qsTr("Scroll tabs left")
            onClicked: root.scrollTabs(-Math.max(tabContainer.width * 0.72, 160))
        }

        // Tab row — Repeater in a Row so we can control ordering
        // and implement drag-to-reorder.
        Flickable {
            id: tabContainer
            Layout.fillHeight: true
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            clip: true
            contentWidth: tabRow.width
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.horizontal: ScrollBar { height: 0; active: false; visible: false } // Hide default scrollbar but keep functionality
            NumberAnimation on contentX { duration: Theme.durNormal; easing.type: Theme.easingType }

            property int dragFromIndex: -1    // source index being dragged
            property int dragToIndex: -1      // visual drop target
            property real dragX: 0            // current drag X coordinate

            Row {
                id: tabRow
                height: tabContainer.height
                spacing: 0

                Repeater {
                    id: tabRepeater
                    model: root.model

                    TerminalTab {
                        id: tabDelegate
                        title: model.title
                        active: index === root.currentIndex
                        menuOpen: index === root.contextTabIndex && tabContextMenu.visible

                        // Drag state — shift visually when being dragged
                        property bool isDragging: false
                        z: isDragging ? 100 : 0

                        Behavior on x { enabled: !tabDelegate.isDragging; NumberAnimation { duration: Theme.durFast } }

                        onClicked: root.tabClicked(index)
                        onCloseRequested: root.tabClosed(index)
                        onContextMenuRequested: (menuX, menuY) => {
                            const pos = tabDelegate.mapToItem(root, menuX, menuY)
                            root.openContextMenu(index, pos.x, pos.y)
                        }

                        // ── Drag-to-reorder ─────────────────
                        DragHandler {
                            id: dragHandler
                            target: null  // we handle positioning ourselves
                            xAxis.enabled: true
                            yAxis.enabled: false
                            grabPermissions: PointerHandler.CanTakeOverFromAnything

                            onActiveChanged: {
                                if (active) {
                                    // Start drag
                                    tabContextMenu.close()
                                    tabDelegate.isDragging = true
                                    tabContainer.dragFromIndex = index
                                    tabContainer.dragToIndex = index
                                } else {
                                    // Drop — emit move signal if position changed
                                    tabDelegate.isDragging = false
                                    if (tabContainer.dragFromIndex !== tabContainer.dragToIndex
                                        && tabContainer.dragFromIndex >= 0
                                        && tabContainer.dragToIndex >= 0) {
                                        root.tabMoved(tabContainer.dragFromIndex,
                                                      tabContainer.dragToIndex)
                                    }
                                    tabContainer.dragFromIndex = -1
                                    tabContainer.dragToIndex = -1
                                }
                            }

                            onCentroidChanged: {
                                if (!active) return
                                // Determine which tab slot the drag centroid is over
                                const globalPos = tabDelegate.mapToItem(tabContainer,
                                    centroid.position.x, centroid.position.y)
                                let targetIdx = 0
                                let accW = 0
                                for (let i = 0; i < tabRepeater.count; ++i) {
                                    const tab = tabRepeater.itemAt(i)
                                    if (!tab) continue
                                    accW += tab.width
                                    if (globalPos.x < accW - tab.width / 2) break
                                    targetIdx = i
                                }
                                tabContainer.dragToIndex = Math.min(targetIdx, tabRepeater.count - 1)
                            }
                        }

                        // Visual drag feedback — subtle opacity change
                        opacity: isDragging ? 0.7 : 1.0
                        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }
                    }
                }
            }

            // Drop indicator line — shows where the tab will land
            Rectangle {
                id: dropIndicator
                visible: tabContainer.dragFromIndex >= 0
                         && tabContainer.dragFromIndex !== tabContainer.dragToIndex
                width: 2
                height: parent.height
                color: Theme.accent
                radius: 1
                z: 200

                x: {
                    if (tabContainer.dragToIndex < 0) return 0
                    let accW = 0
                    for (let i = 0; i <= tabContainer.dragToIndex; ++i) {
                        const tab = tabRepeater.itemAt(i)
                        if (tab) accW += tab.width
                    }
                    return accW - 1
                }

                Behavior on x { NumberAnimation { duration: Theme.durFast } }
            }
        }

        IconButton {
            Layout.alignment: Qt.AlignVCenter
            visible: root.hasOverflow
            enabled: root.canScrollRight
            icon: "arrow-left"
            iconRotation: 180
            tooltip: qsTr("Scroll tabs right")
            onClicked: root.scrollTabs(Math.max(tabContainer.width * 0.72, 160))
        }

        IconButton {
            id: overflowButton
            Layout.alignment: Qt.AlignVCenter
            visible: root.hasOverflow
            icon: "chevron-down"
            tooltip: qsTr("All tabs")
            onClicked: {
                const pos = overflowButton.mapToItem(root, 0, 0)
                overflowMenu.x = Math.max(Theme.sp2,
                                          Math.min(root.width - overflowMenu.width - Theme.sp2,
                                                   pos.x + overflowButton.width - overflowMenu.width))
                overflowMenu.y = root.height - 1
                overflowMenu.open()
            }
        }

        IconButton {
            Layout.alignment: Qt.AlignVCenter
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newTabClicked()
        }
    }

    // Bottom border
    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        height: 1
        color: Theme.borderSubtle

        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
    }

    Popup {
        id: tabContextMenu
        width: 184
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

        onClosed: root.contextTabIndex = -1

        contentItem: Column {
            spacing: Theme.sp0_5

            TabMenuItem {
                text: qsTr("Close")
                enabled: root.contextTabIndex >= 0
                onClicked: {
                    const index = root.contextTabIndex
                    tabContextMenu.close()
                    root.tabClosed(index)
                }
            }

            TabMenuItem {
                text: qsTr("Close others")
                enabled: root.contextTabIndex >= 0 && root.model && root.model.count > 1
                onClicked: {
                    const index = root.contextTabIndex
                    tabContextMenu.close()
                    root.closeOtherTabsRequested(index)
                }
            }

            TabMenuItem {
                text: qsTr("Close tabs to the left")
                enabled: root.contextTabIndex > 0
                onClicked: {
                    const index = root.contextTabIndex
                    tabContextMenu.close()
                    root.closeTabsToLeftRequested(index)
                }
            }

            TabMenuItem {
                text: qsTr("Close tabs to the right")
                enabled: root.contextTabIndex >= 0
                         && root.model
                         && root.contextTabIndex < root.model.count - 1
                onClicked: {
                    const index = root.contextTabIndex
                    tabContextMenu.close()
                    root.closeTabsToRightRequested(index)
                }
            }
        }
    }

    Popup {
        id: overflowMenu
        width: 260
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

            Repeater {
                model: root.model

                TabMenuItem {
                    required property string title
                    required property int index

                    width: overflowMenu.width - overflowMenu.leftPadding - overflowMenu.rightPadding
                    text: title
                    active: index === root.currentIndex
                    onClicked: {
                        overflowMenu.close()
                        root.tabClicked(index)
                    }
                }
            }
        }
    }

    component TabMenuItem: Rectangle {
        id: menuItem

        property string text: ""
        property bool active: false
        signal clicked()

        implicitWidth: 184
        implicitHeight: 28
        radius: Theme.radiusSm
        color: active
               ? Theme.bgSelected
               : menuArea.containsMouse ? Theme.bgHover : "transparent"
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
            font.weight: active ? Theme.weightMedium : Theme.weightRegular
            color: active ? Theme.textPrimary : Theme.textSecondary
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

    onCurrentIndexChanged: Qt.callLater(function() { root.ensureTabVisible(root.currentIndex) })
    onWidthChanged: Qt.callLater(function() { root.ensureTabVisible(root.currentIndex) })
    Component.onCompleted: Qt.callLater(function() { root.ensureTabVisible(root.currentIndex) })
}
