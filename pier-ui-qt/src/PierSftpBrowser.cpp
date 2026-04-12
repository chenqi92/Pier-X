#include "PierSftpBrowser.h"
#include "PierSshSessionHandle.h"

#include "pier_sftp.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

PierSftpBrowser::PierSftpBrowser(QObject *parent)
    : QAbstractListModel(parent)
{
}

PierSftpBrowser::~PierSftpBrowser()
{
    stop();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

int PierSftpBrowser::rowCount(const QModelIndex &parent) const
{
    if (parent.isValid()) return 0;
    return static_cast<int>(m_entries.size());
}

QVariant PierSftpBrowser::data(const QModelIndex &index, int role) const
{
    if (!index.isValid()) return {};
    const int row = index.row();
    if (row < 0 || row >= static_cast<int>(m_entries.size())) return {};
    const Entry &e = m_entries[static_cast<size_t>(row)];
    switch (role) {
    case NameRole:     return e.name;
    case PathRole:     return e.path;
    case IsDirRole:    return e.isDir;
    case IsLinkRole:   return e.isLink;
    case SizeRole:     return e.size;
    case ModifiedRole: return e.modified;
    default:           return {};
    }
}

QHash<int, QByteArray> PierSftpBrowser::roleNames() const
{
    return {
        { NameRole,     "name" },
        { PathRole,     "path" },
        { IsDirRole,    "isDir" },
        { IsLinkRole,   "isLink" },
        { SizeRole,     "size" },
        { ModifiedRole, "modified" }
    };
}

void PierSftpBrowser::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierSftpBrowser::setBusy(bool b)
{
    if (m_busy == b) return;
    m_busy = b;
    emit busyChanged();
}

bool PierSftpBrowser::connectTo(const QString &host, int port, const QString &user,
                                 int authKind, const QString &secret, const QString &extra)
{
    if (m_handle || m_status == Connecting) {
        qWarning() << "PierSftpBrowser::connectTo called on already-connected session";
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

    // Capture strings as std::strings so the QByteArray temps
    // can drop with the main-thread frame. Mirrors how
    // PierTerminalSession::dispatchSshConnect works.
    std::string hostStd = host.toStdString();
    std::string userStd = user.toStdString();
    std::string secretStd = secret.toStdString();
    std::string extraStd = extra.toStdString();
    const uint16_t portU16 = static_cast<uint16_t>(port);
    const int kind = authKind;

    QPointer<PierSftpBrowser> selfWeak(this);
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

        PierSftp *h = pier_sftp_new(
            hostStd.c_str(),
            portU16,
            userStd.c_str(),
            kind,
            secretPtr,
            extraPtr);

        QString err;
        QString cwd;
        if (!h) {
            err = QStringLiteral("SFTP connect failed (see log)");
        } else {
            char *resolved = pier_sftp_canonicalize(h, ".");
            if (resolved) {
                cwd = QString::fromUtf8(resolved);
                pier_sftp_free_string(resolved);
            } else {
                cwd = QStringLiteral("/");
            }
        }

        const bool cancelled = cancelFlag && cancelFlag->load();
        if (!selfWeak || cancelled) {
            if (h) pier_sftp_free(h);
            return;
        }

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onConnectResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(QString, err),
            Q_ARG(QString, cwd));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

bool PierSftpBrowser::connectToSession(QObject *sessionObj)
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
    QPointer<PierSftpBrowser> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, session]() {
        PierSftp *h = pier_sftp_new_on_session(session);
        QString err;
        QString cwd;
        if (!h) {
            err = QStringLiteral("SFTP open_on_session failed");
        } else {
            char *resolved = pier_sftp_canonicalize(h, ".");
            if (resolved) { cwd = QString::fromUtf8(resolved); pier_sftp_free_string(resolved); }
            else { cwd = QStringLiteral("/"); }
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) { if (h) pier_sftp_free(h); return; }
        QMetaObject::invokeMethod(selfWeak.data(), "onConnectResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(void*, static_cast<void*>(h)),
            Q_ARG(QString, err), Q_ARG(QString, cwd));
    });
    m_workers.push_back(std::move(worker));
    return true;
}

void PierSftpBrowser::onConnectResult(quint64 requestId, void *handle, const QString &error, const QString &canonicalCwd)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_sftp_free(static_cast<PierSftp *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error.isEmpty() ? QStringLiteral("SFTP connect failed") : error;
        setStatus(Failed);
        setBusy(false);
        return;
    }
    m_handle = static_cast<PierSftp *>(handle);
    m_currentPath = canonicalCwd.isEmpty() ? QStringLiteral("/") : canonicalCwd;
    emit currentPathChanged();
    setStatus(Connected);
    // Auto-list the initial directory.
    spawnList(++m_nextRequestId, m_currentPath);
}

void PierSftpBrowser::listDir(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;
    spawnList(++m_nextRequestId, path);
}

void PierSftpBrowser::navigateUp()
{
    if (m_currentPath.isEmpty() || m_currentPath == QStringLiteral("/")) return;
    int slash = m_currentPath.lastIndexOf('/');
    QString parent = (slash <= 0) ? QStringLiteral("/") : m_currentPath.left(slash);
    listDir(parent);
}

void PierSftpBrowser::refresh()
{
    if (m_currentPath.isEmpty()) return;
    listDir(m_currentPath);
}

