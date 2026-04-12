#include "PierDockerClient.h"

#include "pier_docker.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

PierDockerClientModel::PierDockerClientModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

PierDockerClientModel::~PierDockerClientModel()
{
    stop();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

int PierDockerClientModel::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return static_cast<int>(m_rows.size());
}

QVariant PierDockerClientModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int row = index.row();
    if (row < 0 || row >= static_cast<int>(m_rows.size())) return {};
    const Row &r = m_rows[static_cast<size_t>(row)];
    switch (role) {
    case IdRole:         return r.id;
    case ImageRole:      return r.image;
    case NamesRole:      return r.names;
    case StatusTextRole: return r.statusText;
    case StateRole:      return r.state;
    case IsRunningRole:  return r.isRunning;
    case CreatedRole:    return r.created;
    case PortsRole:      return r.ports;
    default:             return {};
    }
}

QHash<int, QByteArray> PierDockerClientModel::roleNames() const
{
    return {
        { IdRole,         "containerId" },
        { ImageRole,      "image" },
        { NamesRole,      "names" },
        { StatusTextRole, "statusText" },
        { StateRole,      "state" },
        { IsRunningRole,  "isRunning" },
        { CreatedRole,    "created" },
        { PortsRole,      "ports" }
    };
}

void PierDockerClientModel::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierDockerClientModel::setBusy(bool b)
{
    if (m_busy == b) return;
    m_busy = b;
    emit busyChanged();
}

void PierDockerClientModel::setInspectBusy(bool b)
{
    if (m_inspectBusy == b) return;
    m_inspectBusy = b;
    emit inspectStateChanged();
}

void PierDockerClientModel::setShowStopped(bool v)
{
    if (m_showStopped == v) return;
    m_showStopped = v;
    emit showStoppedChanged();
    if (m_handle) refresh();
}

