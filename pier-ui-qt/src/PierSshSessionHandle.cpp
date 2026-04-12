#include "PierSshSessionHandle.h"

#include "pier_ssh_session.h"

#include <QDebug>
#include <QMetaObject>
#include <QPointer>

PierSshSessionHandle::PierSshSessionHandle(QObject *parent)
    : QObject(parent)
{
}

PierSshSessionHandle::~PierSshSessionHandle()
{
    close();
    if (m_worker && m_worker->joinable())
        m_worker->detach();
}

void PierSshSessionHandle::open(const QString &host, int port,
                                 const QString &user, int authKind,
                                 const QString &secret, const QString &extra)
{
    if (m_handle || m_busy) {
        qWarning() << "PierSshSessionHandle::open called while already connected/busy";
        return;
    }
    if (host.isEmpty() || user.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_busy = true;
    emit busyChanged();

    const QString targetStr = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int32_t kind = static_cast<int32_t>(authKind);

    QPointer<PierSshSessionHandle> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    if (m_worker && m_worker->joinable())
        m_worker->detach();

    m_worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, targetStr,
        hostStd = std::move(hostStd),
        userStd = std::move(userStd),
        secretStd = std::move(secretStd),
        extraStd = std::move(extraStd),
        portU16, kind
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *extraPtr = extraStd.empty() ? nullptr : extraStd.c_str();

        ::PierSshSession *h = pier_ssh_session_open(
            hostStd.c_str(), portU16, userStd.c_str(),
            kind, secretPtr, extraPtr);

        QString err;
        if (!h) {
            const char *lastErr = pier_ssh_session_last_error();
            err = lastErr ? QString::fromUtf8(lastErr)
                          : QStringLiteral("SSH session open failed");
        }

        if (!selfWeak || (cancelFlag && cancelFlag->load())) {
            if (h) pier_ssh_session_free(h);
            return;
        }
        QMetaObject::invokeMethod(
            selfWeak.data(), "onOpenResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(QString, err),
            Q_ARG(QString, targetStr));
    });
}

void PierSshSessionHandle::onOpenResult(quint64 requestId, void *handle,
                                         const QString &error, const QString &target)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_ssh_session_free(static_cast<::PierSshSession *>(handle));
        return;
    }
    m_busy = false;
    emit busyChanged();

    if (!handle) {
        m_errorMessage = error;
        emit connectedChanged();
        return;
    }
    m_handle = static_cast<::PierSshSession *>(handle);
    m_target = target;
    m_errorMessage.clear();
    emit connectedChanged();
}

void PierSshSessionHandle::close()
{
    if (m_cancelFlag)
        m_cancelFlag->store(true);
    if (m_handle) {
        pier_ssh_session_free(m_handle);
        m_handle = nullptr;
    }
    m_errorMessage.clear();
    m_target.clear();
    m_busy = false;
    emit connectedChanged();
    emit busyChanged();
}
