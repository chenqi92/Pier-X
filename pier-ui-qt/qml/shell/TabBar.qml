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
    signal tabClicked(int index)
    signal tabClosed(int index)
    signal newTabClicked
    signal tabMoved(int from, int to)

    implicitHeight: 32
    color: Theme.bgPanel

    Behavior on color { ColorAnimation { duration: Theme.durNormal } }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Tab row — Repeater in a Row so we can control ordering
        // and implement drag-to-reorder.
        Flickable {
            id: tabContainer
            Layout.fillHeight: true
            Layout.preferredWidth: tabRow.width
            Layout.maximumWidth: parent.width - 40
            clip: true
            contentWidth: tabRow.width
            contentHeight: height
            flickableDirection: Flickable.HorizontalFlick
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.horizontal: ScrollBar { height: 0; active: false; visible: false } // Hide default scrollbar but keep functionality

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

                        // Drag state — shift visually when being dragged
                        property bool isDragging: false
                        z: isDragging ? 100 : 0

                        Behavior on x { enabled: !tabDelegate.isDragging; NumberAnimation { duration: Theme.durFast } }

                        onClicked: root.tabClicked(index)
                        onCloseRequested: root.tabClosed(index)

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
            Layout.leftMargin: Theme.sp1
            icon: "plus"
            tooltip: qsTr("New session")
            onClicked: root.newTabClicked()
        }

        Item { Layout.fillWidth: true }
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
}
