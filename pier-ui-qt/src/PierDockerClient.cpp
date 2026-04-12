#include "PierDockerClient.h"

#include "pier_docker.h"
#include "pier_local.h"
#include "PierSshSessionHandle.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>
#include <QProcess>
#include <QRegularExpression>

namespace {

struct DockerExecPayload {
    bool ok = false;
    QString output;
    QString error;
};

DockerExecPayload decodeDockerExecResponse(const QString &json, const QString &fallbackError)
{
    DockerExecPayload payload;
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isObject()) {
        payload.error = fallbackError;
        return payload;
    }
    const QJsonObject obj = doc.object();
    payload.ok = obj.value(QStringLiteral("ok")).toBool(false);
    payload.output = obj.value(QStringLiteral("output")).toString();
    if (!payload.ok) {
        const QString trimmed = payload.output.trimmed();
        payload.error = trimmed.isEmpty() ? fallbackError : trimmed;
    }
    return payload;
}

QStringList splitCommandWords(const QString &command)
{
    if (command.trimmed().isEmpty()) {
        return {};
    }
    return QProcess::splitCommand(command);
}

QString composePsFormat()
{
    return QStringLiteral("{{.Service}}|||{{.Status}}|||{{.State}}|||{{.Image}}|||{{.Ports}}");
}

QString readJsonString(const QJsonObject &obj, const QString &key, const QString &legacyKey = QString())
{
    const QJsonValue primary = obj.value(key);
    if (!primary.isUndefined() && !primary.isNull()) {
        return primary.toVariant().toString();
    }
    if (!legacyKey.isEmpty()) {
        const QJsonValue legacy = obj.value(legacyKey);
        if (!legacy.isUndefined() && !legacy.isNull()) {
            return legacy.toVariant().toString();
        }
    }
    return {};
}

} // namespace

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
    if (m_handle || m_localMode) refresh();
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

