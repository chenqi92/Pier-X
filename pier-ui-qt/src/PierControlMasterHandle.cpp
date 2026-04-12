#include "PierControlMasterHandle.h"
#include "pier_control_master.h"

#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>

PierControlMasterHandle::PierControlMasterHandle(QObject *parent)
    : QObject(parent)
{
}

PierControlMasterHandle::~PierControlMasterHandle()
{
    close();
    if (m_worker && m_worker->joinable())
        m_worker->detach();
}

void PierControlMasterHandle::connectTo(const QString &host, int port, const QString &user)
{
    if (m_connected || m_busy) return;
    if (host.isEmpty() || user.isEmpty()) return;

    m_busy = true;
    emit busyChanged();
    m_target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    uint16_t portU16 = static_cast<uint16_t>(port);

    // Create the handle on the main thread (fast, no I/O)
    if (m_handle) {
        pier_control_master_free(m_handle);
        m_handle = nullptr;
    }
    m_handle = pier_control_master_new(hostStd.c_str(), portU16, userStd.c_str());
    if (!m_handle) {
        m_busy = false;
        emit busyChanged();
        return;
    }

    // Connect on a worker thread (waits for socket, may spawn master)
    QPointer<PierControlMasterHandle> self(this);
    ::PierControlMaster *h = m_handle;

    if (m_worker && m_worker->joinable())
        m_worker->detach();

    m_worker = std::make_unique<std::thread>([self, h]() {
        bool ok = (pier_control_master_connect(h, 15) != 0);
        if (!self) return;
        QMetaObject::invokeMethod(self.data(), "onConnectResult",
            Qt::QueuedConnection, Q_ARG(bool, ok));
    });
}

void PierControlMasterHandle::onConnectResult(bool ok)
{
    m_busy = false;
    m_connected = ok;
    emit busyChanged();
    emit connectedChanged();
    if (!ok) {
        qWarning() << "PierControlMasterHandle: connect failed for" << m_target;
    }
}

QString PierControlMasterHandle::exec(const QString &command)
{
    if (!m_handle || !m_connected) return {};

    std::string cmdStd = command.toStdString();
    char *json = pier_control_master_exec(m_handle, cmdStd.c_str());
    if (!json) return {};

    QString result = QString::fromUtf8(json);
    pier_control_master_free_string(json);

    QJsonDocument doc = QJsonDocument::fromJson(result.toUtf8());
    if (doc.isObject()) {
        return doc.object().value(QStringLiteral("stdout")).toString();
    }
    return result;
}

void PierControlMasterHandle::close()
{
    if (m_handle) {
        pier_control_master_free(m_handle);
        m_handle = nullptr;
    }
    m_connected = false;
    m_target.clear();
    emit connectedChanged();
}
