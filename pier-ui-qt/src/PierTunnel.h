// ─────────────────────────────────────────────────────────
// PierTunnel — Qt wrapper for a single SSH local forward
// ─────────────────────────────────────────────────────────
//
// One QObject per active tunnel. The QML side instantiates
// one per service pill (lazily, only when the pill is
// clicked) and binds its `localPort` / `state` properties
// into the pill's tunnel badge.
//
// Async pattern mirrors PierSftpBrowser / PierServiceDetector:
// `open(...)` spawns a std::thread that calls the blocking
// pier_tunnel_open FFI. The result is posted back on the
// main thread via QMetaObject::invokeMethod(QueuedConnection).
// A shared cancel flag + bumped request id drops stale
// deliveries if `close()` races with `open()`.

#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>

struct PierTunnel;

class PierTunnelHandle : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierTunnel)

public:
    enum State {
        Idle = 0,
        Opening = 1,
        Open = 2,
        Failed = 3
    };
    Q_ENUM(State)

    Q_PROPERTY(State state READ state NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY stateChanged FINAL)
    Q_PROPERTY(int localPort READ localPort NOTIFY stateChanged FINAL)
    Q_PROPERTY(QString remoteHost READ remoteHost NOTIFY stateChanged FINAL)
    Q_PROPERTY(int remotePort READ remotePort NOTIFY stateChanged FINAL)

    explicit PierTunnelHandle(QObject *parent = nullptr);
    ~PierTunnelHandle() override;

    PierTunnelHandle(const PierTunnelHandle &) = delete;
    PierTunnelHandle &operator=(const PierTunnelHandle &) = delete;

    State state() const { return m_state; }
    QString errorMessage() const { return m_errorMessage; }
    int localPort() const { return m_localPort; }
    QString remoteHost() const { return m_remoteHost; }
    int remotePort() const { return m_remotePort; }

public slots:
    /// Open a new tunnel. If `localPort` is 0, the OS picks
    /// a free port; otherwise we try to bind exactly that.
    /// A common Pier-X convention is `10000 + remotePort`
    /// so MySQL's 3306 → 13306 and Redis's 6379 → 16379.
    bool open(const QString &host, int port, const QString &user,
              int authKind, const QString &secret, const QString &extra,
              int localPort, const QString &remoteHost, int remotePort);

    /// Close the tunnel and free the handle.
    void close();

signals:
    void stateChanged();

private slots:
    void onOpenResult(quint64 requestId, void *handle, int actualLocalPort, const QString &error);

private:
    void setState(State s);

    PierTunnel *m_handle = nullptr;
    State m_state = State::Idle;
    QString m_errorMessage;
    int m_localPort = 0;
    QString m_remoteHost;
    int m_remotePort = 0;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::unique_ptr<std::thread> m_worker;
};
