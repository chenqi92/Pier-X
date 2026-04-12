// ─────────────────────────────────────────────────────────
// PierRedisClient — Qt-side Redis browser handle
// ─────────────────────────────────────────────────────────
//
// Thin QObject around an opaque PierRedis handle. Exposes the
// minimal surface the M5a Redis browser panel needs:
//
//   * async connect to host:port/db via connectTo(...)
//   * scanKeys(pattern, limit)   → keys model (QStringList)
//   * inspect(key)               → KeyDetails fields
//   * fetchInfo(section)         → info dict
//
// Threading
// ─────────
//   Every pier_redis_* call is blocking. We run them on a
//   dedicated std::thread per in-flight request, same pattern
//   as PierSftpBrowser / PierTunnelHandle. The worker captures
//   a QPointer<self> + a monotonically increasing request id;
//   a shared cancel flag drops stale deliveries if stop()
//   races with an in-flight call. Results are posted back on
//   the main thread via QMetaObject::invokeMethod.
//
// Why four distinct ops instead of one generic?
// ─────────────────────────────────────────────
//   Each op has its own result shape (list, map, struct) and
//   its own QML consumer (key list, detail pane, INFO pane).
//   Fusing them into one generic "run" slot would force the
//   QML side to branch on a tag anyway, and JSON parsing would
//   have to live in QML. Keeping them separate lets each slot
//   build its own strongly-typed properties.

#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QVariantMap>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

// Forward-declare the opaque Rust handle.
struct PierRedis;

class PierRedisClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierRedisClient)

public:
    enum Status {
        Idle = 0,
        Connecting = 1,
        Connected = 2,
        Failed = 3
    };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString target READ target NOTIFY statusChanged FINAL)
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)

    // Key list (from the last scan).
    Q_PROPERTY(QStringList keys READ keys NOTIFY keysChanged FINAL)
    Q_PROPERTY(bool keysTruncated READ keysTruncated NOTIFY keysChanged FINAL)
    Q_PROPERTY(int keysLimit READ keysLimit NOTIFY keysChanged FINAL)

    // Currently inspected key. All fields are updated together
    // via the keyDetailsChanged signal.
    Q_PROPERTY(QString selectedKey READ selectedKey NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(QString selectedKind READ selectedKind NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(qint64 selectedLength READ selectedLength NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(qint64 selectedTtl READ selectedTtl NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(QString selectedEncoding READ selectedEncoding NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(QStringList selectedPreview READ selectedPreview NOTIFY keyDetailsChanged FINAL)
    Q_PROPERTY(bool selectedPreviewTruncated READ selectedPreviewTruncated NOTIFY keyDetailsChanged FINAL)

    // Last INFO fetch result. Parsed as a flat k→v string map.
    Q_PROPERTY(QVariantMap serverInfo READ serverInfo NOTIFY serverInfoChanged FINAL)

    explicit PierRedisClient(QObject *parent = nullptr);
    ~PierRedisClient() override;

    PierRedisClient(const PierRedisClient &) = delete;
    PierRedisClient &operator=(const PierRedisClient &) = delete;

    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString target() const { return m_target; }
    bool busy() const { return m_busy; }

    QStringList keys() const { return m_keys; }
    bool keysTruncated() const { return m_keysTruncated; }
    int keysLimit() const { return m_keysLimit; }

    QString selectedKey() const { return m_selectedKey; }
    QString selectedKind() const { return m_selectedKind; }
    qint64 selectedLength() const { return m_selectedLength; }
    qint64 selectedTtl() const { return m_selectedTtl; }
    QString selectedEncoding() const { return m_selectedEncoding; }
    QStringList selectedPreview() const { return m_selectedPreview; }
    bool selectedPreviewTruncated() const { return m_selectedPreviewTruncated; }

    QVariantMap serverInfo() const { return m_serverInfo; }

public slots:
    /// Dial `host:port` at database `db` and PING. Async —
    /// watch `status` for Connected / Failed.
    bool connectTo(const QString &host, int port, int db);

    /// SCAN for keys matching `pattern` (glob syntax, default
    /// `*`). `limit` is clamped server-side to the Rust
    /// module's DEFAULT_SCAN_LIMIT.
    void scanKeys(const QString &pattern, int limit);

    /// Fetch type / ttl / bounded preview for a single key.
    /// Updates the `selected*` properties.
    void inspect(const QString &key);

    /// Run INFO <section>. Empty section = all sections.
    void fetchInfo(const QString &section);

    /// Shut down. Closes the handle and invalidates any
    /// in-flight result. Safe to call multiple times.
    void stop();

signals:
    void statusChanged();
    void busyChanged();
    void keysChanged();
    void keyDetailsChanged();
    void serverInfoChanged();

private slots:
    void onConnectResult(quint64 requestId, void *handle, const QString &error);
    void onScanResult(quint64 requestId, const QString &json, const QString &error);
    void onInspectResult(quint64 requestId, const QString &json, const QString &error);
    void onInfoResult(quint64 requestId, const QString &json, const QString &error);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void ingestScanJson(const QString &json);
    void ingestKeyDetailsJson(const QString &json);
    void ingestInfoJson(const QString &json);

    PierRedis *m_handle = nullptr;

    Status m_status = Status::Idle;
    QString m_errorMessage;
    QString m_target;
    bool m_busy = false;

    QStringList m_keys;
    bool m_keysTruncated = false;
    int m_keysLimit = 0;

    QString m_selectedKey;
    QString m_selectedKind;
    qint64 m_selectedLength = 0;
    qint64 m_selectedTtl = -2;
    QString m_selectedEncoding;
    QStringList m_selectedPreview;
    bool m_selectedPreviewTruncated = false;

    QVariantMap m_serverInfo;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
