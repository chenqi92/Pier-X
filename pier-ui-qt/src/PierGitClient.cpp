#include "PierGitClient.h"

#include "pier_git.h"

#include <QByteArray>
#include <QDebug>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>

// ─── PierGitClient ──────────────────────────────────────

PierGitClient::PierGitClient(QObject *parent)
    : QObject(parent)
{
}

PierGitClient::~PierGitClient()
{
    close();
    for (auto &t : m_workers) {
        if (t && t->joinable()) {
            t->detach();
        }
    }
}

void PierGitClient::setStatus(Status s)
{
    if (m_status == s) return;
    m_status = s;
    emit statusChanged();
}

void PierGitClient::setBusy(bool b)
{
    if (m_busy == b) return;
    m_busy = b;
    emit busyChanged();
}

// ─── Open ───────────────────────────────────────────────

void PierGitClient::open(const QString &path)
{
    if (m_handle) {
        close();
    }
    if (path.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    m_cancelFlag = std::make_shared<std::atomic<bool>>(false);
    m_errorMessage.clear();
    setStatus(Loading);
    setBusy(true);

    std::string pathStd = path.toStdString();
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId,
        pathStd = std::move(pathStd)
    ]() mutable {
        ::PierGit *h = pier_git_open(pathStd.c_str());

        QString err;
        QString repoPath;
        if (!h) {
            err = QStringLiteral("Not a git repository");
        } else {
            repoPath = QString::fromStdString(pathStd);
        }

        if (!selfWeak || (cancelFlag && cancelFlag->load())) {
            if (h) pier_git_free(h);
            return;
        }
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpenResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(void *, static_cast<void *>(h)),
            Q_ARG(QString, repoPath),
            Q_ARG(QString, err));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onOpenResult(quint64 requestId, void *handle, const QString &repoPath, const QString &error)
{
    if (requestId != m_nextRequestId) {
        if (handle) pier_git_free(static_cast<::PierGit *>(handle));
        return;
    }
    if (!handle) {
        m_errorMessage = error;
        setStatus(Failed);
        setBusy(false);
        return;
    }
    m_handle = static_cast<::PierGit *>(handle);
    m_repoPath = repoPath;
    emit repoChanged();
    setStatus(Ready);
    setBusy(false);

    // Auto-load status + branch info after open
    refresh();
}

// ─── Refresh (status + branch) ──────────────────────────

void PierGitClient::refresh()
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    // Status thread
    auto statusWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        char *json = pier_git_status(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onStatusResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(statusWorker));

    // Branch thread
    auto branchWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        char *json = pier_git_branch_info(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("{}");
        if (json) pier_git_free_string(json);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onBranchResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(branchWorker));
}

void PierGitClient::onStatusResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    parseStatusJson(json);
    setBusy(false);
}

void PierGitClient::onBranchResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    parseBranchJson(json);
}

void PierGitClient::parseStatusJson(const QString &json)
{
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());

    // Check for error
    if (doc.isObject() && doc.object().contains(QStringLiteral("error"))) {
        m_errorMessage = doc.object().value(QStringLiteral("error")).toString();
        emit statusChanged();
        return;
    }

    QVariantList staged;
    QVariantList unstaged;

    const QJsonArray arr = doc.array();
    for (const QJsonValue &val : arr) {
        QJsonObject obj = val.toObject();
        QVariantMap entry;
        entry[QStringLiteral("path")] = obj.value(QStringLiteral("path")).toString();

        // Convert FileStatus enum to single-char code for QML
        const QString statusStr = obj.value(QStringLiteral("status")).toString();
        QString code;
        if (statusStr == QStringLiteral("Modified")) code = QStringLiteral("M");
        else if (statusStr == QStringLiteral("Added")) code = QStringLiteral("A");
        else if (statusStr == QStringLiteral("Deleted")) code = QStringLiteral("D");
        else if (statusStr == QStringLiteral("Renamed")) code = QStringLiteral("R");
        else if (statusStr == QStringLiteral("Untracked")) code = QStringLiteral("?");
        else if (statusStr == QStringLiteral("Conflicted")) code = QStringLiteral("U");
        else if (statusStr == QStringLiteral("Copied")) code = QStringLiteral("C");
        else code = statusStr;
        entry[QStringLiteral("status")] = code;

        // Extract just the filename from the path
        const QString path = entry.value(QStringLiteral("path")).toString();
        const int lastSlash = path.lastIndexOf(QLatin1Char('/'));
        entry[QStringLiteral("fileName")] = (lastSlash >= 0)
            ? path.mid(lastSlash + 1) : path;

        if (obj.value(QStringLiteral("staged")).toBool()) {
            staged.append(entry);
        } else {
            unstaged.append(entry);
        }
    }

    m_stagedFiles = staged;
    m_unstagedFiles = unstaged;
    emit filesChanged();
}

