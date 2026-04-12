#include "PierRedisClient.h"

#include "pier_redis.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

PierRedisClient::PierRedisClient(QObject *parent)
    : QObject(parent)
{
}

PierRedisClient::~PierRedisClient()
{
    stop();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

void PierRedisClient::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierRedisClient::setBusy(bool b)
{
    if (m_busy == b) return;
    m_busy = b;
    emit busyChanged();
}

bool PierRedisClient::connectTo(const QString &host, int port, int db)
{
    if (m_handle || m_status == Connecting) {
        qWarning() << "PierRedisClient::connectTo called on already-connected session";
        return false;
    }
    if (host.isEmpty() || port <= 0 || port > 65535) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    if (!m_cancelFlag) {
        m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    }
    m_errorMessage.clear();
    m_target = QStringLiteral("%1:%2/%3").arg(host).arg(port).arg(db);
    setStatus(Connecting);
    setBusy(true);

    std::string hostStd = host.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int64_t dbI64 = static_cast<int64_t>(db);

    QPointer<PierRedisClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        hostStd = std::move(hostStd),
        portU16, dbI64
    ]() mutable {
        PierRedis *h = pier_redis_open(hostStd.c_str(), portU16, dbI64);
        QString err;
        if (!h) {
            err = QStringLiteral("Redis connect failed (see log)");
        }

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_redis_free(h);
            return;
        }
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onConnectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

void PierRedisClient::onConnectResult(quint64 requestId, void *handle, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_redis_free(static_cast<PierRedis *>(handle));
        return;
    }
    setBusy(false);
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("Redis connect failed") : error;
        setStatus(Failed);
        return;
    }
    m_handle = static_cast<PierRedis *>(handle);
    setStatus(Connected);
    // Kick off an initial scan so the key list is populated
    // without the UI needing extra plumbing.
    scanKeys(QStringLiteral("*"), 500);
    fetchInfo(QStringLiteral("server"));
}

void PierRedisClient::scanKeys(const QString &pattern, int limit)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string pat = pattern.isEmpty() ? std::string("*") : pattern.toStdString();
    const size_t lim = static_cast<size_t>(limit > 0 ? limit : 500);
    PierRedis *h = m_handle;
    QPointer<PierRedisClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        pat = std::move(pat), lim
    ]() mutable {
        char *json = pier_redis_scan_keys(h, pat.c_str(), lim);
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_redis_free_string(json);
        } else {
            err = QStringLiteral("scan failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onScanResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierRedisClient::onScanResult(quint64 requestId, const QString &json, const QString &error)
{
    (void)requestId;
    setBusy(false);
    if (!error.isEmpty()) {
        m_errorMessage = error;
        emit statusChanged();
        return;
    }
    ingestScanJson(json);
}

void PierRedisClient::ingestScanJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        qWarning() << "PierRedisClient: malformed scan JSON:" << parseErr.errorString();
        return;
    }
    const QJsonObject obj = doc.object();
    const QJsonArray keysArr = obj.value(QStringLiteral("keys")).toArray();
    QStringList keys;
    keys.reserve(keysArr.size());
    for (const QJsonValue &v : keysArr) {
        keys.append(v.toString());
    }
    m_keys = std::move(keys);
    m_keysTruncated = obj.value(QStringLiteral("truncated")).toBool();
    m_keysLimit = obj.value(QStringLiteral("limit")).toInt();
    emit keysChanged();
}

void PierRedisClient::inspect(const QString &key)
{
    if (!m_handle || key.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string keyStd = key.toStdString();
    PierRedis *h = m_handle;
    QPointer<PierRedisClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        keyStd = std::move(keyStd)
    ]() mutable {
        char *json = pier_redis_inspect(h, keyStd.c_str());
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_redis_free_string(json);
        } else {
            err = QStringLiteral("inspect failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onInspectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierRedisClient::onInspectResult(quint64 requestId, const QString &json, const QString &error)
{
    (void)requestId;
    setBusy(false);
    if (!error.isEmpty()) {
        m_errorMessage = error;
        emit statusChanged();
        return;
    }
    ingestKeyDetailsJson(json);
}

void PierRedisClient::ingestKeyDetailsJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        qWarning() << "PierRedisClient: malformed inspect JSON:" << parseErr.errorString();
        return;
    }
    const QJsonObject obj = doc.object();
    m_selectedKey = obj.value(QStringLiteral("key")).toString();
    m_selectedKind = obj.value(QStringLiteral("kind")).toString();
    m_selectedLength = static_cast<qint64>(obj.value(QStringLiteral("length")).toDouble());
    m_selectedTtl = static_cast<qint64>(obj.value(QStringLiteral("ttl_seconds")).toDouble());
    m_selectedEncoding = obj.value(QStringLiteral("encoding")).toString();

    QStringList preview;
    const QJsonArray arr = obj.value(QStringLiteral("preview")).toArray();
    preview.reserve(arr.size());
    for (const QJsonValue &v : arr) {
        preview.append(v.toString());
    }
    m_selectedPreview = std::move(preview);
    m_selectedPreviewTruncated = obj.value(QStringLiteral("preview_truncated")).toBool();
    emit keyDetailsChanged();
}

void PierRedisClient::fetchInfo(const QString &section)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    std::string sectionStd = section.toStdString();
    PierRedis *h = m_handle;
    QPointer<PierRedisClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        sectionStd = std::move(sectionStd)
    ]() mutable {
        const char *sectionPtr = sectionStd.empty() ? nullptr : sectionStd.c_str();
        char *json = pier_redis_info(h, sectionPtr);
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_redis_free_string(json);
        } else {
            err = QStringLiteral("info failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onInfoResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierRedisClient::onInfoResult(quint64 requestId, const QString &json, const QString &error)
{
    (void)requestId;
    setBusy(false);
    if (!error.isEmpty()) {
        m_errorMessage = error;
        emit statusChanged();
        return;
    }
    ingestInfoJson(json);
}

void PierRedisClient::ingestInfoJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        qWarning() << "PierRedisClient: malformed info JSON:" << parseErr.errorString();
        return;
    }
    const QJsonObject obj = doc.object();
    QVariantMap info;
    for (auto it = obj.constBegin(); it != obj.constEnd(); ++it) {
        info.insert(it.key(), it.value().toString());
    }
    m_serverInfo = std::move(info);
    emit serverInfoChanged();
}

void PierRedisClient::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    if (m_handle) {
        PierRedis *h = m_handle;
        m_handle = nullptr;
        pier_redis_free(h);
    }
    if (!m_keys.isEmpty()) {
        m_keys.clear();
        m_keysTruncated = false;
        m_keysLimit = 0;
        emit keysChanged();
    }
    if (!m_selectedKey.isEmpty()) {
        m_selectedKey.clear();
        m_selectedKind.clear();
        m_selectedLength = 0;
        m_selectedTtl = -2;
        m_selectedEncoding.clear();
        m_selectedPreview.clear();
        m_selectedPreviewTruncated = false;
        emit keyDetailsChanged();
    }
    if (!m_serverInfo.isEmpty()) {
        m_serverInfo.clear();
        emit serverInfoChanged();
    }
    if (m_status != Idle) {
        m_errorMessage.clear();
        m_target.clear();
        setStatus(Idle);
    }
    setBusy(false);
}
