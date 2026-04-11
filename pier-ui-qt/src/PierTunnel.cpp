#include "PierTunnel.h"

#include "pier_tunnel.h"

#include <QByteArray>
#include <QDebug>
#include <QMetaObject>

PierTunnelHandle::PierTunnelHandle(QObject *parent)
    : QObject(parent)
{
}

PierTunnelHandle::~PierTunnelHandle()
{
    close();
    if (m_worker && m_worker->joinable()) {
        m_worker->detach();
    }
}

void PierTunnelHandle::setState(State s)
{
    if (m_state == s) return;
    m_state = s;
    emit stateChanged();
}

bool PierTunnelHandle::open(const QString &host, int port, const QString &user,
                             int authKind, const QString &secret, const QString &extra,
                             int localPort, const QString &remoteHost, int remotePort)
{
    if (m_state == Opening || m_state == Open) {
        qWarning() << "PierTunnelHandle::open called on already-open tunnel";
        return false;
    }
    if (host.isEmpty() || user.isEmpty() || remoteHost.isEmpty()
        || port <= 0 || port > 65535
        || localPort < 0 || localPort > 65535
        || remotePort <= 0 || remotePort > 65535) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_remoteHost = remoteHost;
    m_remotePort = remotePort;
    m_localPort = localPort;
    setState(Opening);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    std::string remoteHostStd = remoteHost.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const uint16_t localU16 = static_cast<uint16_t>(localPort);
    const uint16_t remoteU16 = static_cast<uint16_t>(remotePort);
    const int kind = authKind;

    QPointer<PierTunnelHandle> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    if (m_worker && m_worker->joinable()) {
        m_worker->detach();
    }
    m_worker.reset();

    m_worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        hostStd = std::move(hostStd),
        userStd = std::move(userStd),
        secretStd = std::move(secretStd),
        extraStd = std::move(extraStd),
        remoteHostStd = std::move(remoteHostStd),
        portU16, localU16, remoteU16, kind
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *extraPtr  = extraStd.empty()  ? nullptr : extraStd.c_str();

        PierTunnel *h = pier_tunnel_open(
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            kind,
            secretPtr,
            extraPtr,
            localU16,
            remoteHostStd.c_str(),
            remoteU16);

        int actualPort = 0;
        QString err;
        if (h) {
            actualPort = pier_tunnel_local_port(h);
        } else {
            err = QStringLiteral("tunnel open failed (see log)");
        }

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_tunnel_free(h);
            return;
        }

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpenResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(int, actualPort),
            Q_ARG(QString, err));
    });
    return true;
}

void PierTunnelHandle::onOpenResult(quint64 requestId, void *handle, int actualLocalPort, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_tunnel_free(static_cast<PierTunnel *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("tunnel failed") : error;
        setState(Failed);
        return;
    }
    m_handle = static_cast<PierTunnel *>(handle);
    m_localPort = actualLocalPort;
    setState(Open);
}

void PierTunnelHandle::close()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    if (m_handle) {
        PierTunnel *h = m_handle;
        m_handle = nullptr;
        pier_tunnel_free(h);
    }
    if (m_state != Idle) {
        m_errorMessage.clear();
        m_remoteHost.clear();
        m_remotePort = 0;
        m_localPort = 0;
        setState(Idle);
    }
}
