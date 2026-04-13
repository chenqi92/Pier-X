// ─────────────────────────────────────────────────────────
// PierTerminalSession — Qt-side owner of a pier-core terminal
// ─────────────────────────────────────────────────────────
//
// This QObject wraps a single opaque `PierTerminal *` from the
// pier-core C ABI. It is the main-thread face of the terminal
// subsystem: QML creates one, gets signals when the grid updates,
// and calls write/resize/snapshot as needed.
//
// Threading — two boundary crossings
// ──────────────────────────────────
// 1. pier-core's reader thread calls us (via `notifyTrampoline`)
//    with a DataReady or Exited event. We immediately forward
//    that wakeup to our own thread via
//    `QMetaObject::invokeMethod(..., Qt::QueuedConnection)`.
//
// 2. For SSH sessions, the blocking `pier_terminal_new_ssh` call
//    runs on a dedicated `std::thread` we spawn from `startSsh`.
//    When it finishes (success or failure) it posts its result
//    to the main thread via another queued invoke. The connect
//    thread holds a `QPointer<PierTerminalSession>` so if the
//    user closes the tab mid-handshake the result is cleanly
//    dropped and any returned PierTerminal handle is freed right
//    there, without ever touching main-thread state.
//
// Invariant: no Qt type is ever touched from a worker thread
// except the QObject pointer itself (which is just a number to
// `invokeMethod`). This keeps us clear of Qt's "objects live in a
// thread" rule without needing moveToThread dances.

#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariant>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <thread>
#include <vector>

// We need the full PierCell definition for std::vector<PierCell> and
// for the rawCells() accessor; the opaque PierTerminal forward-declared
// by the header is also enough for the m_handle pointer member.
#include "pier_terminal.h"

class PierTerminalSession : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierTerminalSession)

public:
    // Lifecycle of an SSH session from the UI's point of view.
    // A local-shell `start()` call sets status directly to Connected.
    // `startSsh()` transitions Idle → Connecting → (Connected|Failed).
    // `stop()` and the destructor transition into Idle, freeing the
    // handle and cancelling any in-flight connect.
    enum class SshStatus {
        Idle = 0,        // no connection in progress, no child running
        Connecting = 1,  // async SSH handshake in flight
        Connected = 2,   // child is running (local or remote)
        Failed = 3       // last SSH handshake failed; sshErrorMessage is set
    };
    Q_ENUM(SshStatus)

    Q_PROPERTY(int cols READ cols NOTIFY gridChanged FINAL)
    Q_PROPERTY(int rows READ rows NOTIFY gridChanged FINAL)
    Q_PROPERTY(int cursorX READ cursorX NOTIFY gridChanged FINAL)
    Q_PROPERTY(int cursorY READ cursorY NOTIFY gridChanged FINAL)
    Q_PROPERTY(int scrollOffset READ scrollOffset NOTIFY scrollStateChanged FINAL)
    Q_PROPERTY(int maxScrollOffset READ maxScrollOffset NOTIFY scrollStateChanged FINAL)
    Q_PROPERTY(bool cursorVisible READ cursorVisible NOTIFY scrollStateChanged FINAL)
    Q_PROPERTY(int scrollbackLimit READ scrollbackLimit WRITE setScrollbackLimit NOTIFY scrollbackLimitChanged FINAL)
    Q_PROPERTY(bool running READ running NOTIFY runningChanged FINAL)
    Q_PROPERTY(SshStatus status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshErrorMessage READ sshErrorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshTarget READ sshTarget NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshHost READ sshHost NOTIFY statusChanged FINAL)
    Q_PROPERTY(int sshPort READ sshPort NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshUser READ sshUser NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshPassword READ sshPassword NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshCredentialId READ sshCredentialId NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshKeyPath READ sshKeyPath NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString sshPassphraseCredentialId READ sshPassphraseCredentialId NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool sshUsesAgent READ sshUsesAgent NOTIFY statusChanged FINAL)

    // SSH command detected in terminal output (local terminal manual ssh)
    Q_PROPERTY(QString detectedSshHost READ detectedSshHost NOTIFY sshCommandDetected FINAL)
    Q_PROPERTY(int detectedSshPort READ detectedSshPort NOTIFY sshCommandDetected FINAL)
    Q_PROPERTY(QString detectedSshUser READ detectedSshUser NOTIFY sshCommandDetected FINAL)

    explicit PierTerminalSession(QObject *parent = nullptr);
    ~PierTerminalSession() override;

    PierTerminalSession(const PierTerminalSession &) = delete;
    PierTerminalSession &operator=(const PierTerminalSession &) = delete;

    int cols() const { return m_cols; }
    int rows() const { return m_rows; }
    int cursorX() const { return m_cursorX; }
    int cursorY() const { return m_cursorY; }
    int scrollOffset() const { return m_scrollOffset; }
    int maxScrollOffset() const { return m_maxScrollOffset; }
    bool cursorVisible() const { return m_scrollOffset == 0; }
    int scrollbackLimit() const { return m_scrollbackLimit; }
    bool running() const { return m_running; }
    SshStatus status() const { return m_status; }
    QString sshErrorMessage() const { return m_sshErrorMessage; }
    QString sshTarget() const { return m_sshTarget; }
    QString sshHost() const { return m_sshHost; }
    int sshPort() const { return m_sshPort; }
    QString sshUser() const { return m_sshUser; }
    QString sshPassword() const { return m_sshPassword; }
    QString sshCredentialId() const { return m_sshCredentialId; }
    QString sshKeyPath() const { return m_sshKeyPath; }
    QString sshPassphraseCredentialId() const { return m_sshPassphraseCredentialId; }
    bool sshUsesAgent() const { return m_sshUsesAgent; }
    QString detectedSshHost() const { return m_detectedSshHost; }
    int detectedSshPort() const { return m_detectedSshPort; }
    QString detectedSshUser() const { return m_detectedSshUser; }

    // Raw snapshot access for the C++-side renderer. Returns a
    // pointer to the internal cell buffer and fills out_cols/rows.
    // The pointer is valid until the next call to snapshotInto().
    // Callers must NOT retain the pointer across event loop returns.
    const PierCell *rawCells(int *outCols, int *outRows) const {
        if (outCols) *outCols = m_cols;
        if (outRows) *outRows = m_rows;
        return m_cells.empty() ? nullptr : m_cells.data();
    }