bool PierDockerClientModel::connectToSession(QObject *sessionObj)
{
    if (m_handle || m_status == Connecting) return false;
    auto *sh = qobject_cast<PierSshSessionHandle *>(sessionObj);
    if (!sh || !sh->handle()) return false;

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = sh->target();
    setStatus(Connecting);
    setBusy(true);

    ::PierSshSession *session = sh->handle();
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, session
    ]() {
        ::PierDocker *h = pier_docker_open_on_session(session);
        QString err;
        if (!h) err = QStringLiteral("Docker open_on_session failed");
        if (!selfWeak || (cancelFlag && cancelFlag->load())) {
            if (h) pier_docker_free(h);
            return;
        }
        QMetaObject::invokeMethod(selfWeak.data(), "onConnectResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(void *, static_cast<void *>(h)), Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

bool PierDockerClientModel::connectLocal()
{
    if (m_handle || m_localMode || m_status == Connecting) return false;

    m_localMode = true;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    m_target = QStringLiteral("localhost");
    setStatus(Connected);
    spawnList(++m_nextRequestId);
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
    if (!m_handle && !m_localMode) return;
    spawnList(++m_nextRequestId);
}

void PierDockerClientModel::spawnList(quint64 requestId)
{
    if (!m_handle && !m_localMode) return;
    setBusy(true);

    ::PierDocker *h = m_handle;
    const int all = m_showStopped ? 1 : 0;
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h, all, local
    ]() mutable {
        char *json = local
            ? pier_local_docker_list_containers(all)
            : pier_docker_list_containers(h, all);
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            if (local) pier_local_free_string(json);
            else pier_docker_free_string(json);
        } else {
            err = QStringLiteral("docker ps failed");
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
        r.id         = readJsonString(obj, QStringLiteral("id"), QStringLiteral("ID"));
        r.image      = readJsonString(obj, QStringLiteral("image"), QStringLiteral("Image"));
        r.names      = readJsonString(obj, QStringLiteral("names"), QStringLiteral("Names"));
        r.statusText = readJsonString(obj, QStringLiteral("status"), QStringLiteral("Status"));
        r.state      = readJsonString(obj, QStringLiteral("state"), QStringLiteral("State"));
        r.isRunning  = r.state.compare(QStringLiteral("running"), Qt::CaseInsensitive) == 0;
        r.created    = readJsonString(obj, QStringLiteral("created"), QStringLiteral("CreatedAt"));
        r.ports      = readJsonString(obj, QStringLiteral("ports"), QStringLiteral("Ports"));
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
    if ((!m_handle && !m_localMode) || id.isEmpty()) return;

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
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h, local,
        idStd = std::move(idStd)
    ]() mutable {
        char *json = local
            ? pier_local_docker_inspect(idStd.c_str())
            : pier_docker_inspect_container(h, idStd.c_str());
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
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

void PierDockerClientModel::inspectImage(const QString &id)
{
    if (id.isEmpty()) return;
    spawnInspectExec(++m_nextInspectRequestId, id,
                     QStringList{ QStringLiteral("inspect"), QStringLiteral("--type"), QStringLiteral("image"), id });
}

void PierDockerClientModel::inspectVolume(const QString &name)
{
    if (name.isEmpty()) return;
    spawnInspectExec(++m_nextInspectRequestId, name,
                     QStringList{ QStringLiteral("volume"), QStringLiteral("inspect"), name });
}

void PierDockerClientModel::inspectNetwork(const QString &name)
{
    if (name.isEmpty()) return;
    spawnInspectExec(++m_nextInspectRequestId, name,
                     QStringList{ QStringLiteral("network"), QStringLiteral("inspect"), name });
}

void PierDockerClientModel::spawnAction(quint64 requestId,
                                         const QString &verb,
                                         const QString &id,
                                         bool force)
{
    if ((!m_handle && !m_localMode) || id.isEmpty()) return;
    setBusy(true);

    std::string idStd = id.toStdString();
    std::string verbStd = verb.toStdString();
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h, local,
        verbStd = std::move(verbStd),
        idStd = std::move(idStd),
        force
    ]() mutable {
        int32_t rc = PIER_DOCKER_ERR_FAILED;
        if (local) {
            rc = pier_local_docker_action(verbStd.c_str(), idStd.c_str(), force ? 1 : 0) == 0
                 ? PIER_DOCKER_OK : PIER_DOCKER_ERR_FAILED;
        } else if (verbStd == "start") {
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

void PierDockerClientModel::onInspectExecResult(
    quint64 requestId,
    const QString &target,
    bool ok,
    const QString &output,
    const QString &error)
{
    if (requestId != m_nextInspectRequestId) {
        return;
    }
    setInspectBusy(false);
    m_inspectTarget = target;
    if (!ok) {
        m_inspectJson.clear();
        m_inspectError = error;
    } else {
        m_inspectError.clear();
        m_inspectJson = formatInspectJson(output);
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

void PierDockerClientModel::spawnInspectExec(
    quint64 requestId,
    const QString &target,
    const QStringList &args)
{
    if ((!m_handle && !m_localMode) || args.isEmpty()) return;
    if (!m_cancelFlag) {
        m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    }

    m_inspectTarget = target;
    m_inspectError.clear();
    m_inspectJson.clear();
    emit inspectStateChanged();
    setInspectBusy(true);

    const QByteArray argsJson = QJsonDocument(QJsonArray::fromStringList(args)).toJson(QJsonDocument::Compact);
    const std::string argsStd = argsJson.toStdString();
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;

    auto w = std::make_unique<std::thread>([self, cancel, requestId, h, local,
                                            target, argsStd = std::move(argsStd)]() {
        char *json = local
            ? pier_local_docker_exec_json(argsStd.c_str())
            : pier_docker_exec_json(h, argsStd.c_str());
        QString response;
        if (json) {
            response = QString::fromUtf8(json);
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        const DockerExecPayload payload = response.isEmpty()
            ? DockerExecPayload{ false, QString(), QStringLiteral("docker command failed (see log)") }
            : decodeDockerExecResponse(response, QStringLiteral("docker command failed (see log)"));
        QMetaObject::invokeMethod(
            self.data(),
            "onInspectExecResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, target),
            Q_ARG(bool, payload.ok),
            Q_ARG(QString, payload.output),
            Q_ARG(QString, payload.error));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::spawnDockerExec(
    quint64 requestId,
    const QString &op,
    const QStringList &args)
{
    if ((!m_handle && !m_localMode) || args.isEmpty()) return;
    setBusy(true);

    const QByteArray argsJson = QJsonDocument(QJsonArray::fromStringList(args)).toJson(QJsonDocument::Compact);
    const std::string argsStd = argsJson.toStdString();
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;

    auto w = std::make_unique<std::thread>([self, cancel, requestId, h, local, op,
                                            argsStd = std::move(argsStd)]() {
        char *json = local
            ? pier_local_docker_exec_json(argsStd.c_str())
            : pier_docker_exec_json(h, argsStd.c_str());
        QString response;
        if (json) {
            response = QString::fromUtf8(json);
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        const DockerExecPayload payload = response.isEmpty()
            ? DockerExecPayload{ false, QString(), QStringLiteral("docker command failed (see log)") }
            : decodeDockerExecResponse(response, QStringLiteral("docker command failed (see log)"));
        QMetaObject::invokeMethod(
            self.data(),
            "onDockerExecResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, op),
            Q_ARG(bool, payload.ok),
            Q_ARG(QString, payload.output),
            Q_ARG(QString, payload.error));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::onDockerExecResult(
    quint64 requestId,
    const QString &op,
    bool ok,
    const QString &output,
    const QString &error)
{
    if (requestId != m_nextRequestId) {
        return;
    }

    setBusy(false);

    QString message = error;
    if (ok) {
        if (op.startsWith(QStringLiteral("composeUp::"))) {
            const QString composeFilePath = op.section(QStringLiteral("::"), 1);
            emit actionFinished(true, QStringLiteral("Compose stack started"));
            refreshCompose(composeFilePath);
            return;
        }
        if (op.startsWith(QStringLiteral("composeDown::"))) {
            const QString composeFilePath = op.section(QStringLiteral("::"), 1);
            emit actionFinished(true, QStringLiteral("Compose stack stopped"));
            refreshCompose(composeFilePath);
            return;
        }
        if (op.startsWith(QStringLiteral("composeRestart::"))) {
            const QString composeFilePath = op.section(QStringLiteral("::"), 1, 1);
            emit actionFinished(true, QStringLiteral("Compose service restarted"));
            refreshCompose(composeFilePath);
            return;
        }
        if (op == QStringLiteral("pullImage")) {
            message = QStringLiteral("Image pulled");
            emit actionFinished(true, message);
            refreshImages();
            return;
        }
        if (op == QStringLiteral("pruneImages")) {
            message = QStringLiteral("Unused images pruned");
            emit actionFinished(true, message);
            refreshImages();
            return;
        }
        if (op == QStringLiteral("runImage")) {
            message = QStringLiteral("Container created");
            emit actionFinished(true, message);
            refresh();
            return;
        }
        if (op == QStringLiteral("removeImage")) {
            message = QStringLiteral("Image removed");
            emit actionFinished(true, message);
            refreshImages();
            return;
        }
        if (op == QStringLiteral("pruneVolumes")) {
            message = QStringLiteral("Unused volumes pruned");
            emit actionFinished(true, message);
            refreshVolumes();
            return;
        }
        if (op == QStringLiteral("removeVolume")) {
            message = QStringLiteral("Volume removed");
            emit actionFinished(true, message);
            refreshVolumes();
            return;
        }
        if (op == QStringLiteral("createNetwork")) {
            message = QStringLiteral("Network created");
            emit actionFinished(true, message);
            refreshNetworks();
            return;
        }
        if (op == QStringLiteral("removeNetwork")) {
            message = QStringLiteral("Network removed");
            emit actionFinished(true, message);
            refreshNetworks();
            return;
        }
        message = output.trimmed();
        if (message.isEmpty()) {
            message = QStringLiteral("Docker command completed");
        }
    }

    emit actionFinished(ok, message);
}

void PierDockerClientModel::refreshCompose(const QString &composeFilePath)
{
    if ((!m_handle && !m_localMode) || composeFilePath.trimmed().isEmpty()) {
        if (!m_composeServices.isEmpty()) {
            m_composeServices.clear();
            emit composeServicesChanged();
        }
        return;
    }

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    const QString path = composeFilePath.trimmed();
    const QByteArray argsJson = QJsonDocument(QJsonArray::fromStringList(
        QStringList{
            QStringLiteral("compose"),
            QStringLiteral("-f"),
            path,
            QStringLiteral("ps"),
            QStringLiteral("--format"),
            composePsFormat()
        }
    )).toJson(QJsonDocument::Compact);
    const std::string argsStd = argsJson.toStdString();
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;

    auto w = std::make_unique<std::thread>([self, cancel, requestId, h, local,
                                            argsStd = std::move(argsStd)]() {
        char *json = local
            ? pier_local_docker_exec_json(argsStd.c_str())
            : pier_docker_exec_json(h, argsStd.c_str());
        QString response;
        if (json) {
            response = QString::fromUtf8(json);
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        const DockerExecPayload payload = response.isEmpty()
            ? DockerExecPayload{ false, QString(), QStringLiteral("docker compose ps failed") }
            : decodeDockerExecResponse(response, QStringLiteral("docker compose ps failed"));
        QMetaObject::invokeMethod(
            self.data(),
            "onComposeResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, payload.output),
            Q_ARG(QString, payload.ok ? QString() : payload.error));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::composeUp(const QString &composeFilePath)
{
    const QString path = composeFilePath.trimmed();
    if (path.isEmpty()) return;
    spawnDockerExec(++m_nextRequestId,
                    QStringLiteral("composeUp::") + path,
                    QStringList{
                        QStringLiteral("compose"),
                        QStringLiteral("-f"),
                        path,
                        QStringLiteral("up"),
                        QStringLiteral("-d")
                    });
}

void PierDockerClientModel::composeDown(const QString &composeFilePath)
{
    const QString path = composeFilePath.trimmed();
    if (path.isEmpty()) return;
    spawnDockerExec(++m_nextRequestId,
                    QStringLiteral("composeDown::") + path,
                    QStringList{
                        QStringLiteral("compose"),
                        QStringLiteral("-f"),
                        path,
                        QStringLiteral("down")
                    });
}

void PierDockerClientModel::composeRestart(const QString &composeFilePath, const QString &service)
{
    const QString path = composeFilePath.trimmed();
    if (path.isEmpty()) return;
    QStringList args{
        QStringLiteral("compose"),
        QStringLiteral("-f"),
        path,
        QStringLiteral("restart")
    };
    const QString trimmedService = service.trimmed();
    if (!trimmedService.isEmpty()) {
        args << trimmedService;
    }
    spawnDockerExec(++m_nextRequestId,
                    QStringLiteral("composeRestart::") + path + QStringLiteral("::") + trimmedService,
                    args);
}

void PierDockerClientModel::onComposeResult(quint64 requestId, const QString &output, const QString &error)
{
    if (requestId != m_nextRequestId) {
        return;
    }

    setBusy(false);

    if (!error.isEmpty()) {
        if (!m_composeServices.isEmpty()) {
            m_composeServices.clear();
            emit composeServicesChanged();
        }
        emit actionFinished(false, error);
        return;
    }

    QVariantList list;
    const QStringList lines = output.split(QRegularExpression(QStringLiteral("[\r\n]+")),
                                           Qt::SkipEmptyParts);
    for (const QString &rawLine : lines) {
        const QStringList parts = rawLine.split(QStringLiteral("|||"));
        if (parts.size() < 3) continue;
        QVariantMap row;
        const QString service = parts.value(0).trimmed();
        const QString status = parts.value(1).trimmed();
        const QString state = parts.value(2).trimmed();
        row.insert(QStringLiteral("service"), service);
        row.insert(QStringLiteral("name"), service);
        row.insert(QStringLiteral("status"), status);
        row.insert(QStringLiteral("state"), state);
        row.insert(QStringLiteral("isRunning"), state.compare(QStringLiteral("running"), Qt::CaseInsensitive) == 0);
        row.insert(QStringLiteral("image"), parts.value(3).trimmed());
        row.insert(QStringLiteral("ports"), parts.value(4).trimmed());
        list.append(row);
    }

    m_composeServices = list;
    emit composeServicesChanged();
}

// ─── Images ─────────────────────────────────────────────

void PierDockerClientModel::refreshImages()
{
    if (!m_handle && !m_localMode) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    auto w = std::make_unique<std::thread>([self, cancel, id, h, local]() {
        char *json = local ? pier_local_docker_list_images() : pier_docker_list_images(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) {
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onImagesResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::onImagesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const auto &v : doc.array()) {
            if (!v.isObject()) continue;
            const QJsonObject obj = v.toObject();
            QVariantMap map;
            map.insert(QStringLiteral("id"), readJsonString(obj, QStringLiteral("id"), QStringLiteral("ID")));
            map.insert(QStringLiteral("repository"), readJsonString(obj, QStringLiteral("repository"), QStringLiteral("Repository")));
            map.insert(QStringLiteral("tag"), readJsonString(obj, QStringLiteral("tag"), QStringLiteral("Tag")));
            map.insert(QStringLiteral("size"), readJsonString(obj, QStringLiteral("size"), QStringLiteral("Size")));
            map.insert(QStringLiteral("created"), readJsonString(obj, QStringLiteral("created"), QStringLiteral("CreatedAt")));
            list.append(map);
        }
    }
    m_images = list;
    emit imagesChanged();
    setBusy(false);
}

void PierDockerClientModel::pullImage(const QString &imageRef)
{
    if (imageRef.trimmed().isEmpty()) return;
    spawnDockerExec(++m_nextRequestId, QStringLiteral("pullImage"),
                    QStringList{ QStringLiteral("pull"), imageRef.trimmed() });
}

void PierDockerClientModel::pruneImages()
{
    spawnDockerExec(++m_nextRequestId, QStringLiteral("pruneImages"),
                    QStringList{ QStringLiteral("image"), QStringLiteral("prune"), QStringLiteral("-f") });
}

void PierDockerClientModel::runImage(
    const QString &imageRef,
    const QString &containerName,
    const QVariantList &ports,
    const QVariantList &envVars,
    const QVariantList &volumes,
    const QString &restartPolicy,
    const QString &command,
    bool detached)
{
    const QString image = imageRef.trimmed();
    if (image.isEmpty()) return;

    QStringList args{ QStringLiteral("run") };
    if (detached) {
        args << QStringLiteral("-d");
    }
    if (!containerName.trimmed().isEmpty()) {
        args << QStringLiteral("--name") << containerName.trimmed();
    }

    for (const QVariant &entry : ports) {
        const QVariantMap map = entry.toMap();
        const QString hostPort = map.value(QStringLiteral("host")).toString().trimmed();
        const QString containerPort = map.value(QStringLiteral("container")).toString().trimmed();
        if (hostPort.isEmpty() || containerPort.isEmpty()) continue;
        args << QStringLiteral("-p") << (hostPort + QStringLiteral(":") + containerPort);
    }

    for (const QVariant &entry : envVars) {
        const QVariantMap map = entry.toMap();
        const QString key = map.value(QStringLiteral("key")).toString().trimmed();
        if (key.isEmpty()) continue;
        const QString value = map.value(QStringLiteral("value")).toString();
        args << QStringLiteral("-e") << (key + QStringLiteral("=") + value);
    }

    for (const QVariant &entry : volumes) {
        const QVariantMap map = entry.toMap();
        const QString hostPath = map.value(QStringLiteral("host")).toString().trimmed();
        const QString containerPath = map.value(QStringLiteral("container")).toString().trimmed();
        if (hostPath.isEmpty() || containerPath.isEmpty()) continue;
        args << QStringLiteral("-v") << (hostPath + QStringLiteral(":") + containerPath);
    }

    const QString restart = restartPolicy.trimmed();
    if (!restart.isEmpty() && restart != QStringLiteral("no")) {
        args << QStringLiteral("--restart") << restart;
    }

    args << image;
    args.append(splitCommandWords(command));

    spawnDockerExec(++m_nextRequestId, QStringLiteral("runImage"), args);
}

void PierDockerClientModel::removeImage(const QString &id, bool force)
{
    if (id.isEmpty()) return;
    QStringList args{ QStringLiteral("rmi") };
    if (force) {
        args << QStringLiteral("--force");
    }
    args << id;
    spawnDockerExec(++m_nextRequestId, QStringLiteral("removeImage"), args);
}

// ─── Volumes ────────────────────────────────────────────

void PierDockerClientModel::refreshVolumes()
{
    if (!m_handle && !m_localMode) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    auto w = std::make_unique<std::thread>([self, cancel, id, h, local]() {
        char *json = local ? pier_local_docker_list_volumes() : pier_docker_list_volumes(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) {
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onVolumesResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::onVolumesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const auto &v : doc.array()) {
            if (!v.isObject()) continue;
            const QJsonObject obj = v.toObject();
            QVariantMap map;
            map.insert(QStringLiteral("name"), readJsonString(obj, QStringLiteral("name"), QStringLiteral("Name")));
            map.insert(QStringLiteral("driver"), readJsonString(obj, QStringLiteral("driver"), QStringLiteral("Driver")));
            map.insert(QStringLiteral("mountpoint"), readJsonString(obj, QStringLiteral("mountpoint"), QStringLiteral("Mountpoint")));
            list.append(map);
        }
    }
    m_volumes = list;
    emit volumesChanged();
    setBusy(false);
}

void PierDockerClientModel::pruneVolumes()
{
    spawnDockerExec(++m_nextRequestId, QStringLiteral("pruneVolumes"),
                    QStringList{ QStringLiteral("volume"), QStringLiteral("prune"), QStringLiteral("-f") });
}

void PierDockerClientModel::removeVolume(const QString &name)
{
    if (name.isEmpty()) return;
    spawnDockerExec(++m_nextRequestId, QStringLiteral("removeVolume"),
                    QStringList{ QStringLiteral("volume"), QStringLiteral("rm"), name });
}

// ─── Networks ───────────────────────────────────────────

void PierDockerClientModel::refreshNetworks()
{
    if (!m_handle && !m_localMode) return;
    const quint64 id = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierDockerClientModel> self(this);
    auto cancel = m_cancelFlag;
    ::PierDocker *h = m_handle;
    const bool local = m_localMode;
    auto w = std::make_unique<std::thread>([self, cancel, id, h, local]() {
        char *json = local ? pier_local_docker_list_networks() : pier_docker_list_networks(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) {
            if (local) {
                pier_local_free_string(json);
            } else {
                pier_docker_free_string(json);
            }
        }
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onNetworksResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierDockerClientModel::onNetworksResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const auto &v : doc.array()) {
            if (!v.isObject()) continue;
            const QJsonObject obj = v.toObject();
            QVariantMap map;
            map.insert(QStringLiteral("id"), readJsonString(obj, QStringLiteral("id"), QStringLiteral("ID")));
            map.insert(QStringLiteral("name"), readJsonString(obj, QStringLiteral("name"), QStringLiteral("Name")));
            map.insert(QStringLiteral("driver"), readJsonString(obj, QStringLiteral("driver"), QStringLiteral("Driver")));
            map.insert(QStringLiteral("scope"), readJsonString(obj, QStringLiteral("scope"), QStringLiteral("Scope")));
            list.append(map);
        }
    }
    m_networks = list;
    emit networksChanged();
    setBusy(false);
}

void PierDockerClientModel::createNetwork(const QString &name, const QString &driver)
{
    const QString trimmedName = name.trimmed();
    if (trimmedName.isEmpty()) return;
    const QString driverName = driver.trimmed().isEmpty() ? QStringLiteral("bridge") : driver.trimmed();
    spawnDockerExec(++m_nextRequestId, QStringLiteral("createNetwork"),
                    QStringList{
                        QStringLiteral("network"),
                        QStringLiteral("create"),
                        QStringLiteral("--driver"),
                        driverName,
                        trimmedName
                    });
}

void PierDockerClientModel::removeNetwork(const QString &name)
{
    if (name.isEmpty()) return;
    spawnDockerExec(++m_nextRequestId, QStringLiteral("removeNetwork"),
                    QStringList{ QStringLiteral("network"), QStringLiteral("rm"), name });
}

// ─── Stop ───────────────────────────────────────────────

void PierDockerClientModel::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    ++m_nextRequestId;
    m_localMode = false;
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
    if (!m_images.isEmpty()) {
        m_images.clear();
        emit imagesChanged();
    }
    if (!m_volumes.isEmpty()) {
        m_volumes.clear();
        emit volumesChanged();
    }
    if (!m_networks.isEmpty()) {
        m_networks.clear();
        emit networksChanged();
    }
    if (!m_composeServices.isEmpty()) {
        m_composeServices.clear();
        emit composeServicesChanged();
    }
    clearInspect();
    if (m_status != Idle) {
        m_errorMessage.clear();
        m_target.clear();
        setStatus(Idle);
    }
    setBusy(false);
}