void PierGitClient::parseBranchJson(const QString &json)
{
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) return;

    QJsonObject obj = doc.object();
    m_currentBranch = obj.value(QStringLiteral("name")).toString();
    m_trackingBranch = obj.value(QStringLiteral("tracking")).toString();
    m_aheadCount = obj.value(QStringLiteral("ahead")).toInt();
    m_behindCount = obj.value(QStringLiteral("behind")).toInt();
    emit branchChanged();
}

// ─── Stage / Unstage ────────────────────────────────────

void PierGitClient::stageFile(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string pathsJson = QStringLiteral("[\"%1\"]").arg(path).toStdString();

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        pathsJson = std::move(pathsJson)
    ]() {
        int rc = pier_git_stage(h, pathsJson.c_str());
        bool ok = (rc == 0);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("stage")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? QString() : QStringLiteral("Stage failed")));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::unstageFile(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string pathsJson = QStringLiteral("[\"%1\"]").arg(path).toStdString();

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        pathsJson = std::move(pathsJson)
    ]() {
        int rc = pier_git_unstage(h, pathsJson.c_str());
        bool ok = (rc == 0);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("unstage")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? QString() : QStringLiteral("Unstage failed")));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::stageAll()
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        int rc = pier_git_stage_all(h);
        bool ok = (rc == 0);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("stageAll")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? QString() : QStringLiteral("Stage all failed")));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::unstageAll()
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        int rc = pier_git_unstage_all(h);
        bool ok = (rc == 0);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("unstageAll")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? QString() : QStringLiteral("Unstage all failed")));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::discardFile(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string pathsJson = QStringLiteral("[\"%1\"]").arg(path).toStdString();

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        pathsJson = std::move(pathsJson)
    ]() {
        int rc = pier_git_discard(h, pathsJson.c_str());
        bool ok = (rc == 0);

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("discard")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? QString() : QStringLiteral("Discard failed")));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Diff ───────────────────────────────────────────────

void PierGitClient::loadDiff(const QString &path, bool staged)
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string pathStd = path.toStdString();
    const int stagedFlag = staged ? 1 : 0;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        pathStd = std::move(pathStd),
        stagedFlag,
        qmlPath = path
    ]() {
        char *raw = pier_git_diff(h, pathStd.c_str(), stagedFlag);
        QString text = raw ? QString::fromUtf8(raw) : QString();
        if (raw) pier_git_free_string(raw);

        // If the diff is empty and this is an untracked file, try
        // loading the full content instead
        if (text.isEmpty() && stagedFlag == 0) {
            char *ut = pier_git_diff_untracked(h, pathStd.c_str());
            if (ut) {
                text = QString::fromUtf8(ut);
                pier_git_free_string(ut);
            }
        }

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onDiffResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, qmlPath),
            Q_ARG(QString, text));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onDiffResult(quint64 requestId, const QString &path, const QString &text)
{
    if (requestId != m_nextRequestId) return;
    m_diffPath = path;
    m_diffText = text;
    emit diffChanged();
    setBusy(false);
}

// ─── Commit / Push / Pull ───────────────────────────────

void PierGitClient::commit(const QString &message)
{
    if (!m_handle || message.isEmpty()) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string msgStd = message.toStdString();

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h,
        msgStd = std::move(msgStd)
    ]() {
        char *result = pier_git_commit(h, msgStd.c_str());
        QString output = result ? QString::fromUtf8(result) : QString();
        if (result) pier_git_free_string(result);

        bool ok = !output.contains(QStringLiteral("\"error\""));

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("commit")),
            Q_ARG(bool, ok),
            Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::push()
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        char *result = pier_git_push(h);
        QString output = result ? QString::fromUtf8(result) : QString();
        if (result) pier_git_free_string(result);

        bool ok = !output.contains(QStringLiteral("\"error\""));

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("push")),
            Q_ARG(bool, ok),
            Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::pull()
{
    if (!m_handle) return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, h
    ]() {
        char *result = pier_git_pull(h);
        QString output = result ? QString::fromUtf8(result) : QString();
        if (result) pier_git_free_string(result);

        bool ok = !output.contains(QStringLiteral("\"error\""));

        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("pull")),
            Q_ARG(bool, ok),
            Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onOpResult(quint64 requestId, const QString &operation, bool success, const QString &message)
{
    if (requestId != m_nextRequestId) return;
    setBusy(false);
    emit operationFinished(operation, success, message);

    // Auto-refresh after mutating operations
    if (operation == QStringLiteral("stage") || operation == QStringLiteral("unstage")
        || operation == QStringLiteral("stageAll") || operation == QStringLiteral("unstageAll")
        || operation == QStringLiteral("discard") || operation == QStringLiteral("commit")
        || operation == QStringLiteral("pull") || operation == QStringLiteral("checkout")) {
        refresh();
    }
    if (operation == QStringLiteral("stashPush") || operation == QStringLiteral("stashPop")
        || operation == QStringLiteral("stashDrop") || operation == QStringLiteral("stashApply")) {
        loadStashes();
        refresh();
    }
}