public slots:
    // Spawn a local shell. Returns true on success, false if the
    // backend refused (e.g. null shell path, unsupported platform).
    // Transitions status to Connected synchronously on success.
    bool start(const QString &shell, int cols, int rows);

    // Spawn a remote shell over SSH, asynchronously. Returns true
    // immediately if the handshake was scheduled successfully
    // (validation passed + worker thread started). The actual
    // connect runs on a std::thread; watch `status` for the
    // transition to Connected or Failed. On Failed,
    // `sshErrorMessage` holds the reason.
    //
    // Calling startSsh while already Connecting or Connected is
    // rejected (returns false). Call stop() first.
    bool startSsh(const QString &host, int port, const QString &user,
                  const QString &password, int cols, int rows);

    // Spawn a remote shell over SSH where the password lives in
    // the OS keychain rather than crossing the FFI boundary.
    // The Rust SSH layer pulls it from the keychain by id at
    // handshake time. Same async + status semantics as startSsh.
    // Used by the sidebar reconnect path so saved connections
    // never re-prompt for a password.
    bool startSshWithCredential(const QString &host, int port, const QString &user,
                                const QString &credentialId, int cols, int rows);

    // Spawn a remote shell over SSH authenticated by an
    // OpenSSH-format private key file. `passphraseCredentialId`
    // is empty for an unencrypted key, otherwise the keychain
    // id holding the passphrase. The Rust SSH layer pulls the
    // passphrase from the keychain at handshake time —
    // plaintext passphrases never cross the FFI.
    bool startSshWithKey(const QString &host, int port, const QString &user,
                         const QString &privateKeyPath,
                         const QString &passphraseCredentialId,
                         int cols, int rows);

    // Spawn a remote shell over SSH authenticated via the
    // system SSH agent. No credentials cross the FFI at all —
    // the agent signs challenges on its own and pier-core
    // never sees the private keys.
    bool startSshWithAgent(const QString &host, int port,
                           const QString &user,
                           int cols, int rows);

    // Spawn a remote shell over an existing shared SSH session.
    // No handshake — the session is already authenticated.
    // The session handle is obtained from PierSshSessionHandle.
    bool startSshOnSession(QObject *sessionHandle, int cols, int rows);

    // Cancel an in-progress SSH handshake. If the worker thread
    // has not yet returned from pier_terminal_new_ssh, we can't
    // interrupt it — instead we flag the request as cancelled, so
    // when the thread eventually returns it frees the handle
    // (if any) and discards the result. Transitions status back
    // to Idle. No-op if no connect is in flight.
    void cancelSsh();

    // Send UTF-8 bytes (keystrokes, paste, etc.) to the shell.
    // Returns the number of bytes written, or -1 on error.
    int write(const QString &text);

    // Tell the shell its visible area is now cols x rows cells.
    bool resize(int cols, int rows);

    // Scroll the terminal viewport by a number of history lines.
    // Positive values move up into older content; negative values move
    // back down toward the live bottom.
    void scrollBy(int lines);

    // Jump directly back to live terminal output.
    void scrollToBottom();

    // Update the bounded scrollback limit retained by pier-core.
    void setScrollbackLimit(int limit);

    // Shut down and reap the child. Safe to call multiple times.
    // Also cancels any in-flight SSH handshake.
    void stop();

