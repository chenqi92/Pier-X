import QtQuick
import QtQuick.Dialogs
import QtQuick.Effects
import QtQuick.Layouts
import Pier

// Modal dialog — collect SSH connection details and emit `saved` with the
// resulting object. Pure UI; persistence comes from pier-core later.
Item {
    id: root

    property bool open: false
    signal saved(var connection)
    signal cancelled

    visible: open
    z: 9500
    anchors.fill: parent

    function show() {
        nameField.text = ""
        hostField.text = ""
        portField.text = "22"
        userField.text = ""
        passwordField.text = ""
        keyPathField.text = ""
        passphraseField.text = ""
        authCombo.currentIndex = 0
        // Reset animation state before showing
        dialog.scale = 0.96
        dialog.opacity = 0
        open = true
        // Trigger entry animation
        dialog.scale = 1.0
        dialog.opacity = 1.0
        nameField.forceActiveFocus()
    }

    function hide() {
        open = false
    }

    Keys.onEscapePressed: {
        cancelled()
        hide()
    }

    // Backdrop
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.open ? 0.5 : 0.0

        Behavior on opacity { NumberAnimation { duration: Theme.durFast } }

        MouseArea {
            anchors.fill: parent
            enabled: root.open
            onClicked: { root.cancelled(); root.hide() }
        }
    }

    // Dialog card
    Rectangle {
        id: dialog
        anchors.centerIn: parent
        width: 480
        height: form.implicitHeight + Theme.sp4 * 2

        // Entry animation — scale-up + fade-in
        scale: 0.96
        opacity: 0
        Behavior on scale   { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: Theme.durNormal; easing.type: Easing.OutCubic } }
        transformOrigin: Item.Center

        color: Theme.bgElevated
        border.color: Theme.borderDefault
        border.width: 1
        radius: Theme.radiusLg

        Behavior on color        { ColorAnimation { duration: Theme.durNormal } }
        Behavior on border.color { ColorAnimation { duration: Theme.durNormal } }

        layer.enabled: true
        layer.effect: MultiEffect {
            shadowEnabled: true
            shadowColor: "#000000"
            shadowOpacity: 0.5
            shadowBlur: 1.0
            shadowVerticalOffset: 16
        }

        // Block clicks on the card from propagating to the backdrop
        // MouseArea. Without this, clicking any non-interactive area
        // inside the dialog (labels, spacing) would close the dialog.
        MouseArea {
            anchors.fill: parent
            onClicked: (mouse) => mouse.accepted = true
            onPressed: (mouse) => mouse.accepted = true
        }

        ColumnLayout {
            id: form
            anchors.fill: parent
            anchors.margins: Theme.sp4
            spacing: Theme.sp3

            // Title
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp0_5

                    SectionLabel { text: qsTr("Connection") }

                    Text {
                        text: qsTr("New SSH connection")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeH2
                        font.weight: Theme.weightMedium
                        color: Theme.textPrimary

                        Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                    }
                }

                IconButton {
                    icon: "x"
                    tooltip: qsTr("Close")
                    onClicked: { root.cancelled(); root.hide() }
                }
            }

            Separator { Layout.fillWidth: true; Layout.topMargin: Theme.sp1 }

            // Form fields
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Text {
                    text: qsTr("Name")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: nameField
                    Layout.fillWidth: true
                    placeholder: qsTr("My production server")
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2
                    Text {
                        text: qsTr("Host")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        id: hostField
                        Layout.fillWidth: true
                        placeholder: qsTr("example.com")
                    }
                }

                ColumnLayout {
                    Layout.preferredWidth: 96
                    spacing: Theme.sp2
                    Text {
                        text: qsTr("Port")
                        font.family: Theme.fontUi
                        font.pixelSize: Theme.sizeCaption
                        font.weight: Theme.weightMedium
                        color: Theme.textSecondary
                    }
                    PierTextField {
                        id: portField
                        Layout.fillWidth: true
                        text: "22"
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                Text {
                    text: qsTr("Username")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: userField
                    Layout.fillWidth: true
                    placeholder: qsTr("root")
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                Text {
                    text: qsTr("Authentication")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierComboBox {
                    id: authCombo
                    Layout.fillWidth: true
                    // M3c4: all three auth methods wired end-to-end.
                    options: [qsTr("Password"),
                              qsTr("Private key"),
                              qsTr("SSH agent")]
                    currentIndex: 0
                }
            }

            // Password field — shown only when "Password" is the
            // selected auth method. Bullet echo via PierTextField's
            // new `password` property.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                visible: authCombo.currentIndex === 0
                Text {
                    text: qsTr("Password")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                PierTextField {
                    id: passwordField
                    Layout.fillWidth: true
                    placeholder: qsTr("password")
                    password: true
                }
            }

            // ─── Private key block ───────────────────────
            // Visible only when "Private key" is selected.
            // Two fields: an absolute path to the private key
            // file (with a Browse button that opens a native
            // file picker) and an optional passphrase (left
            // empty for unencrypted keys).
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                visible: authCombo.currentIndex === 1

                Text {
                    text: qsTr("Private key file")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    PierTextField {
                        id: keyPathField
                        Layout.fillWidth: true
                        placeholder: qsTr("~/.ssh/id_ed25519")
                    }
                    GhostButton {
                        text: qsTr("Browse…")
                        onClicked: keyFileDialog.open()
                    }
                }

                Text {
                    text: qsTr("Passphrase (optional)")
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    font.weight: Theme.weightMedium
                    color: Theme.textSecondary
                    Layout.topMargin: Theme.sp2
                }
                PierTextField {
                    id: passphraseField
                    Layout.fillWidth: true
                    placeholder: qsTr("leave empty if unencrypted")
                    password: true
                }
            }

            // ─── SSH agent info block ────────────────────
            // The agent path needs no additional fields — it's
            // the only "zero-credentials-collected" option, so
            // the visual density drops to a single explanatory
            // caption so the user isn't confused about why
            // nothing else appeared.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                visible: authCombo.currentIndex === 2

                Text {
                    Layout.fillWidth: true
                    text: Qt.platform.os === "windows"
                        ? qsTr("Uses Pageant / the Windows SSH agent. Make sure it's running and has your keys loaded before connecting.")
                        : qsTr("Uses the SSH agent at $SSH_AUTH_SOCK. Make sure your keys are added (ssh-add -l) before connecting.")
                    wrapMode: Text.Wrap
                    font.family: Theme.fontUi
                    font.pixelSize: Theme.sizeCaption
                    color: Theme.textTertiary

                    Behavior on color { ColorAnimation { duration: Theme.durNormal } }
                }
            }

            // Native file picker for the key path field.
            // The FileDialog remembers its last location across
            // invocations, so users only have to navigate to
            // ~/.ssh once per session.
            FileDialog {
                id: keyFileDialog
                title: qsTr("Select SSH private key")
                fileMode: FileDialog.OpenFile
                onAccepted: {
                    // selectedFile is a file:// URL — strip the
                    // scheme so the path field shows a friendly
                    // absolute path that the Rust SSH layer can
                    // pass directly to load_secret_key.
                    var url = selectedFile.toString()
                    if (url.startsWith("file://")) {
                        url = url.substring(7)
                    }
                    keyPathField.text = url
                }
            }

            Item { Layout.preferredHeight: Theme.sp1 }

            // Action buttons
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Item { Layout.fillWidth: true }

                GhostButton {
                    text: qsTr("Cancel")
                    onClicked: { root.cancelled(); root.hide() }
                }

                PrimaryButton {
                    text: qsTr("Connect")
                    // Require name + host + user always; then
                    // require the auth-method-specific field.
                    // SSH agent needs no extra field — just the
                    // above.
                    enabled: nameField.text.length > 0
                             && hostField.text.length > 0
                             && userField.text.length > 0
                             && (
                                (authCombo.currentIndex === 0
                                    && passwordField.text.length > 0)
                                || (authCombo.currentIndex === 1
                                    && keyPathField.text.length > 0)
                                || authCombo.currentIndex === 2
                             )
                    onClicked: {
                        const conn = {
                            name: nameField.text,
                            host: hostField.text,
                            port: parseInt(portField.text) || 22,
                            username: userField.text,
                            authKind: authCombo.currentIndex === 0 ? "password"
                                    : authCombo.currentIndex === 1 ? "private_key"
                                    : "agent",
                            password: passwordField.text,
                            privateKeyPath: keyPathField.text,
                            passphrase: passphraseField.text
                        }
                        root.saved(conn)
                        root.hide()
                    }
                }
            }
        }
    }
}
