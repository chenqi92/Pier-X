// PierServerMonitor — Qt-side server resource dashboard (M7b)
#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QTimer>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

struct PierServerMonitor;

class PierServerMonitorModel : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierServerMonitor)

public:
    enum Status { Idle = 0, Connecting = 1, Connected = 2, Failed = 3 };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)

    // Snapshot fields — updated after each probe.
    Q_PROPERTY(QString uptime READ uptime NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double load1 READ load1 NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double load5 READ load5 NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double load15 READ load15 NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double memTotalMb READ memTotalMb NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double memUsedMb READ memUsedMb NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double memFreeMb READ memFreeMb NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double swapTotalMb READ swapTotalMb NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double swapUsedMb READ swapUsedMb NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(QString diskTotal READ diskTotal NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(QString diskUsed READ diskUsed NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(QString diskAvail READ diskAvail NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double diskUsePct READ diskUsePct NOTIFY snapshotChanged FINAL)
    Q_PROPERTY(double cpuPct READ cpuPct NOTIFY snapshotChanged FINAL)

    explicit PierServerMonitorModel(QObject *parent = nullptr);
    ~PierServerMonitorModel() override;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    bool busy() const { return m_busy; }

    QString uptime() const { return m_uptime; }
    double load1() const { return m_load1; }
    double load5() const { return m_load5; }
    double load15() const { return m_load15; }
    double memTotalMb() const { return m_memTotalMb; }
    double memUsedMb() const { return m_memUsedMb; }
    double memFreeMb() const { return m_memFreeMb; }
    double swapTotalMb() const { return m_swapTotalMb; }
    double swapUsedMb() const { return m_swapUsedMb; }
    QString diskTotal() const { return m_diskTotal; }
    QString diskUsed() const { return m_diskUsed; }
    QString diskAvail() const { return m_diskAvail; }
    double diskUsePct() const { return m_diskUsePct; }
    double cpuPct() const { return m_cpuPct; }

public slots:
    bool connectTo(const QString &host, int port, const QString &user,
                   int authKind, const QString &secret, const QString &extra);
    void probeOnce();
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void snapshotChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onProbeResult(quint64 requestId, const QString &json);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestSnapshotJson(const QString &json);

    ::PierServerMonitor *m_handle = nullptr;
    Status m_status = Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;
    QTimer m_pollTimer;

    QString m_uptime;
    double m_load1 = -1, m_load5 = -1, m_load15 = -1;
    double m_memTotalMb = -1, m_memUsedMb = -1, m_memFreeMb = -1;
    double m_swapTotalMb = -1, m_swapUsedMb = -1;
    QString m_diskTotal, m_diskUsed, m_diskAvail;
    double m_diskUsePct = -1;
    double m_cpuPct = -1;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