signals:
    // Fired on the main thread whenever the grid might have changed.
    // QML should react by requesting a repaint of the grid item.
    void gridChanged();

    // Fired exactly once when the child process exits. `running`
    // transitions from true to false immediately before this.
    void exited();

    // Mirrors the Qt property change; exists so QML bindings on
    // `running` update correctly.
    void runningChanged();

    // Fires when scrollback state changes: current offset, maximum
    // available offset, or cursor visibility.
    void scrollStateChanged();

    // Fires when the configured scrollback cap changes.
    void scrollbackLimitChanged();

    // Fires when status / sshErrorMessage / sshTarget change.
    // Combining all three into one signal is deliberate — QML
    // bindings on any of them update together and there's never a
    // useful moment to re-render one without the others.
    void statusChanged();

    // Fired when the emulator detects an SSH command in terminal output.
    void sshCommandDetected();

    // Fired when the emulator detects `exit` or `logout` — user left
    // the current SSH session. Right sidebar should disconnect.
    void sshExitDetected();

private slots:
    // Runs on the main thread. Called via queued connection from
    // the reader thread's notify callback.
    void onCoreNotify(int event);

    // Runs on the main thread. Called via queued connection from
    // the SSH connect worker thread with either a ready handle
    // (success) or a null handle + error message (failure).
    // `requestId` identifies which startSsh invocation produced
    // this result — stale deliveries from a cancelled request
    // are detected by comparing to m_connectRequestId and
    // silently dropped.
    void onSshConnectResult(quint64 requestId, void *handle, const QString &errorMessage);

private:
    // Trampoline entry point for the pier-core reader thread. Must
    // be thread-safe and must not touch Qt types except the QObject
    // pointer in `user_data`.
    static void notifyTrampoline(void *user_data, uint32_t event);

    // Pull a fresh snapshot from pier-core into m_cells + metadata.
    void refreshSnapshot();

    // Internal helper: set status + emit statusChanged only if it
    // actually changed. Centralizes the invariant that every
    // status transition notifies QML bindings exactly once.
    void setStatus(SshStatus s);
    void clearSshContext();

    // M3c3: factory-style dispatcher.
    //
    // The three pier_terminal_new_ssh* C functions don't share
    // a single signature anymore — the key-auth variant takes
    // an extra `passphrase_credential_id` argument that the
    // password / credential variants don't have. Rather than
    // forcing them into one typedef, the dispatcher takes a
    // `std::function<PierTerminal*(void*)>` factory closure
    // that the worker thread invokes with the notify
    // trampoline's user_data. Each public start* method
    // captures whatever args it needs (host, port, user,
    // secret, key path, passphrase id, ...) into its own
    // closure and passes a fresh closure to the dispatcher.
    //
    // Result: the entire 70-line worker-thread + cancel-flag +
    // queued-result body lives in dispatchSshConnect() exactly
    // once, regardless of how many auth methods we add.
    using SshConnectFactory = std::function<PierTerminal *(void *user_data)>;

    bool dispatchSshConnect(const QString &targetLabel,
                            int cols, int rows,
                            SshConnectFactory factory);

    PierTerminal *m_handle = nullptr;
    std::vector<PierCell> m_cells;
    int m_cols = 0;
    int m_rows = 0;
    int m_cursorX = 0;
    int m_cursorY = 0;
    int m_scrollOffset = 0;
    int m_maxScrollOffset = 0;
    int m_scrollbackLimit = 10'000;
    bool m_running = false;

    SshStatus m_status = SshStatus::Idle;
    QString m_sshErrorMessage;
    QString m_sshTarget;

    // Cached SSH auth context from the most recent startSsh* call.
    QString m_sshHost;
    int     m_sshPort = 22;
    QString m_sshUser;
    QString m_sshPassword;
    QString m_sshCredentialId;
    QString m_sshKeyPath;
    QString m_sshPassphraseCredentialId;
    bool    m_sshUsesAgent = false;

    // SSH command detected in terminal output
    QString m_detectedSshHost;
    int     m_detectedSshPort = 22;
    QString m_detectedSshUser;

    // In-flight SSH handshake bookkeeping.
    //
    // m_connectThread:        the worker thread running the blocking
    //                         FFI. Detached on cancel / destructor
    //                         so we never block the UI on join.
    // m_connectCancelFlag:    shared between session and worker.
    //                         The worker checks it after the FFI
    //                         returns; if set, the worker frees
    //                         any returned handle and discards.
    // m_connectRequestId:     monotonically increasing. onSshConnectResult
    //                         compares this to the id stamped on
    //                         the worker's delivery and drops stale
    //                         ones.
    std::unique_ptr<std::thread> m_connectThread;
    std::shared_ptr<std::atomic<bool>> m_connectCancelFlag;
    quint64 m_connectRequestId = 0;
};
