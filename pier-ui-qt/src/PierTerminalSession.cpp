#include "PierTerminalSession.h"
#include "PierSshSessionHandle.h"

#include "pier_terminal.h"
#include "pier_ssh_session.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QPointer>

#include <algorithm>
#include <utility>

PierTerminalSession::PierTerminalSession(QObject *parent)
    : QObject(parent)
{
}

PierTerminalSession::~PierTerminalSession()
{
    // Deterministic shutdown. ~QObject runs after this, so any queued
    // onCoreNotify / onSshConnectResult callbacks scheduled by other
    // threads between now and the pier_terminal_free call will find
    // m_handle == nullptr (for onCoreNotify) or a bumped
    // m_connectRequestId (for onSshConnectResult) on the main thread
    // and just early-return harmlessly.
    stop();
    // If a connect thread is still running after stop() returned,
    // detach it so the OS reaps it when it finishes. It already
    // holds the cancel flag (checked inside the worker) so any
    // handle it produces will be freed there.
    if (m_connectThread && m_connectThread->joinable()) {
        m_connectThread->detach();
    }
}

bool PierTerminalSession::start(const QString &shell, int cols, int rows)
{
    if (m_handle || m_status == SshStatus::Connecting) {
        qWarning() << "PierTerminalSession::start called on already-running session";
        return false;
    }
    if (cols <= 0 || rows <= 0) {
        qWarning() << "PierTerminalSession::start rejected invalid size"
                   << cols << "x" << rows;
        return false;
    }

    const QByteArray shellUtf8 = shell.toUtf8();
    m_handle = pier_terminal_new(
        static_cast<uint16_t>(cols),
        static_cast<uint16_t>(rows),
        shellUtf8.constData(),
        &PierTerminalSession::notifyTrampoline,
        this);
    if (!m_handle) {
        qWarning() << "pier_terminal_new failed for" << shell;
        return false;
    }

    m_cols = cols;
    m_rows = rows;
    m_scrollOffset = 0;
    m_maxScrollOffset = 0;
    m_running = true;
    pier_terminal_set_scrollback_limit(m_handle, static_cast<uint32_t>(m_scrollbackLimit));
    emit runningChanged();
    // Local shells transition directly to Connected — there is no
    // handshake phase to display a "Connecting..." overlay for.
    setStatus(SshStatus::Connected);
    // Seed an initial snapshot so the grid paints even before the
    // shell has written its prompt.
    refreshSnapshot();
    return true;
}

void PierTerminalSession::clearSshContext()
{
    m_sshHost.clear();
    m_sshPort = 22;
    m_sshUser.clear();
    m_sshPassword.clear();
    m_sshCredentialId.clear();
    m_sshKeyPath.clear();
    m_sshPassphraseCredentialId.clear();
    m_sshUsesAgent = false;
}