void PierSftpBrowser::spawnList(quint64 requestId, const QString &path)
{
    if (!m_handle) return;
    setBusy(true);

    std::string pathStd = path.toStdString();
    PierSftp *handle = m_handle;
    QPointer<PierSftpBrowser> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, handle,
        pathStd = std::move(pathStd)
    ]() mutable {
        char *json = pier_sftp_list_dir(handle, pathStd.c_str());
        QString jsonStr;
        QString err;
        if (json) {
            jsonStr = QString::fromUtf8(json);
            pier_sftp_free_string(json);
        } else {
            err = QStringLiteral("list_dir failed (see log)");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onListResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QString::fromStdString(pathStd)),
            Q_ARG(QString, jsonStr),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierSftpBrowser::onListResult(quint64 requestId, const QString &path, const QString &jsonEntries, const QString &error)
{
    (void)requestId;
    setBusy(false);
    if (!error.isEmpty()) {
        m_errorMessage = error;
        setStatus(Failed);
        return;
    }
    ingestListJson(jsonEntries);
    if (m_currentPath != path) {
        m_currentPath = path;
        emit currentPathChanged();
    }
}

void PierSftpBrowser::ingestListJson(const QString &json)
{
    QJsonParseError parseErr {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &parseErr);
    if (parseErr.error != QJsonParseError::NoError || !doc.isArray()) {
        qWarning() << "PierSftpBrowser: malformed listing JSON:" << parseErr.errorString();
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
        e.name = obj.value(QStringLiteral("name")).toString();
        e.path = obj.value(QStringLiteral("path")).toString();
        e.isDir = obj.value(QStringLiteral("is_dir")).toBool();
        e.isLink = obj.value(QStringLiteral("is_link")).toBool();
        e.size = static_cast<qint64>(obj.value(QStringLiteral("size")).toDouble());
        const QJsonValue mod = obj.value(QStringLiteral("modified"));
        e.modified = mod.isDouble() ? static_cast<qint64>(mod.toDouble()) : 0;
        m_entries.push_back(std::move(e));
    }
    endResetModel();
}

// ── Mutations (mkdir / rm / rename) ─────────────────────────
// They all follow the same shape: spawn a worker thread that
// calls the blocking FFI, post an onOperationResult back, then
// refresh() so the listing reflects the change.

static void spawn_bool_op(
    PierSftpBrowser *self,
    std::shared_ptr<std::atomic<bool>> cancelFlag,
    quint64 requestId,
    std::vector<std::unique_ptr<std::thread>> *workerSink,
    std::function<int32_t()> call,
    const QString &opLabel)
{
    QPointer<PierSftpBrowser> weak(self);
    auto worker = std::make_unique<std::thread>([weak, cancelFlag, requestId, call = std::move(call), opLabel]() mutable {
        const int32_t rc = call();
        if (!weak || (cancelFlag && cancelFlag->load())) return;
        const bool ok = (rc == 0);
        const QString msg = ok
            ? QStringLiteral("%1 succeeded").arg(opLabel)
            : QStringLiteral("%1 failed (code %2)").arg(opLabel).arg(rc);
        QMetaObject::invokeMethod(
            weak.data(),
            "onOperationResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(bool, ok),
            Q_ARG(QString, msg));
    });
    workerSink->push_back(std::move(worker));
}

void PierSftpBrowser::mkdir(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;
    setBusy(true);
    const quint64 rid = ++m_nextRequestId;
    std::string pathStd = path.toStdString();
    PierSftp *h = m_handle;
    spawn_bool_op(
        this, m_cancelFlag, rid, &m_workers,
        [h, pathStd]() mutable { return pier_sftp_mkdir(h, pathStd.c_str()); },
        QStringLiteral("mkdir"));
}

void PierSftpBrowser::removeFile(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;
    setBusy(true);
    const quint64 rid = ++m_nextRequestId;
    std::string pathStd = path.toStdString();
    PierSftp *h = m_handle;
    spawn_bool_op(
        this, m_cancelFlag, rid, &m_workers,
        [h, pathStd]() mutable { return pier_sftp_remove_file(h, pathStd.c_str()); },
        QStringLiteral("remove file"));
}

void PierSftpBrowser::removeDir(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;
    setBusy(true);
    const quint64 rid = ++m_nextRequestId;
    std::string pathStd = path.toStdString();
    PierSftp *h = m_handle;
    spawn_bool_op(
        this, m_cancelFlag, rid, &m_workers,
        [h, pathStd]() mutable { return pier_sftp_remove_dir(h, pathStd.c_str()); },
        QStringLiteral("remove dir"));
}

void PierSftpBrowser::rename(const QString &from, const QString &to)
{
    if (!m_handle || from.isEmpty() || to.isEmpty()) return;
    setBusy(true);
    const quint64 rid = ++m_nextRequestId;
    std::string fromStd = from.toStdString();
    std::string toStd = to.toStdString();
    PierSftp *h = m_handle;
    spawn_bool_op(
        this, m_cancelFlag, rid, &m_workers,
        [h, fromStd, toStd]() mutable {
            return pier_sftp_rename(h, fromStd.c_str(), toStd.c_str());
        },
        QStringLiteral("rename"));
}

void PierSftpBrowser::onOperationResult(quint64 requestId, bool ok, const QString &message)
{
    (void)requestId;
    setBusy(false);
    emit operationFinished(ok, message);
    if (ok) {
        refresh();
    }
}

void PierSftpBrowser::stop()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    // Bump the request id so any in-flight result is dropped.
    ++m_nextRequestId;
    if (m_handle) {
        PierSftp *h = m_handle;
        m_handle = nullptr;
        pier_sftp_free(h);
    }
    beginResetModel();
    m_entries.clear();
    endResetModel();
    if (m_status != Idle) {
        m_errorMessage.clear();
        m_target.clear();
        m_currentPath.clear();
        emit currentPathChanged();
        setStatus(Idle);
    }
    setBusy(false);
}