// ─── History ────────────────────────────────────────────

void PierGitClient::loadHistory(int limit)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    const uint32_t lim = static_cast<uint32_t>(limit > 0 ? limit : 100);

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, lim]() {
        char *json = pier_git_log(h, lim);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onHistoryResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onHistoryResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const QJsonValue &v : doc.array()) {
            QJsonObject obj = v.toObject();
            QVariantMap m;
            m[QStringLiteral("hash")] = obj.value(QStringLiteral("hash")).toString();
            m[QStringLiteral("shortHash")] = obj.value(QStringLiteral("short_hash")).toString();
            m[QStringLiteral("message")] = obj.value(QStringLiteral("message")).toString();
            m[QStringLiteral("author")] = obj.value(QStringLiteral("author")).toString();
            m[QStringLiteral("relativeDate")] = obj.value(QStringLiteral("relative_date")).toString();
            m[QStringLiteral("refs")] = obj.value(QStringLiteral("refs")).toString();
            list.append(m);
        }
    }
    m_commits = list;
    emit commitsChanged();
    setBusy(false);
}

// ─── Stash ──────────────────────────────────────────────

void PierGitClient::loadStashes()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h]() {
        char *json = pier_git_stash_list(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onStashResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onStashResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const QJsonValue &v : doc.array()) {
            QJsonObject obj = v.toObject();
            QVariantMap m;
            m[QStringLiteral("index")] = obj.value(QStringLiteral("index")).toString();
            m[QStringLiteral("message")] = obj.value(QStringLiteral("message")).toString();
            m[QStringLiteral("relativeDate")] = obj.value(QStringLiteral("relative_date")).toString();
            list.append(m);
        }
    }
    m_stashes = list;
    emit stashesChanged();
    setBusy(false);
}

void PierGitClient::stashPush(const QString &message)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string msg = message.toStdString();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, msg = std::move(msg)]() {
        char *r = pier_git_stash_push(h, msg.empty() ? nullptr : msg.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("stashPush")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::stashApply(const QString &index)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string idx = index.toStdString();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, idx = std::move(idx)]() {
        char *r = pier_git_stash_apply(h, idx.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("stashApply")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::stashPop(const QString &index)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string idx = index.toStdString();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, idx = std::move(idx)]() {
        char *r = pier_git_stash_pop(h, idx.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("stashPop")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::stashDrop(const QString &index)
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string idx = index.toStdString();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, idx = std::move(idx)]() {
        char *r = pier_git_stash_drop(h, idx.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("stashDrop")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Branches ───────────────────────────────────────────

void PierGitClient::loadBranches()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h]() {
        char *json = pier_git_branch_list_local(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onBranchesResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onBranchesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QStringList list;
    if (doc.isArray()) {
        for (const QJsonValue &v : doc.array()) {
            if (v.isString()) list.append(v.toString());
        }
    }
    m_branches = list;
    emit branchesChanged();
}

void PierGitClient::checkoutBranch(const QString &name)
{
    if (!m_handle || name.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string n = name.toStdString();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, n = std::move(n)]() {
        char *r = pier_git_checkout_branch(h, n.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("checkout")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Close ──────────────────────────────────────────────

void PierGitClient::close()
{
    if (m_cancelFlag) {
        m_cancelFlag->store(true);
    }
    if (m_handle) {
        pier_git_free(m_handle);
        m_handle = nullptr;
    }
    m_repoPath.clear();
    m_currentBranch.clear();
    m_trackingBranch.clear();
    m_aheadCount = 0;
    m_behindCount = 0;
    m_stagedFiles.clear();
    m_unstagedFiles.clear();
    m_diffText.clear();
    m_diffPath.clear();
    setStatus(Idle);
    emit repoChanged();
    emit branchChanged();
    emit filesChanged();
    emit diffChanged();
}