bool PierDockerClientModel::connectTo(const QString &host, int port, const QString &user,
                                       int authKind, const QString &secret, const QString &extra)
{
    if (m_handle || m_status == Connecting) {
        qWarning() << "PierDockerClientModel::connectTo called on already-connected session";
        return false;
    }
    if (host.isEmpty() || user.isEmpty() || port <= 0 || port > 65535) {
        return false;
    }

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = QStringLiteral("%1@%2:%3").arg(user, host).arg(port);
    setStatus(Connecting);
    setBusy(true);

    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int kind = authKind;

    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        hostStd = std::move(hostStd),
        userStd = std::move(userStd),
        secretStd = std::move(secretStd),
        extraStd = std::move(extraStd),
        portU16, kind
    ]() mutable {
        const char *secretPtr = secretStd.empty() ? nullptr : secretStd.c_str();
        const char *extraPtr  = extraStd.empty()  ? nullptr : extraStd.c_str();

        ::PierDocker *h = pier_docker_open(
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            kind,
            secretPtr,
            extraPtr);

        QString err;
        if (!h) err = QStringLiteral("Docker connect failed (see log)");

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_docker_free(h);
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

void PierDockerClientModel::onConnectResult(quint64 requestId, void *handle, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_docker_free(static_cast<::PierDocker *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("Docker connect failed") : error;
        setStatus(Failed);
        setBusy(false);
        return;
    }
    m_handle = static_cast<::PierDocker *>(handle);
    setStatus(Connected);
    // Auto-load the initial listing.
    spawnList(++m_nextRequestId);
}

void PierDockerClientModel::refresh()
{
    if (!m_handle) return;
    spawnList(++m_nextRequestId);
}

void PierDockerClientModel::spawnList(quint64 requestId)
{
    if (!m_handle) return;
    setBusy(true);

    ::PierDocker *h = m_handle;
    const int all = m_showStopped ? 1 : 0;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h, all
    ]() mutable {
        char *json = pier_docker_list_containers(h, all);
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_docker_free_string(json);
        } else {
            err = QStringLiteral("docker ps failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onListResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierDockerClientModel::onListResult(quint64 requestId, const QString &json, const QString &error)
{
    (void)requestId;
    setBusy(false);
    if (!error.isEmpty()) {
        m_errorMessage = error;
        emit statusChanged();
        return;
    }
    ingestListJson(json);
}

void PierDockerClientModel::ingestListJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierDockerClientModel: malformed list JSON:" << parseErr.errorString();
        return;
    }
    beginResetModel();
    m_rows.clear();
    const QJsonArray arr = doc.array();
    m_rows.reserve(static_cast<size_t>(arr.size()));
    for (const QJsonValue &v : arr) {
        if (!v.isObject()) continue;
        const QJsonObject obj = v.toObject();
        Row r;
        r.id         = obj.value(QStringLiteral("id")).toString();
        r.image      = obj.value(QStringLiteral("image")).toString();
        r.names      = obj.value(QStringLiteral("names")).toString();
        r.statusText = obj.value(QStringLiteral("status")).toString();
        r.state      = obj.value(QStringLiteral("state")).toString();
        r.isRunning  = r.state.compare(QStringLiteral("running"), Qt::CaseInsensitive) == 0;
        r.created    = obj.value(QStringLiteral("created")).toString();
        r.ports      = obj.value(QStringLiteral("ports")).toString();
        m_rows.push_back(std::move(r));
    }
    endResetModel();
    emit containerCountChanged();
}

void PierDockerClientModel::start(const QString &id)
{
    spawnAction(++m_nextRequestId, QStringLiteral("start"), id, false);
}

void PierDockerClientModel::stopContainer(const QString &id)
{
    spawnAction(++m_nextRequestId, QStringLiteral("stop"), id, false);
}

void PierDockerClientModel::restart(const QString &id)
{
    spawnAction(++m_nextRequestId, QStringLiteral("restart"), id, false);
}

void PierDockerClientModel::remove(const QString &id, bool force)
{
    spawnAction(++m_nextRequestId, QStringLiteral("rm"), id, force);
}

void PierDockerClientModel::inspect(const QString &id)
{
    if (!m_handle || id.isEmpty()) return;

    const quint64 requestId = ++m_nextInspectRequestId;
    if (!m_cancelFlag) {
        m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    }

    m_inspectTarget = id;
    m_inspectError.clear();
    m_inspectJson.clear();
    emit inspectStateChanged();
    setInspectBusy(true);

    std::string idStd = id.toStdString();
    ::PierDocker *h = m_handle;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        idStd = std::move(idStd)
    ]() mutable {
        char *json = pier_docker_inspect_container(h, idStd.c_str());
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_docker_free_string(json);
        } else {
            err = QStringLiteral("docker inspect failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onInspectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QString::fromStdString(idStd)),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierDockerClientModel::clearInspect()
{
    const bool changed = !m_inspectTarget.isEmpty()
        || !m_inspectJson.isEmpty()
        || !m_inspectError.isEmpty()
        || m_inspectBusy;
    m_inspectTarget.clear();
    m_inspectJson.clear();
    m_inspectError.clear();
    setInspectBusy(false);
    if (changed) {
        emit inspectStateChanged();
    }
}

void PierDockerClientModel::spawnAction(quint64 requestId,
                                         const QString &verb,
                                         const QString &id,
                                         bool force)
{
    if (!m_handle || id.isEmpty()) return;
    setBusy(true);

    std::string idStd = id.toStdString();
    std::string verbStd = verb.toStdString();
    ::PierDocker *h = m_handle;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        verbStd = std::move(verbStd),
        idStd = std::move(idStd),
        force
    ]() mutable {
        int32_t rc = PIER_DOCKER_ERR_FAILED;
        if (verbStd == "start") {
            rc = pier_docker_start(h, idStd.c_str());
        } else if (verbStd == "stop") {
            rc = pier_docker_stop(h, idStd.c_str());
        } else if (verbStd == "restart") {
            rc = pier_docker_restart(h, idStd.c_str());
        } else if (verbStd == "rm") {
            rc = pier_docker_remove(h, idStd.c_str(), force ? 1 : 0);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;

        const bool ok = (rc == PIER_DOCKER_OK);
        QString msg;
        switch (rc) {
        case PIER_DOCKER_OK:
            msg = QStringLiteral("%1 %2 ok")
                      .arg(QString::fromStdString(verbStd),
                           QString::fromStdString(idStd).left(12));
            break;
        case PIER_DOCKER_ERR_NULL:
            msg = QStringLiteral("internal: null handle");
            break;
        case PIER_DOCKER_ERR_UTF8:
            msg = QStringLiteral("internal: bad UTF-8 id");
            break;
        case PIER_DOCKER_ERR_UNSAFE_ID:
            msg = QStringLiteral("refusing unsafe id");
            break;
        case PIER_DOCKER_ERR_FAILED:
        default:
            msg = QStringLiteral("docker %1 failed (see log)")
                      .arg(QString::fromStdString(verbStd));
            break;
        }
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onActionResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(bool, ok),
            Q_ARG(QString, msg));
    });
    m_workers.push_back(std::move(worker));
}

void PierDockerClientModel::onActionResult(quint64 requestId, bool ok, const QString &message)
{
    (void)requestId;
    setBusy(false);
    emit actionFinished(ok, message);
    if (ok) {
        refresh();
    }
}

void PierDockerClientModel::onInspectResult(
    quint64 requestId,
    const QString &id,
    const QString &json,
    const QString &error)
{
    if (requestId != m_nextInspectRequestId) {
        return;
    }

    setInspectBusy(false);
    m_inspectTarget = id;
    if (!error.isEmpty()) {
        m_inspectJson.clear();
        m_inspectError = error;
    } else {
        m_inspectError.clear();
        m_inspectJson = formatInspectJson(json);
    }
    emit inspectStateChanged();
}

QString PierDockerClientModel::formatInspectJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || doc.isNull()) {
        return json.trimmed();
    }
    return QString::fromUtf8(doc.toJson(QJsonDocument::Indented)).trimmed();
}

void PierDockerClientModel::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    if (m_handle) {
        ::PierDocker *h = m_handle;
        m_handle = nullptr;
        pier_docker_free(h);
    }
    if (!m_rows.empty()) {
        beginResetModel();
        m_rows.clear();
        endResetModel();
        emit containerCountChanged();
    }
    clearInspect();
    if (m_status != Idle) {
        m_errorMessage.clear();
        m_target.clear();
        setStatus(Idle);
    }
    setBusy(false);
}