bool PierTerminalSession::startSsh(const QString &host, int port, const QString &user,
                                   const QString &password, int cols, int rows)
{
    if (host.isEmpty() || user.isEmpty()) {
        qWarning() << "startSsh: empty host/user";
        return false;
    }
    clearSshContext();
    m_sshHost = host;
    m_sshPort = port;
    m_sshUser = user;
    m_sshPassword = password;
    const QString target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const uint16_t colsU16 = static_cast<uint16_t>(cols);
    const uint16_t rowsU16 = static_cast<uint16_t>(rows);
    // Capture everything by value as std::string so the QByteArray
    // temporaries can drop with the main-thread stack frame.
    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string passStd = password.toStdString();

    auto factory = [hostStd = std::move(hostStd),
                    userStd = std::move(userStd),
                    passStd = std::move(passStd),
                    portU16, colsU16, rowsU16](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh(
            colsU16, rowsU16,
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            passStd.c_str(),
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };
    return dispatchSshConnect(target, cols, rows, std::move(factory));
}

bool PierTerminalSession::startSshWithCredential(const QString &host, int port,
                                                 const QString &user,
                                                 const QString &credentialId,
                                                 int cols, int rows)
{
    if (host.isEmpty() || user.isEmpty() || credentialId.isEmpty()) {
        qWarning() << "startSshWithCredential: empty host/user/credentialId";
        return false;
    }
    clearSshContext();
    m_sshHost = host;
    m_sshPort = port;
    m_sshUser = user;
    m_sshCredentialId = credentialId;

    const QString target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const uint16_t colsU16 = static_cast<uint16_t>(cols);
    const uint16_t rowsU16 = static_cast<uint16_t>(rows);
    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string credStd = credentialId.toStdString();

    auto factory = [hostStd = std::move(hostStd),
                    userStd = std::move(userStd),
                    credStd = std::move(credStd),
                    portU16, colsU16, rowsU16](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh_credential(
            colsU16, rowsU16,
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            credStd.c_str(),
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };
    return dispatchSshConnect(target, cols, rows, std::move(factory));
}

bool PierTerminalSession::startSshWithKey(const QString &host, int port,
                                          const QString &user,
                                          const QString &privateKeyPath,
                                          const QString &passphraseCredentialId,
                                          int cols, int rows)
{
    if (host.isEmpty() || user.isEmpty() || privateKeyPath.isEmpty()) {
        qWarning() << "startSshWithKey: empty host/user/keyPath";
        return false;
    }
    clearSshContext();
    m_sshHost = host;
    m_sshPort = port;
    m_sshUser = user;
    m_sshKeyPath = privateKeyPath;
    m_sshPassphraseCredentialId = passphraseCredentialId;

    const QString target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const uint16_t colsU16 = static_cast<uint16_t>(cols);
    const uint16_t rowsU16 = static_cast<uint16_t>(rows);
    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string keyStd = privateKeyPath.toStdString();
    // Empty string maps to "no passphrase" — the FFI accepts
    // both null and empty as the unencrypted-key signal.
    std::string passId = passphraseCredentialId.toStdString();
    bool hasPassphrase = !passId.empty();

    auto factory = [hostStd = std::move(hostStd),
                    userStd = std::move(userStd),
                    keyStd = std::move(keyStd),
                    passId = std::move(passId),
                    hasPassphrase,
                    portU16, colsU16, rowsU16](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh_key(
            colsU16, rowsU16,
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            keyStd.c_str(),
            hasPassphrase ? passId.c_str() : nullptr,
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };
    return dispatchSshConnect(target, cols, rows, std::move(factory));
}

bool PierTerminalSession::startSshWithAgent(const QString &host, int port,
                                             const QString &user,
                                             int cols, int rows)
{
    if (host.isEmpty() || user.isEmpty()) {
        qWarning() << "startSshWithAgent: empty host/user";
        return false;
    }
    clearSshContext();
    m_sshHost = host;
    m_sshPort = port;
    m_sshUser = user;
    m_sshUsesAgent = true;

    const QString target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const uint16_t colsU16 = static_cast<uint16_t>(cols);
    const uint16_t rowsU16 = static_cast<uint16_t>(rows);
    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();

    auto factory = [hostStd = std::move(hostStd),
                    userStd = std::move(userStd),
                    portU16, colsU16, rowsU16](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh_agent(
            colsU16, rowsU16,
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };
    return dispatchSshConnect(target, cols, rows, std::move(factory));
}

bool PierTerminalSession::startSshOnSession(QObject *sessionObj, int cols, int rows)
{
    auto *sh = qobject_cast<PierSshSessionHandle *>(sessionObj);
    if (!sh || !sh->handle()) {
        qWarning() << "startSshOnSession: invalid session handle";
        return false;
    }

    // Cache SSH context from the session handle
    clearSshContext();
    m_sshHost = sh->target().section('@', 1).section(':', 0, 0);
    m_sshPort = sh->target().section(':', -1).toInt();
    m_sshUser = sh->target().section('@', 0, 0);

    const QString target = sh->target();
    ::PierSshSession *session = sh->handle();

    auto factory = [session](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh_on_session(
            session,
            0, 0,  // cols/rows set by dispatchSshConnect
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };

    // dispatchSshConnect will set proper cols/rows in its own factory wrapper
    // But since pier_terminal_new_ssh_on_session takes cols/rows directly,
    // we need to capture them properly.
    const uint16_t colsU16 = static_cast<uint16_t>(cols);
    const uint16_t rowsU16 = static_cast<uint16_t>(rows);

    auto factoryWithSize = [session, colsU16, rowsU16](void *user_data) -> PierTerminal * {
        return pier_terminal_new_ssh_on_session(
            session,
            colsU16, rowsU16,
            &PierTerminalSession::notifyTrampoline,
            user_data);
    };

    return dispatchSshConnect(target, cols, rows, std::move(factoryWithSize));
}

bool PierTerminalSession::dispatchSshConnect(const QString &targetLabel,
                                             int cols, int rows,
                                             SshConnectFactory factory)
{
    if (m_handle || m_status == SshStatus::Connecting) {
        qWarning() << "PierTerminalSession::startSsh* called on already-running session";
        return false;
    }
    if (cols <= 0 || rows <= 0) {
        qWarning() << "PierTerminalSession::startSsh* rejected invalid size"
                   << cols << "x" << rows;
        return false;
    }
    if (!factory) {
        qWarning() << "PierTerminalSession::dispatchSshConnect: null factory";
        return false;
    }

    // Prepare a fresh request: bump the id, install a new cancel
    // flag, record the target for the "Connecting..." overlay.
    const quint64 requestId = ++m_connectRequestId;
    m_connectCancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_sshErrorMessage.clear();
    m_sshTarget = targetLabel;
    m_cols = cols;
    m_rows = rows;
    setStatus(SshStatus::Connecting);

    // If a previous worker thread is still hanging around (a
    // cancelled connect that hasn't returned yet), detach it so
    // the OS reaps it when it finishes. Its cancel flag is
    // already set, so it will clean up its own PierTerminal handle
    // if the blocking FFI happens to produce one before the thread
    // notices cancellation.
    if (m_connectThread && m_connectThread->joinable()) {
        m_connectThread->detach();
    }
    m_connectThread.reset();

    QPointer<PierTerminalSession> selfWeak(this);
    auto cancelFlag = m_connectCancelFlag;

    m_connectThread = std::make_unique<std::thread>([
        selfWeak,
        cancelFlag,
        requestId,
        factory = std::move(factory)
    ]() mutable {
        // ── worker thread ──────────────────────────────────────
        // Blocking FFI call — this is exactly why we're on a
        // dedicated thread. On LAN this returns in ~300 ms; on
        // a dead host it can take 15+ seconds.
        //
        // IMPORTANT: we must not pass the QPointer itself through
        // as user_data for notifyTrampoline, because the
        // trampoline runs on pier-core's reader thread (a
        // different thread from us) and a QPointer is not
        // Send-safe to share across arbitrary threads. The real
        // session pointer works because QMetaObject::invokeMethod
        // with Qt::QueuedConnection is documented thread-safe for
        // any non-null QObject*. If the session has been deleted
        // by then, we catch it below by checking selfWeak and
        // freeing the handle before any invoke is posted.
        PierTerminalSession *rawSelf = selfWeak.data();
        PierTerminal *handle = factory(static_cast<void *>(rawSelf));

        QString errorMessage;
        if (!handle) {
            // Copy the thread-local error message into an owned
            // QString RIGHT HERE on the worker thread, before any
            // cross-thread delivery. The thread-local slot would
            // be the wrong thread's slot on the main thread.
            const char *lastErr = pier_terminal_last_ssh_error();
            if (lastErr && *lastErr) {
                errorMessage = QString::fromUtf8(lastErr);
            } else {
                errorMessage = QStringLiteral("SSH connect failed (no detail)");
            }
        }

        // Cancellation / session-deleted checks. If either is set
        // and we DO have a handle, we must free it here on the
        // worker thread — the main thread has no way to find out
        // about it otherwise.
        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (handle) {
                pier_terminal_free(handle);
            }
            return;
        }

        // Post the result back to the main thread. Using the
        // typed slot + Q_ARG form rather than a lambda overload
        // so moc-generated dispatch handles everything. The
        // slot itself rechecks m_connectRequestId against
        // `requestId` to catch races where cancelSsh ran between
        // our cancelFlag check above and the queued invoke
        // actually arriving.
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onSshConnectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(handle)),
            Q_ARG(QString, errorMessage));
    });

    return true;
}

void PierTerminalSession::cancelSsh()
{
    if (m_status != SshStatus::Connecting) {
        return;
    }
    // Flag the in-flight worker as cancelled. It will eventually
    // return from pier_terminal_new_ssh, see the flag, and either
    // discard the produced handle or just exit.
    if (m_connectCancelFlag) {
        m_connectCancelFlag->store(true);
    }
    // Bump the request id so any already-queued onSshConnectResult
    // delivery from this worker lands with a stale id and is
    // dropped.
    ++m_connectRequestId;
    m_sshErrorMessage.clear();
    m_sshTarget.clear();
    setStatus(SshStatus::Idle);
}

int PierTerminalSession::write(const QString &text)
{
    if (!m_handle) {
        return -1;
    }
    if (m_scrollOffset != 0) {
        m_scrollOffset = 0;
        refreshSnapshot();
        emit gridChanged();
    }
    const QByteArray utf8 = text.toUtf8();
    const int64_t n = pier_terminal_write(
        m_handle,
        reinterpret_cast<const uint8_t *>(utf8.constData()),
        static_cast<size_t>(utf8.size()));
    return n < 0 ? -1 : static_cast<int>(n);
}

bool PierTerminalSession::resize(int cols, int rows)
{
    if (!m_handle || cols <= 0 || rows <= 0) {
        return false;
    }
    const int32_t rc = pier_terminal_resize(
        m_handle,
        static_cast<uint16_t>(cols),
        static_cast<uint16_t>(rows));
    if (rc != 0) {
        qWarning() << "pier_terminal_resize failed rc=" << rc;
        return false;
    }
    m_cols = cols;
    m_rows = rows;
    refreshSnapshot();
    emit gridChanged();
    return true;
}

void PierTerminalSession::scrollBy(int lines)
{
    if (!m_handle || lines == 0) {
        return;
    }
    const int nextOffset = std::clamp(m_scrollOffset + lines, 0, m_maxScrollOffset);
    if (nextOffset == m_scrollOffset) {
        return;
    }
    m_scrollOffset = nextOffset;
    refreshSnapshot();
    emit gridChanged();
}

void PierTerminalSession::scrollToBottom()
{
    if (m_scrollOffset == 0) {
        return;
    }
    m_scrollOffset = 0;
    refreshSnapshot();
    emit gridChanged();
}

void PierTerminalSession::setScrollbackLimit(int limit)
{
    limit = std::max(1, limit);
    if (m_scrollbackLimit == limit) {
        return;
    }
    m_scrollbackLimit = limit;
    if (m_handle) {
        pier_terminal_set_scrollback_limit(m_handle, static_cast<uint32_t>(m_scrollbackLimit));
        const int newMaxScrollOffset =
            static_cast<int>(pier_terminal_scrollback_len(m_handle));
        m_maxScrollOffset = newMaxScrollOffset;
        if (m_scrollOffset > m_maxScrollOffset) {
            m_scrollOffset = m_maxScrollOffset;
        }
        refreshSnapshot();
        emit gridChanged();
    }
    emit scrollbackLimitChanged();
}

void PierTerminalSession::stop()
{
    // First, cancel any in-flight SSH handshake. Safe no-op if
    // none is running.
    cancelSsh();

    const bool hadScrollState = m_scrollOffset != 0 || m_maxScrollOffset != 0;
    m_scrollOffset = 0;
    m_maxScrollOffset = 0;

    if (!m_handle) {
        if (m_status != SshStatus::Idle) {
            m_sshErrorMessage.clear();
            m_sshTarget.clear();
            clearSshContext();
            setStatus(SshStatus::Idle);
        }
        if (hadScrollState) {
            emit scrollStateChanged();
        }
        return;
    }
    PierTerminal *h = m_handle;
    m_handle = nullptr;
    // pier_terminal_free blocks until the reader thread has joined
    // and the child has been reaped. Clearing m_handle before the
    // call is deliberate: if the reader thread manages to enqueue
    // one final onCoreNotify event during the join, it fires on the
    // main thread after ~QObject has returned from stop() but still
    // inside the event loop turn, and finds m_handle == nullptr so
    // it no-ops instead of dereferencing a freed pointer.
    pier_terminal_free(h);

    if (m_running) {
        m_running = false;
        emit runningChanged();
        emit exited();
    }
    // Transition out of Connected/Failed into Idle so the overlay
    // doesn't linger on a torn-down session.
    if (m_status != SshStatus::Idle) {
        m_sshErrorMessage.clear();
        m_sshTarget.clear();
        clearSshContext();
        setStatus(SshStatus::Idle);
    }
    if (hadScrollState) {
        emit scrollStateChanged();
    }
}

void PierTerminalSession::onCoreNotify(int event)
{
    // ── main thread ────────────────────────────────────────
    // We got here via a queued connection scheduled from the
    // reader thread. If stop() already ran (m_handle == nullptr)
    // the wakeup is stale — just drop it.
    if (!m_handle) {
        return;
    }

    refreshSnapshot();
    emit gridChanged();

    if (event == 1 /* PIER_NOTIFY_EXITED */) {
        // Mirror what stop() does minus the pier_terminal_free:
        // the session is already cleanly shut down on the Rust
        // side — the reader thread emitted this event and exited.
        // We still have to free the handle though, because the C++
        // side owns it.
        PierTerminal *h = m_handle;
        m_handle = nullptr;
        pier_terminal_free(h);
        if (m_running) {
            m_running = false;
            emit runningChanged();
            emit exited();
        }
        // Leave status at Connected — the user should still see
        // the final grid. A future iteration can show a banner
        // with the exit code. We intentionally do NOT transition
        // to Failed because the connect DID succeed; the child
        // exited later, which is a different event.
    }
}

void PierTerminalSession::onSshConnectResult(quint64 requestId, void *handle, const QString &errorMessage)
{
    // ── main thread ────────────────────────────────────────
    // Any delivery with a stale request id is from a cancelled
    // connect — drop it, freeing the handle if there was one.
    if (requestId != m_connectRequestId) {
        if (handle) {
            pier_terminal_free(static_cast<PierTerminal *>(handle));
        }
        return;
    }

    // The worker thread has handed us its final result. Drop our
    // reference to the thread itself; the join happens below.
    // detach() is safe because the worker has already returned
    // from its closure (otherwise we wouldn't be in this slot).
    if (m_connectThread && m_connectThread->joinable()) {
        m_connectThread->detach();
    }
    m_connectThread.reset();
    m_connectCancelFlag.reset();

    if (!handle) {
        // Failure path. Surface the error to the QML overlay.
        m_sshErrorMessage = errorMessage.isEmpty()
            ? QStringLiteral("SSH connect failed")
            : errorMessage;
        setStatus(SshStatus::Failed);
        return;
    }

    // Success path. Adopt the handle and run the same
    // "session is now live" bookkeeping as the local start().
    m_handle = static_cast<PierTerminal *>(handle);
    pier_terminal_set_scrollback_limit(m_handle, static_cast<uint32_t>(m_scrollbackLimit));
    m_scrollOffset = 0;
    m_maxScrollOffset = 0;
    m_running = true;
    emit runningChanged();
    setStatus(SshStatus::Connected);
    refreshSnapshot();
    emit gridChanged();
}

void PierTerminalSession::notifyTrampoline(void *user_data, uint32_t event)
{
    // ── reader thread ──────────────────────────────────────
    // We are not allowed to touch ANY Qt state from here except
    // the bare QObject pointer, because this runs on a non-Qt
    // thread. QMetaObject::invokeMethod with Qt::QueuedConnection
    // is safe to call from any thread — it just posts an event
    // to the target object's thread. The int cast is safe because
    // PierNotifyEvent values are small (0 or 1 today).
    auto *self = static_cast<PierTerminalSession *>(user_data);
    if (!self) {
        return;
    }
    QMetaObject::invokeMethod(
        self,
        "onCoreNotify",
        Qt::QueuedConnection,
        Q_ARG(int, static_cast<int>(event)));
}

void PierTerminalSession::refreshSnapshot()
{
    if (!m_handle) {
        return;
    }

    const int previousScrollOffset = m_scrollOffset;
    const int previousMaxScrollOffset = m_maxScrollOffset;
    const int scrollbackLen = static_cast<int>(pier_terminal_scrollback_len(m_handle));
    if (m_scrollOffset > 0 && scrollbackLen > m_maxScrollOffset) {
        m_scrollOffset += (scrollbackLen - m_maxScrollOffset);
    }
    m_maxScrollOffset = scrollbackLen;
    if (m_scrollOffset > m_maxScrollOffset) {
        m_scrollOffset = m_maxScrollOffset;
    }

    // Grow the cell buffer lazily. Typical 120x40 = 4800 cells,
    // about 77 KB; the buffer is reused across snapshots so there
    // is no per-frame allocation once it has settled.
    const size_t needed = static_cast<size_t>(m_cols) * static_cast<size_t>(m_rows);
    if (m_cells.size() < needed) {
        m_cells.resize(needed);
    }

    PierGridInfo info {};
    const int32_t rc = pier_terminal_snapshot_view(
        m_handle,
        &info,
        m_cells.data(),
        m_cells.size(),
        static_cast<uint32_t>(m_scrollOffset));

    if (rc == -2) {
        // Grid grew between the size() call and the snapshot call —
        // enlarge and retry exactly once.
        const size_t newNeeded = static_cast<size_t>(info.cols) * static_cast<size_t>(info.rows);
        m_cells.resize(newNeeded);
        const int32_t rc2 = pier_terminal_snapshot_view(
            m_handle,
            &info,
            m_cells.data(),
            m_cells.size(),
            static_cast<uint32_t>(m_scrollOffset));
        if (rc2 != 0) {
            qWarning() << "pier_terminal_snapshot retry failed rc=" << rc2;
            return;
        }
    } else if (rc != 0) {
        qWarning() << "pier_terminal_snapshot failed rc=" << rc;
        return;
    }

    m_cols = info.cols;
    m_rows = info.rows;
    m_cursorX = info.cursor_x;
    m_cursorY = info.cursor_y;
    const bool nowRunning = (info.alive != 0);
    if (nowRunning != m_running) {
        m_running = nowRunning;
        emit runningChanged();
    }
    if (previousScrollOffset != m_scrollOffset
        || previousMaxScrollOffset != m_maxScrollOffset) {
        emit scrollStateChanged();
    }

    // Check if the emulator detected an SSH command
    char *sshJson = pier_terminal_ssh_detected(m_handle);
    if (sshJson) {
        QString json = QString::fromUtf8(sshJson);
        pier_terminal_free_string(sshJson);
        QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
        if (doc.isObject()) {
            QJsonObject obj = doc.object();
            if (obj.value(QStringLiteral("detected")).toBool()) {
                m_detectedSshHost = obj.value(QStringLiteral("host")).toString();
                m_detectedSshPort = obj.value(QStringLiteral("port")).toInt(22);
                m_detectedSshUser = obj.value(QStringLiteral("user")).toString();
                emit sshCommandDetected();
            }
        }
    }
}

void PierTerminalSession::setStatus(SshStatus s)
{
    if (m_status == s) {
        return;
    }
    m_status = s;
    emit statusChanged();
}
