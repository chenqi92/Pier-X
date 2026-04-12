#include "PierServerMonitor.h"
#include "pier_server_monitor.h"

#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>

PierServerMonitorModel::PierServerMonitorModel(QObject *parent)
    : QObject(parent)
{
    m_pollTimer.setInterval(5000);
    m_pollTimer.setSingleShot(false);
    connect(&m_pollTimer, &QTimer::timeout, this, &PierServerMonitorModel::probeOnce);
}

PierServerMonitorModel::~PierServerMonitorModel()
{
    stop();
    for (auto &t : m_workers) { if (t && t->joinable()) t->detach(); }
}

void PierServerMonitorModel::setStatus(Status s)
{ if (m_status != s) { m_status = s; emit statusChanged(); } }

void PierServerMonitorModel::setBusy(bool b)
{ if (m_busy != b) { m_busy = b; emit busyChanged(); } }

bool PierServerMonitorModel::connectTo(const QString &host, int port, const QString &user,
                                        int authKind, const QString &secret, const QString &extra)
{
    if (m_handle || m_status == Connecting) return false;
    if (host.isEmpty() || user.isEmpty() || port <= 0 || port > 65535) return false;

    const quint64 rid = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    setStatus(Connecting);
    setBusy(true);

    std::string h = host.toStdString(), u = user.toStdString(),
                s = secret.toStdString(), e = extra.toStdString();
    const uint16_t pt = static_cast<uint16_t>(port);
    const int ak = authKind;
    QPointer<PierServerMonitorModel> self(this);
    auto cf = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([self, cf, rid,
        h=std::move(h), u=std::move(u), s=std::move(s), e=std::move(e), pt, ak
    ]() mutable {
        const char *sp = s.empty() ? nullptr : s.c_str();
        const char *ep = e.empty() ? nullptr : e.c_str();
        ::PierServerMonitor *handle = pier_server_monitor_open(
            h.c_str(), pt, u.c_str(), ak, sp, ep);
        QString err;
        if (!handle) err = QStringLiteral("Monitor connect failed (see log)");
        if (!self || (cf && cf->load())) { if (handle) pier_server_monitor_free(handle); return; }
        QMetaObject::invokeMethod(self.data(), "onConnectResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(void*, static_cast<void*>(handle)), Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

void PierServerMonitorModel::onConnectResult(quint64 rid, void *handle, const QString &error)
{
    if (rid != m_nextRequestId) { if (handle) pier_server_monitor_free(static_cast<::PierServerMonitor*>(handle)); return; }
    setBusy(false);
    if (!handle) { m_errorMessage = error.isEmpty() ? QStringLiteral("Monitor connect failed") : error; setStatus(Failed); return; }
    m_handle = static_cast<::PierServerMonitor*>(handle);
    setStatus(Connected);
    probeOnce();
    m_pollTimer.start();
}

void PierServerMonitorModel::probeOnce()
{
    if (!m_handle || m_busy) return;
    const quint64 rid = ++m_nextRequestId;
    setBusy(true);

    ::PierServerMonitor *h = m_handle;
    QPointer<PierServerMonitorModel> self(this);
    auto cf = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([self, cf, rid, h]() {
        char *json = pier_server_monitor_probe(h);
        QString js;
        if (json) { js = QString::fromUtf8(json); pier_server_monitor_free_string(json); }
        if (!self || (cf && cf->load())) return;
        QMetaObject::invokeMethod(self.data(), "onProbeResult", Qt::QueuedConnection,
            Q_ARG(quint64, rid), Q_ARG(QString, js));
    });
    m_workers.push_back(std::move(worker));
}

void PierServerMonitorModel::onProbeResult(quint64, const QString &json)
{
    setBusy(false);
    if (json.isEmpty()) return;
    ingestSnapshotJson(json);
}

void PierServerMonitorModel::ingestSnapshotJson(const QString &json)
{
    QJsonParseError pe{};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &pe);
    if (pe.error != QJsonParseError::NoError || !doc.isObject()) return;
    const QJsonObject o = doc.object();
    m_uptime = o.value(QStringLiteral("uptime")).toString();
    m_load1 = o.value(QStringLiteral("load_1")).toDouble(-1);
    m_load5 = o.value(QStringLiteral("load_5")).toDouble(-1);
    m_load15 = o.value(QStringLiteral("load_15")).toDouble(-1);
    m_memTotalMb = o.value(QStringLiteral("mem_total_mb")).toDouble(-1);
    m_memUsedMb = o.value(QStringLiteral("mem_used_mb")).toDouble(-1);
    m_memFreeMb = o.value(QStringLiteral("mem_free_mb")).toDouble(-1);
    m_swapTotalMb = o.value(QStringLiteral("swap_total_mb")).toDouble(-1);
    m_swapUsedMb = o.value(QStringLiteral("swap_used_mb")).toDouble(-1);
    m_diskTotal = o.value(QStringLiteral("disk_total")).toString();
    m_diskUsed = o.value(QStringLiteral("disk_used")).toString();
    m_diskAvail = o.value(QStringLiteral("disk_avail")).toString();
    m_diskUsePct = o.value(QStringLiteral("disk_use_pct")).toDouble(-1);
    m_cpuPct = o.value(QStringLiteral("cpu_pct")).toDouble(-1);
    emit snapshotChanged();
}

void PierServerMonitorModel::stop()
{
    if (m_cancelFlag) m_cancelFlag->store(true);
    ++m_nextRequestId;
    m_pollTimer.stop();
    if (m_handle) { pier_server_monitor_free(m_handle); m_handle = nullptr; }
    if (m_status != Idle) { m_errorMessage.clear(); m_target.clear(); setStatus(Idle); }
    setBusy(false);
}
