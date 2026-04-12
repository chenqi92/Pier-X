import QtQuick
import QtQuick.Layouts
import QtQuick.Controls
import Pier
import "../components"

Rectangle {
    id: root

    property var model: null
    property int currentIndex: 0
    property int contextTabIndex: -1
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

    implicitHeight: Theme.tabBarHeight
    color: Theme.bgPanel

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
        if (tabLeft < tabContainer.contentX)
            tabContainer.contentX = tabLeft
        else if (tabRight > tabContainer.contentX + tabContainer.width)
            tabContainer.contentX = tabRight - tabContainer.width
    }

    function openContextMenu(index, x, y) {
        contextTabIndex = index
        tabContextMenu.x = Math.max(Theme.sp2,
                                    Math.min(root.width - tabContextMenu.width - Theme.sp2, x))
        tabContextMenu.y = root.height - 1
        tabContextMenu.open()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp2
        anchors.rightMargin: Theme.sp2
        spacing: Theme.sp1

        IconButton {
            visible: root.hasOverflow
            enabled: root.canScrollLeft
            icon: "arrow-left"
            tooltip: qsTr("Scroll tabs left")
            onClicked: root.scrollTabs(-Math.max(tabContainer.width * 0.72, 160))
        }

        Flickable {
            id: tabContainer
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            clip: true
            contentWidth: tabRow.width
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.horizontal: ScrollBar { height: 0; active: false; visible: false }
            NumberAnimation on contentX { duration: Theme.durNormal; easing.type: Theme.easingType }

            property int dragFromIndex: -1
            property int dragToIndex: -1

            Row {
                id: tabRow
                height: tabContainer.height
                spacing: Theme.sp1

                Repeater {
                    id: tabRepeater
                    model: root.model

                    TerminalTab {
                        id: tabDelegate
                        title: model.title
                        kind: model.backend || ""
                        active: index === root.currentIndex
                        menuOpen: index === root.contextTabIndex && tabContextMenu.visible

                        property bool isDragging: false
                        z: isDragging ? 100 : 0
                        opacity: isDragging ? 0.72 : 1.0

                        Behavior on x { enabled: !tabDelegate.isDragging; NumberAnimation { duration: Theme.durFast } }
                        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

                        onClicked: root.tabClicked(index)
                        onCloseRequested: root.tabClosed(index)
                        onContextMenuRequested: (menuX, menuY) => {
                            const pos = tabDelegate.mapToItem(root, menuX, menuY)
                            root.openContextMenu(index, pos.x, pos.y)
                        }

                        DragHandler {
                            target: null
                            xAxis.enabled: true
                            yAxis.enabled: false
                            grabPermissions: PointerHandler.CanTakeOverFromAnything

                            onActiveChanged: {
                                if (active) {
                                    tabContextMenu.close()
                                    tabDelegate.isDragging = true
                                    tabContainer.dragFromIndex = index
                                    tabContainer.dragToIndex = index
                                } else {
                                    tabDelegate.isDragging = false
                                    if (tabContainer.dragFromIndex !== tabContainer.dragToIndex
                                        && tabContainer.dragFromIndex >= 0
                                        && tabContainer.dragToIndex >= 0) {
                                        root.tabMoved(tabContainer.dragFromIndex, tabContainer.dragToIndex)
                                    }
                                    tabContainer.dragFromIndex = -1
                                    tabContainer.dragToIndex = -1
                                }
                            }

                            onCentroidChanged: {
                                if (!active)
                                    return
                                const globalPos = tabDelegate.mapToItem(tabContainer,
                                                                         centroid.position.x,
                                                                         centroid.position.y)
                                let targetIdx = 0
                                let accW = 0
                                for (let i = 0; i < tabRepeater.count; ++i) {
                                    const tab = tabRepeater.itemAt(i)
                                    if (!tab)
                                        continue
                                    accW += tab.width + tabRow.spacing
                                    if (globalPos.x < accW - tab.width / 2)
                                        break
                                    targetIdx = i
                                }
                                tabContainer.dragToIndex = Math.min(targetIdx, tabRepeater.count - 1)
                            }
                        }
                    }
                }
            }

            Rectangle {
                visible: tabContainer.dragFromIndex >= 0
                         && tabContainer.dragFromIndex !== tabContainer.dragToIndex
                width: 2
                height: parent.height - Theme.sp1
                radius: 1
                color: Theme.accent
                z: 200
                y: Theme.sp0_5

                x: {
                    if (tabContainer.dragToIndex < 0)
                        return 0
                    let accW = 0
                    for (let i = 0; i <= tabContainer.dragToIndex; ++i) {
                        const tab = tabRepeater.itemAt(i)
                        if (tab)
                            accW += tab.width + tabRow.spacing
                    }
                    return accW - tabRow.spacing / 2 - 1
                }

                Behavior on x { NumberAnimation { duration: Theme.durFast } }
            }
        }

        IconButton {
            visible: root.hasOverflow
            enabled: root.canScrollRight
            icon: "arrow-left"
            iconRotation: 180
            tooltip: qsTr("Scroll tabs right")
            onClicked: root.scrollTabs(Math.max(tabContainer.width * 0.72, 160))
        }

        IconButton {
            id: overflowButton
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
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newTabClicked()
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: Theme.borderSubtle
    }

    Popup {
        id: tabContextMenu
        width: 194
        modal: false
        focus: true
        padding: Theme.sp1
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg
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
        width: 280
        modal: false
        focus: true
        padding: Theme.sp1
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        background: Rectangle {
            color: Theme.bgElevated
            border.color: Theme.borderDefault
            border.width: 1
            radius: Theme.radiusLg
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

        implicitWidth: 194
        implicitHeight: Theme.controlHeight
        radius: Theme.radiusSm
        color: active ? Theme.bgSelected : menuArea.containsMouse ? Theme.bgHover : "transparent"
        opacity: enabled ? 1.0 : 0.48

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
