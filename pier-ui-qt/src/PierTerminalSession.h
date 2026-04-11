// ─────────────────────────────────────────────────────────
// PierTerminalSession — Qt-side owner of a pier-core terminal
// ─────────────────────────────────────────────────────────
//
// This QObject wraps a single opaque `PierTerminal *` from the
// pier-core C ABI. It is the main-thread face of the terminal
// subsystem: QML creates one, gets signals when the grid updates,
// and calls write/resize/snapshot as needed.
//
// Threading
// ─────────
//   pier-core's reader thread calls us (via `notifyTrampoline`) with
//   a DataReady or Exited event. We immediately forward that wakeup
//   to our own thread via `QMetaObject::invokeMethod(...,
//   Qt::QueuedConnection)`. Every Qt signal this class emits, every
//   write/resize/snapshot it makes, runs on the main thread.
//
//   The invariant: no Qt type is ever touched from the reader thread
//   except the QObject pointer itself (which is just a number to
//   `invokeMethod`). This keeps us clear of Qt's "objects live in a
//   thread" rule without needing moveToThread dances.

#pragma once

#include <QObject>
#include <QString>
#include <QVariant>
#include <qqml.h>

#include <cstdint>
#include <vector>

// We need the full PierCell definition for std::vector<PierCell> and
// for the rawCells() accessor; the opaque PierTerminal forward-declared
// by the header is also enough for the m_handle pointer member.
#include "pier_terminal.h"

class PierTerminalSession : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierTerminalSession)

    Q_PROPERTY(int cols READ cols NOTIFY gridChanged FINAL)
    Q_PROPERTY(int rows READ rows NOTIFY gridChanged FINAL)
    Q_PROPERTY(int cursorX READ cursorX NOTIFY gridChanged FINAL)
    Q_PROPERTY(int cursorY READ cursorY NOTIFY gridChanged FINAL)
    Q_PROPERTY(bool running READ running NOTIFY runningChanged FINAL)

public:
    explicit PierTerminalSession(QObject *parent = nullptr);
    ~PierTerminalSession() override;

    PierTerminalSession(const PierTerminalSession &) = delete;
    PierTerminalSession &operator=(const PierTerminalSession &) = delete;

    int cols() const { return m_cols; }
    int rows() const { return m_rows; }
    int cursorX() const { return m_cursorX; }
    int cursorY() const { return m_cursorY; }
    bool running() const { return m_running; }

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
    // Emits gridChanged once the initial prompt lands.
    bool start(const QString &shell, int cols, int rows);

    // Spawn a remote shell over SSH. Same ownership semantics as
    // start() — the C++ object owns the opaque PierTerminal handle
    // for the lifetime of the session, and the same gridChanged /
    // exited signals fire.
    //
    // BLOCKING: this method runs the full SSH handshake
    // (TCP + key exchange + auth + pty-req + shell-req) on the
    // calling thread before returning. Typical LAN latency is
    // under 300 ms; expect 1–3 s across the internet. Qt's event
    // loop is blocked for the duration. M3c introduces an async
    // variant that fires a signal on completion.
    //
    // Returns true on success, false on any failure (invalid
    // args, DNS / TCP / auth / host key / channel open error).
    bool startSsh(const QString &host, int port, const QString &user,
                  const QString &password, int cols, int rows);

    // Send UTF-8 bytes (keystrokes, paste, etc.) to the shell.
    // Returns the number of bytes written, or -1 on error.
    int write(const QString &text);

    // Tell the shell its visible area is now cols x rows cells.
    bool resize(int cols, int rows);

    // Shut down and reap the child. Safe to call multiple times.
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

private slots:
    // Runs on the main thread. Called via queued connection from
    // the reader thread's notify callback.
    void onCoreNotify(int event);

private:
    // Trampoline entry point for the pier-core reader thread. Must
    // be thread-safe and must not touch Qt types except the QObject
    // pointer in `user_data`.
    static void notifyTrampoline(void *user_data, uint32_t event);

    // Pull a fresh snapshot from pier-core into m_cells + metadata.
    void refreshSnapshot();

    PierTerminal *m_handle = nullptr;
    std::vector<PierCell> m_cells;
    int m_cols = 0;
    int m_rows = 0;
    int m_cursorX = 0;
    int m_cursorY = 0;
    bool m_running = false;
};
