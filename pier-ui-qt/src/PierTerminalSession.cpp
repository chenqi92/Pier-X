#include "PierTerminalSession.h"

#include "pier_terminal.h"

#include <QByteArray>
#include <QDebug>
#include <QMetaObject>

PierTerminalSession::PierTerminalSession(QObject *parent)
    : QObject(parent)
{
}

PierTerminalSession::~PierTerminalSession()
{
    // Deterministic shutdown. ~QObject runs after this, so any queued
    // onCoreNotify callbacks scheduled by the reader thread between
    // now and the pier_terminal_free call will find m_handle == nullptr
    // on the main thread and just early-return harmlessly.
    stop();
}

bool PierTerminalSession::start(const QString &shell, int cols, int rows)
{
    if (m_handle) {
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
    m_running = true;
    emit runningChanged();
    // Seed an initial snapshot so the grid paints even before the
    // shell has written its prompt.
    refreshSnapshot();
    return true;
}

int PierTerminalSession::write(const QString &text)
{
    if (!m_handle) {
        return -1;
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

void PierTerminalSession::stop()
{
    if (!m_handle) {
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
    }
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

    // Grow the cell buffer lazily. Typical 120x40 = 4800 cells,
    // about 77 KB; the buffer is reused across snapshots so there
    // is no per-frame allocation once it has settled.
    const size_t needed = static_cast<size_t>(m_cols) * static_cast<size_t>(m_rows);
    if (m_cells.size() < needed) {
        m_cells.resize(needed);
    }

    PierGridInfo info {};
    const int32_t rc = pier_terminal_snapshot(
        m_handle,
        &info,
        m_cells.data(),
        m_cells.size());

    if (rc == -2) {
        // Grid grew between the size() call and the snapshot call —
        // enlarge and retry exactly once.
        const size_t newNeeded = static_cast<size_t>(info.cols) * static_cast<size_t>(info.rows);
        m_cells.resize(newNeeded);
        const int32_t rc2 = pier_terminal_snapshot(
            m_handle,
            &info,
            m_cells.data(),
            m_cells.size());
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
}
