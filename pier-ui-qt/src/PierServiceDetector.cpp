#include "PierServiceDetector.h"

#include "pier_services.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

PierServiceDetector::PierServiceDetector(QObject *parent)
    : QAbstractListModel(parent)
{
}

PierServiceDetector::~PierServiceDetector()
{
    cancel();
    if (m_worker && m_worker->joinable()) {
        m_worker->detach();
    }
}

int PierServiceDetector::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return static_cast<int>(m_entries.size());
}

QVariant PierServiceDetector::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int row = index.row();
    if (row < 0 || row >= static_cast<int>(m_entries.size())) return {};
    const Entry &e = m_entries[static_cast<size_t>(row)];
    switch (role) {
    case NameRole:    return e.name;
    case VersionRole: return e.version;
    case StatusRole:  return e.status;
    case PortRole:    return e.port;
    default:          return {};
    }
}

QHash<int, QByteArray> PierServiceDetector::roleNames() const
{
    return {
        { NameRole,    "name" },
        { VersionRole, "version" },
        { StatusRole,  "status" },
        { PortRole,    "port" }
    };
}

void PierServiceDetector::setState(State s)
{
    if (m_state == s) return;
    m_state = s;
    emit stateChanged();
}

bool PierServiceDetector::detect(const QString &host, int port, const QString &user,
                                  int authKind, const QString &secret, const QString &extra)
{
    if (m_state == Running) {
        qWarning() << "PierServiceDetector::detect called while already running";
        return false;
    }
    if (host.isEmpty() || user.isEmpty() || port <= 0 || port > 65535) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    setState(Running);

    // Clear the existing model state so the UI doesn't show
    // stale pills while the new detection is in flight.
    beginResetModel();
    m_entries.clear();
    endResetModel();
    emit countChanged();

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int kind = authKind;

    QPointer<PierServiceDetector> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    // Detach any previous worker so the unique_ptr reset
    // below doesn't trip the "std::thread destroyed while
    // still joinable" assertion.
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
        portU16, kind
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *extraPtr  = extraStd.empty()  ? nullptr : extraStd.c_str();

        char *json = pier_services_detect(
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            kind,
            secretPtr,
            extraPtr);

        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_services_free_json(json);
        } else {
            err = QStringLiteral("detect failed (see log)");
        }

        if (!selfWeak || (cancelFlag && cancelFlag->load())) {
            return;
        }

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onDetectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    return true;
}

void PierServiceDetector::cancel()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    if (m_state == Running || m_state == Done || m_state == Failed) {
        beginResetModel();
        m_entries.clear();
        endResetModel();
        emit countChanged();
        m_errorMessage.clear();
        setState(Idle);
    }
}

void PierServiceDetector::onDetectResult(quint64 requestId, const QString &json, const QString &error)
{
    if (requestId != m_nextRequestId) {
        return;
    }
    if (!error.isEmpty()) {
        m_errorMessage = error;
        setState(Failed);
        return;
    }
    ingestJson(json);
    setState(Done);
}

void PierServiceDetector::ingestJson(const QString &json)
{
    QJsonParseError err {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &err);
    if (err.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierServiceDetector JSON parse failed:" << err.errorString();
        m_errorMessage = QStringLiteral("malformed service JSON");
        return;
    }

    beginResetModel();
    m_entries.clear();
    const QJsonArray arr = doc.array();
    m_entries.reserve(static_cast<size_t>(arr.size()));
    for (const QJsonValue &v : arr) {
        if (!v.isObject()) continue;
        const QJsonObject obj = v.toObject();
        Entry e;
        e.name    = obj.value(QStringLiteral("name")).toString();
        e.version = obj.value(QStringLiteral("version")).toString();
        e.status  = obj.value(QStringLiteral("status")).toString();
        e.port    = obj.value(QStringLiteral("port")).toInt();
        m_entries.push_back(std::move(e));
    }
    endResetModel();
    emit countChanged();
}
