// ─────────────────────────────────────────────────────────
// PierSshSessionHandle — QML-friendly shared SSH session
// ─────────────────────────────────────────────────────────
//
// Wraps an opaque PierSshSession* from pier-core's M3e
// shared session API. One instance per SSH tab; child tools
// (Docker, SFTP, Monitor, LogStream, Terminal) all call the
// `_on_session` FFI constructors with this handle's pointer,
// eliminating redundant SSH handshakes.
//
// Threading: open() dispatches the blocking handshake to a
// worker thread and delivers the result via queued invoke.

#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <qqml.h>

#include <atomic>
#include <memory>
#include <thread>

struct PierSshSession;

class PierSshSessionHandle : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierSshSessionHandle)

    Q_PROPERTY(bool connected READ connected NOTIFY connectedChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY connectedChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY connectedChanged FINAL)

public:
    explicit PierSshSessionHandle(QObject *parent = nullptr);
    ~PierSshSessionHandle() override;

    PierSshSessionHandle(const PierSshSessionHandle &) = delete;
    PierSshSessionHandle &operator=(const PierSshSessionHandle &) = delete;

    bool connected() const { return m_handle != nullptr; }
    bool busy() const { return m_busy; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }

    /// Raw handle for _on_session FFI consumers.
    ::PierSshSession *handle() const { return m_handle; }

public slots:
    /// Open a shared SSH session asynchronously.
    void open(const QString &host, int port, const QString &user,
              int authKind, const QString &secret, const QString &extra);

    /// Release the session handle.
    void close();

signals:
    void connectedChanged();
    void busyChanged();

private slots:
    void onOpenResult(quint64 requestId, void *handle, const QString &error, const QString &target);

private:
    ::PierSshSession *m_handle = nullptr;
    bool m_busy = false;
    QString m_errorMessage;
    QString m_target;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::unique_ptr<std::thread> m_worker;
};
