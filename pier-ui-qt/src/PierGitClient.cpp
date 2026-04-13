#include "PierGitClient.h"

#include "pier_git.h"

#include <QByteArray>
#include <QDebug>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QMetaObject>
#include <QProcess>
#include <QProcessEnvironment>
#include <QSaveFile>
#include <QTemporaryFile>

namespace {

QString runGitCommandAt(const QString &repoPath,
                        const QStringList &args,
                        bool *ok = nullptr,
                        const QProcessEnvironment &environment = QProcessEnvironment::systemEnvironment(),
                        int timeoutMs = 20000)
{
    if (repoPath.isEmpty()) {
        if (ok) *ok = false;
        return QStringLiteral("Missing repository path");
    }

    QProcess process;
    process.setProgram(QStringLiteral("git"));
    process.setArguments(QStringList{QStringLiteral("-C"), repoPath} + args);
    process.setProcessEnvironment(environment);
    process.start();
    if (!process.waitForStarted(3000)) {
        if (ok) *ok = false;
        return QStringLiteral("Failed to start git");
    }
    if (!process.waitForFinished(timeoutMs)) {
        process.kill();
        if (ok) *ok = false;
        return QStringLiteral("git command timed out");
    }

    const QString stdOut = QString::fromUtf8(process.readAllStandardOutput()).trimmed();
    const QString stdErr = QString::fromUtf8(process.readAllStandardError()).trimmed();
    const bool success = process.exitStatus() == QProcess::NormalExit && process.exitCode() == 0;
    if (ok) *ok = success;

    if (!success) {
        if (!stdErr.isEmpty()) return stdErr;
        if (!stdOut.isEmpty()) return stdOut;
        return QStringLiteral("git command failed");
    }

    return !stdOut.isEmpty() ? stdOut : stdErr;
}

QString decodeGitOutput(char *raw, const QString &fallback = QString())
{
    const QString output = raw ? QString::fromUtf8(raw) : fallback;
    if (raw) pier_git_free_string(raw);
    return output;
}

bool gitOutputOk(const QString &output)
{
    return !output.contains(QStringLiteral("\"error\""));
}

QVariantList parseConflictHunksFromContent(const QString &content)
{
    QVariantList hunks;
    const QStringList lines = content.split(QLatin1Char('\n'));
    int i = 0;

    while (i < lines.size()) {
        if (!lines.at(i).startsWith(QStringLiteral("<<<<<<<"))) {
            ++i;
            continue;
        }

        QStringList oursLines;
        QStringList theirsLines;
        ++i;

        while (i < lines.size() && !lines.at(i).startsWith(QStringLiteral("======="))) {
            oursLines.append(lines.at(i));
            ++i;
        }

        if (i < lines.size())
            ++i;

        while (i < lines.size() && !lines.at(i).startsWith(QStringLiteral(">>>>>>>"))) {
            theirsLines.append(lines.at(i));
            ++i;
        }

        QVariantMap hunk;
        hunk[QStringLiteral("oursLines")] = oursLines;
        hunk[QStringLiteral("theirsLines")] = theirsLines;
        hunk[QStringLiteral("resolution")] = QString();
        hunks.append(hunk);

        if (i < lines.size())
            ++i;
    }

    return hunks;
}

QString gitPathForRepo(const QString &repoPath, const QString &relativePath, bool *ok = nullptr)
{
    return runGitCommandAt(repoPath,
                           {QStringLiteral("rev-parse"), QStringLiteral("--git-path"), relativePath},
                           ok).trimmed();
}

QVariantMap parseCommitDetailDocument(const QString &metaOutput,
                                      const QString &statsOutput,
                                      const QString &numstatOutput,
                                      const QString &parentsOutput)
{
    QVariantMap detail;
    const QChar sep(0x1f);

    const int first = metaOutput.indexOf(sep);
    const int second = metaOutput.indexOf(sep, first + 1);
    const int third = metaOutput.indexOf(sep, second + 1);
    const int fourth = metaOutput.indexOf(sep, third + 1);

    if (first > 0 && second > first && third > second && fourth > third) {
        const QString hash = metaOutput.left(first).trimmed();
        detail[QStringLiteral("hash")] = hash;
        detail[QStringLiteral("shortHash")] = metaOutput.mid(first + 1, second - first - 1).trimmed();
        detail[QStringLiteral("author")] = metaOutput.mid(second + 1, third - second - 1).trimmed();
        detail[QStringLiteral("date")] = metaOutput.mid(third + 1, fourth - third - 1).trimmed();
        detail[QStringLiteral("message")] = metaOutput.mid(fourth + 1).trimmed();
    }

    const QStringList parentTokens = parentsOutput.split(QLatin1Char(' '), Qt::SkipEmptyParts);
    QVariantList parentHashes;
    for (int i = 1; i < parentTokens.size(); ++i)
        parentHashes.append(parentTokens.at(i).trimmed());
    detail[QStringLiteral("parentHashes")] = parentHashes;
    detail[QStringLiteral("parentHash")] = parentHashes.isEmpty() ? QString() : parentHashes.first();

    QString statsSummary;
    const QStringList statLines = statsOutput.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
    for (auto it = statLines.crbegin(); it != statLines.crend(); ++it) {
        if (!it->trimmed().isEmpty()) {
            statsSummary = it->trimmed();
            break;
        }
    }
    detail[QStringLiteral("stats")] = statsSummary;

    QVariantList changedFiles;
    const QStringList numstatLines = numstatOutput.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
    for (const QString &line : numstatLines) {
        const QStringList parts = line.split(QLatin1Char('\t'));
        if (parts.size() < 3)
            continue;
        QVariantMap file;
        file[QStringLiteral("additions")] = parts.at(0) == QStringLiteral("-") ? 0 : parts.at(0).toInt();
        file[QStringLiteral("deletions")] = parts.at(1) == QStringLiteral("-") ? 0 : parts.at(1).toInt();
        file[QStringLiteral("path")] = parts.mid(2).join(QStringLiteral("\t")).trimmed();
        changedFiles.append(file);
    }
    detail[QStringLiteral("changedFiles")] = changedFiles;

    return detail;
}

QVariantMap parseRebaseLine(const QString &line)
{
    QVariantMap item;
    const QString trimmed = line.trimmed();
    if (trimmed.isEmpty() || trimmed.startsWith(QLatin1Char('#')) || trimmed == QStringLiteral("noop"))
        return item;

    const QStringList parts = trimmed.split(QLatin1Char(' '), Qt::SkipEmptyParts);
    if (parts.size() < 2)
        return item;

    const QString action = parts.at(0).trimmed();
    const QString hash = parts.at(1).trimmed();
    const QString message = parts.mid(2).join(QStringLiteral(" ")).trimmed();
    item[QStringLiteral("id")] = hash;
    item[QStringLiteral("action")] = action;
    item[QStringLiteral("hash")] = hash;
    item[QStringLiteral("shortHash")] = hash.left(7);
    item[QStringLiteral("message")] = message;
    return item;
}

QHash<QString, QString> parseGitmodulesUrls(const QString &repoPath)
{
    QHash<QString, QString> urlsByPath;
    QFile file(QDir(repoPath).filePath(QStringLiteral(".gitmodules")));
    if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
        return urlsByPath;

    QString currentPath;
    QString currentUrl;
    const QStringList lines = QString::fromUtf8(file.readAll()).split(QLatin1Char('\n'));
    for (const QString &rawLine : lines) {
        const QString line = rawLine.trimmed();
        if (line.startsWith(QLatin1Char('['))) {
            if (!currentPath.isEmpty())
                urlsByPath.insert(currentPath, currentUrl);
            currentPath.clear();
            currentUrl.clear();
            continue;
        }
        if (line.startsWith(QStringLiteral("path"))) {
            const int idx = line.indexOf(QLatin1Char('='));
            currentPath = idx >= 0 ? line.mid(idx + 1).trimmed() : QString();
        } else if (line.startsWith(QStringLiteral("url"))) {
            const int idx = line.indexOf(QLatin1Char('='));
            currentUrl = idx >= 0 ? line.mid(idx + 1).trimmed() : QString();
        }
    }
    if (!currentPath.isEmpty())
        urlsByPath.insert(currentPath, currentUrl);
    return urlsByPath;
}

} // namespace

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
    detectConflicts();
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

void PierGitClient::commitAndPush(const QString &message)
{
    if (m_repoPath.isEmpty() || message.trimmed().isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitMessage = message.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitMessage]() {
        bool ok = false;
        QString output = runGitCommandAt(repoPath,
                                         {QStringLiteral("commit"), QStringLiteral("-m"), commitMessage},
                                         &ok);
        if (ok)
            output = runGitCommandAt(repoPath, {QStringLiteral("push")}, &ok, QProcessEnvironment::systemEnvironment(), 120000);

        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("commitAndPush")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
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

    QString normalizedMessage = message;
    if (success && normalizedMessage.trimmed().isEmpty()) {
        if (operation == QStringLiteral("tagPush"))
            normalizedMessage = tr("Pushed tag.");
        else if (operation == QStringLiteral("tagPushAll"))
            normalizedMessage = tr("Pushed all tags.");
    }

    if (operation == QStringLiteral("comparisonFiles")) {
        QVariantList files;
        if (success) {
            const QJsonDocument doc = QJsonDocument::fromJson(normalizedMessage.toUtf8());
            if (doc.isArray()) {
                for (const QJsonValue &value : doc.array())
                    files.append(value.toObject().toVariantMap());
            }
        }
        m_comparisonFiles = files;
        m_comparisonBaseHash = success ? m_comparisonBaseHash : QString();
        emit comparisonChanged();
        return;
    }

    if (operation == QStringLiteral("comparisonDiff")) {
        m_comparisonDiff = success ? normalizedMessage : QString();
        emit comparisonChanged();
        return;
    }

    emit operationFinished(operation, success, normalizedMessage);

    // Auto-refresh after mutating operations
    if (operation == QStringLiteral("stage") || operation == QStringLiteral("unstage")
        || operation == QStringLiteral("stageAll") || operation == QStringLiteral("unstageAll")
        || operation == QStringLiteral("discard") || operation == QStringLiteral("commit")
        || operation == QStringLiteral("commitAndPush")
        || operation == QStringLiteral("pull") || operation == QStringLiteral("checkout")
        || operation == QStringLiteral("branchCreate") || operation == QStringLiteral("branchDelete")
        || operation == QStringLiteral("branchRename") || operation == QStringLiteral("branchMerge")
        || operation == QStringLiteral("commitReset") || operation == QStringLiteral("commitDrop")
        || operation == QStringLiteral("commitEditMessage")
        || operation == QStringLiteral("branchTrackingSet")
        || operation == QStringLiteral("branchTrackingUnset")
        || operation == QStringLiteral("remoteFetch")
        || operation == QStringLiteral("rebaseExecute")
        || operation == QStringLiteral("rebaseAbort")
        || operation == QStringLiteral("rebaseContinue")) {
        refresh();
        loadGraphMetadata();
    }
    if (operation == QStringLiteral("stashPush") || operation == QStringLiteral("stashPop")
        || operation == QStringLiteral("stashDrop") || operation == QStringLiteral("stashApply")) {
        loadStashes();
        refresh();
    }
    if (operation == QStringLiteral("tagCreate") || operation == QStringLiteral("tagDelete")
        || operation == QStringLiteral("tagPush") || operation == QStringLiteral("tagPushAll")) {
        loadTags();
    }
    if (operation == QStringLiteral("remoteAdd")
        || operation == QStringLiteral("remoteRemove")
        || operation == QStringLiteral("remoteSetUrl")) {
        loadRemotes();
    }
    if (operation == QStringLiteral("configSet") || operation == QStringLiteral("configUnset")) {
        loadConfig();
    }
    if (operation == QStringLiteral("rebaseExecute")
        || operation == QStringLiteral("rebaseAbort")
        || operation == QStringLiteral("rebaseContinue")) {
        loadRebasePlan();
    }
    if (operation == QStringLiteral("submoduleInit")
        || operation == QStringLiteral("submoduleUpdate")
        || operation == QStringLiteral("submoduleSync")) {
        loadSubmodules();
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

void PierGitClient::loadGraphHistory(int limit,
                                     int skip,
                                     const QString &branch,
                                     const QString &author,
                                     const QString &searchText,
                                     bool firstParent,
                                     bool noMerges,
                                     qint64 afterTimestamp,
                                     const QString &pathFilter,
                                     bool topoOrder,
                                     bool showLongEdges)
{
    if (m_repoPath.isEmpty()) return;

    const quint64 requestId = ++m_nextGraphRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branchName = branch.trimmed().isEmpty() ? m_currentBranch : branch.trimmed();
    const QString authorName = author.trimmed();
    const QString search = searchText.trimmed();
    const QString path = pathFilter;

    auto worker = std::make_unique<std::thread>([
        selfWeak,
        cancelFlag,
        requestId,
        repoPath,
        branchName,
        authorName,
        search,
        path,
        limit,
        skip,
        firstParent,
        noMerges,
        afterTimestamp,
        topoOrder,
        showLongEdges
    ]() {
        QByteArray repoUtf8 = repoPath.toUtf8();
        QByteArray branchUtf8 = branchName.toUtf8();
        QByteArray authorUtf8 = authorName.toUtf8();
        QByteArray searchUtf8 = search.toUtf8();
        QByteArray pathsUtf8;

        const char *branchPtr = branchName.isEmpty() ? nullptr : branchUtf8.constData();
        const char *authorPtr = authorName.isEmpty() ? nullptr : authorUtf8.constData();
        const char *searchPtr = search.isEmpty() ? nullptr : searchUtf8.constData();
        const char *pathsPtr = nullptr;

        const QStringList pathEntries = path.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
        if (!pathEntries.isEmpty()) {
            QJsonArray paths;
            for (const QString &entry : pathEntries) {
                const QString trimmed = entry.trimmed();
                if (!trimmed.isEmpty())
                    paths.append(trimmed);
            }
            pathsUtf8 = QJsonDocument(paths).toJson(QJsonDocument::Compact);
            pathsPtr = pathsUtf8.constData();
        }

        QString commitsJson = decodeGitOutput(
            pier_git_graph_log(repoUtf8.constData(),
                               static_cast<uint32_t>(limit > 0 ? limit : 180),
                               static_cast<uint32_t>(skip > 0 ? skip : 0),
                               branchPtr,
                               authorPtr,
                               searchPtr,
                               afterTimestamp,
                               topoOrder,
                               firstParent,
                               noMerges,
                               pathsPtr),
            QStringLiteral("[]"));

        QString mainRef = branchName;
        if (mainRef.isEmpty()) {
            mainRef = decodeGitOutput(pier_git_detect_default_branch(repoUtf8.constData()),
                                      QStringLiteral("HEAD"));
            if (mainRef.trimmed().isEmpty())
                mainRef = QStringLiteral("HEAD");
        }

        QByteArray mainRefUtf8 = mainRef.toUtf8();
        QString mainChainJson = decodeGitOutput(
            pier_git_first_parent_chain(repoUtf8.constData(),
                                        mainRefUtf8.constData(),
                                        static_cast<uint32_t>(limit > 0 ? limit : 180)),
            QStringLiteral("[]"));

        QByteArray commitsUtf8 = commitsJson.toUtf8();
        QByteArray mainChainUtf8 = mainChainJson.toUtf8();
        QString rowsJson = decodeGitOutput(
            pier_git_compute_graph_layout(commitsUtf8.constData(),
                                          mainChainUtf8.constData(),
                                          12.0f,
                                          24.0f,
                                          showLongEdges),
            QStringLiteral("[]"));

        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onGraphResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, rowsJson));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::loadGraphMetadata()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextGraphMetadataRequestId;

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto branchesWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, repoPath
    ]() {
        const QByteArray repoUtf8 = repoPath.toUtf8();
        const QString json = decodeGitOutput(
            pier_git_list_branches(repoUtf8.constData()),
            QStringLiteral("[]"));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onGraphBranchesResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(branchesWorker));

    auto authorsWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, repoPath
    ]() {
        const QByteArray repoUtf8 = repoPath.toUtf8();
        const QString json = decodeGitOutput(
            pier_git_list_authors(repoUtf8.constData(), 96),
            QStringLiteral("[]"));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onGraphAuthorsResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(authorsWorker));

    auto filesWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, repoPath
    ]() {
        const QByteArray repoUtf8 = repoPath.toUtf8();
        const QString json = decodeGitOutput(
            pier_git_list_tracked_files(repoUtf8.constData()),
            QStringLiteral("[]"));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onGraphRepoFilesResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(filesWorker));

    auto gitUserWorker = std::make_unique<std::thread>([
        selfWeak, cancelFlag, requestId, repoPath
    ]() {
        bool ok = false;
        const QString user = runGitCommandAt(repoPath,
                                             {QStringLiteral("config"), QStringLiteral("user.name")},
                                             &ok).trimmed();
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onGraphGitUserResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, ok ? user : QString()));
    });
    m_workers.push_back(std::move(gitUserWorker));
}

void PierGitClient::onGraphResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextGraphRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array())
            list.append(value.toObject().toVariantMap());
    }
    m_graphRows = list;
    emit graphChanged();
    setBusy(false);
}

void PierGitClient::onGraphBranchesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextGraphMetadataRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QStringList branches;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array()) {
            if (value.isString())
                branches.append(value.toString());
        }
    }
    branches.removeDuplicates();
    branches.sort(Qt::CaseInsensitive);
    m_graphBranches = branches;
    emit graphMetadataChanged();
}

void PierGitClient::onGraphAuthorsResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextGraphMetadataRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QStringList authors;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array()) {
            if (value.isString())
                authors.append(value.toString());
        }
    }
    authors.removeDuplicates();
    authors.sort(Qt::CaseInsensitive);
    m_graphAuthors = authors;
    emit graphMetadataChanged();
}

void PierGitClient::onGraphRepoFilesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextGraphMetadataRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QStringList files;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array()) {
            if (value.isString())
                files.append(value.toString());
        }
    }
    files.removeDuplicates();
    files.sort(Qt::CaseInsensitive);
    m_graphRepoFiles = files;
    emit graphMetadataChanged();
}

void PierGitClient::onGraphGitUserResult(quint64 requestId, const QString &value)
{
    if (requestId != m_nextGraphMetadataRequestId)
        return;

    m_graphGitUserName = value.trimmed();
    emit graphMetadataChanged();
}

void PierGitClient::loadComparisonFiles(const QString &hash)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    m_comparisonBaseHash = commitHash;
    m_comparisonPath.clear();
    m_comparisonDiff.clear();
    emit comparisonChanged();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("diff"),
                                                QStringLiteral("--name-only"),
                                                commitHash,
                                                QStringLiteral("HEAD")},
                                               &ok);

        QVariantList files;
        if (ok) {
            const QStringList lines = output.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
            for (const QString &line : lines) {
                const QString path = line.trimmed();
                if (path.isEmpty())
                    continue;
                QVariantMap item;
                item[QStringLiteral("path")] = path;
                item[QStringLiteral("name")] = QFileInfo(path).fileName();
                item[QStringLiteral("dir")] = QFileInfo(path).path();
                files.append(item);
            }
        }

        const QString json = QString::fromUtf8(QJsonDocument::fromVariant(files).toJson(QJsonDocument::Compact));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("comparisonFiles")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? json : output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::loadComparisonDiff(const QString &hash, const QString &path)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty() || path.trimmed().isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    const QString relativePath = path.trimmed();
    m_comparisonBaseHash = commitHash;
    m_comparisonPath = relativePath;
    m_comparisonDiff.clear();
    emit comparisonChanged();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash, relativePath]() {
        bool ok = false;
        const QString diff = runGitCommandAt(repoPath,
                                             {QStringLiteral("diff"),
                                              QStringLiteral("--stat=0"),
                                              commitHash,
                                              QStringLiteral("HEAD"),
                                              QStringLiteral("--"),
                                              relativePath},
                                             &ok,
                                             QProcessEnvironment::systemEnvironment(),
                                             60000);

        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onOpResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, QStringLiteral("comparisonDiff")),
            Q_ARG(bool, ok),
            Q_ARG(QString, ok ? diff : QString()));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::clearComparison()
{
    m_comparisonFiles.clear();
    m_comparisonDiff.clear();
    m_comparisonBaseHash.clear();
    m_comparisonPath.clear();
    emit comparisonChanged();
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
    checkoutTarget(name, QString());
}

void PierGitClient::checkoutTarget(const QString &target, const QString &tracking)
{
    if (m_repoPath.isEmpty() || target.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString targetRef = target.trimmed();
    const QString trackingRef = tracking.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, targetRef, trackingRef]() {
        bool ok = false;
        QStringList args{QStringLiteral("checkout")};
        if (!trackingRef.isEmpty()) {
            args << QStringLiteral("-b") << targetRef << trackingRef;
        } else {
            args << targetRef;
        }
        const QString out = runGitCommandAt(repoPath, args, &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("checkout")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::createBranch(const QString &name)
{
    createBranchAt(name, QString());
}

void PierGitClient::createBranchAt(const QString &name, const QString &startPoint)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branchName = name.trimmed();
    const QString startRef = startPoint.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, branchName, startRef]() {
        bool ok = false;
        QStringList args{QStringLiteral("branch"), branchName};
        if (!startRef.isEmpty())
            args << startRef;
        const QString output = runGitCommandAt(repoPath, args, &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchCreate")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::deleteBranch(const QString &name)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branchName = name.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, branchName]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("branch"), QStringLiteral("-D"), branchName},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchDelete")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::renameBranch(const QString &oldName, const QString &newName)
{
    if (m_repoPath.isEmpty() || oldName.trimmed().isEmpty() || newName.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString from = oldName.trimmed();
    const QString to = newName.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, from, to]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("branch"), QStringLiteral("-m"), from, to},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchRename")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::renameRemoteBranch(const QString &remoteName, const QString &oldBranch, const QString &newName)
{
    if (m_repoPath.isEmpty() || remoteName.trimmed().isEmpty()
        || oldBranch.trimmed().isEmpty() || newName.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString remote = remoteName.trimmed();
    const QString from = oldBranch.trimmed();
    const QString to = newName.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, remote, from, to]() {
        bool ok = false;
        QString output = runGitCommandAt(repoPath,
                                         {QStringLiteral("push"),
                                          remote,
                                          QStringLiteral("%1:%2").arg(from, to)},
                                         &ok);
        if (ok) {
            bool deleteOk = false;
            const QString deleteOutput = runGitCommandAt(repoPath,
                                                         {QStringLiteral("push"),
                                                          remote,
                                                          QStringLiteral("--delete"),
                                                          from},
                                                         &deleteOk);
            ok = deleteOk;
            output = deleteOk ? output : deleteOutput;
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteBranchRename")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::deleteRemoteBranch(const QString &remoteName, const QString &branchName)
{
    if (m_repoPath.isEmpty() || remoteName.trimmed().isEmpty() || branchName.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString remote = remoteName.trimmed();
    const QString branch = branchName.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, remote, branch]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("push"),
                                                remote,
                                                QStringLiteral("--delete"),
                                                branch},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteBranchDelete")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::mergeBranch(const QString &name)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branchName = name.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, branchName]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("merge"), branchName},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchMerge")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::setBranchTracking(const QString &branchName, const QString &upstream)
{
    if (m_repoPath.isEmpty() || branchName.trimmed().isEmpty() || upstream.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branch = branchName.trimmed();
    const QString ref = upstream.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, branch, ref]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("branch"),
                                                QStringLiteral("--set-upstream-to=%1").arg(ref),
                                                branch},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchTrackingSet")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::unsetBranchTracking(const QString &branchName)
{
    if (m_repoPath.isEmpty() || branchName.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString branch = branchName.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, branch]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("branch"), QStringLiteral("--unset-upstream"), branch},
                                               &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("branchTrackingUnset")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Blame ──────────────────────────────────────────────

void PierGitClient::loadBlame(const QString &path)
{
    if (!m_handle || path.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string p = path.toStdString();
    const QString qmlPath = path;
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, p = std::move(p), qmlPath]() {
        char *json = pier_git_blame(h, p.c_str());
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onBlameResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, qmlPath), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::onBlameResult(quint64 requestId, const QString &path, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) for (const auto &v : doc.array()) list.append(v.toObject().toVariantMap());
    m_blameFilePath = path;
    m_blameLines = list;
    emit blameChanged();
    setBusy(false);
}

void PierGitClient::loadCommitDetail(const QString &hash)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash]() {
        bool ok = false;
        const QString meta = runGitCommandAt(repoPath,
                                             {QStringLiteral("show"), QStringLiteral("--quiet"),
                                              QStringLiteral("--format=%H%x1f%h%x1f%an%x1f%aI%x1f%B"),
                                              commitHash},
                                             &ok);
        if (!ok) {
            if (!selfWeak || (cancelFlag && cancelFlag->load()))
                return;
            QMetaObject::invokeMethod(selfWeak.data(), "onCommitDetailResult", Qt::QueuedConnection,
                Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("{}")));
            return;
        }

        bool statsOk = false;
        const QString stats = runGitCommandAt(repoPath,
                                              {QStringLiteral("show"), QStringLiteral("--shortstat"),
                                               QStringLiteral("--format="), commitHash},
                                              &statsOk);
        bool numstatOk = false;
        const QString numstat = runGitCommandAt(repoPath,
                                                {QStringLiteral("show"), QStringLiteral("--numstat"),
                                                 QStringLiteral("--format="), commitHash},
                                                &numstatOk);
        bool parentsOk = false;
        const QString parents = runGitCommandAt(repoPath,
                                                {QStringLiteral("rev-list"), QStringLiteral("--parents"),
                                                 QStringLiteral("-n"), QStringLiteral("1"), commitHash},
                                                &parentsOk);

        QVariantMap detail = parseCommitDetailDocument(meta,
                                                       statsOk ? stats : QString(),
                                                       numstatOk ? numstat : QString(),
                                                       parentsOk ? parents : QString());
        const QString json = QString::fromUtf8(QJsonDocument::fromVariant(detail).toJson(QJsonDocument::Compact));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onCommitDetailResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::loadCommitFileDiff(const QString &hash, const QString &path)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty() || path.trimmed().isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    const QString relativePath = path.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash, relativePath]() {
        bool ok = false;
        const QString diff = runGitCommandAt(repoPath,
                                             {QStringLiteral("show"),
                                              QStringLiteral("--stat=0"),
                                              QStringLiteral("--format=medium"),
                                              commitHash,
                                              QStringLiteral("--"),
                                              relativePath},
                                             &ok);

        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;

        QMetaObject::invokeMethod(
            selfWeak.data(),
            "onDiffResult",
            Qt::QueuedConnection,
            Q_ARG(quint64, requestId),
            Q_ARG(QString, relativePath),
            Q_ARG(QString, ok ? diff : QString()));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onCommitDetailResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    m_commitDetail = doc.isObject() ? doc.object().toVariantMap() : QVariantMap();
    emit commitDetailChanged();
    setBusy(false);
}

// ─── Tags ───────────────────────────────────────────────

void PierGitClient::loadTags()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h]() {
        char *json = pier_git_tag_list(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onTagsResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::onTagsResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) for (const auto &v : doc.array()) list.append(v.toObject().toVariantMap());
    m_tags = list;
    emit tagsChanged();
}

void PierGitClient::createTag(const QString &name, const QString &message)
{
    createTagAt(name, QString(), message);
}

void PierGitClient::createTagAt(const QString &name, const QString &target, const QString &message)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString tagName = name.trimmed();
    const QString targetRef = target.trimmed();
    const QString annotation = message.trimmed();
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, tagName, targetRef, annotation]() {
        bool ok = false;
        QStringList args;
        if (!annotation.isEmpty()) {
            args << QStringLiteral("tag") << QStringLiteral("-a") << tagName;
            if (!targetRef.isEmpty())
                args << targetRef;
            args << QStringLiteral("-m") << annotation;
        } else {
            args << QStringLiteral("tag") << tagName;
            if (!targetRef.isEmpty())
                args << targetRef;
        }
        const QString out = runGitCommandAt(repoPath, args, &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("tagCreate")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::deleteTag(const QString &name)
{
    if (!m_handle || name.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string n = name.toStdString();
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, n = std::move(n)]() {
        char *r = pier_git_tag_delete(h, n.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("tagDelete")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::pushTag(const QString &name)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString tagName = name.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, tagName]() {
        bool ok = false;
        QString output = runGitCommandAt(repoPath,
                                         {QStringLiteral("push"), QStringLiteral("origin"), tagName},
                                         &ok);
        if (!ok) {
            output = runGitCommandAt(repoPath,
                                     {QStringLiteral("push"), QStringLiteral("--tags")},
                                     &ok);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("tagPush")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::pushAllTags()
{
    if (m_repoPath.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        bool ok = false;
        QString output = runGitCommandAt(repoPath,
                                         {QStringLiteral("push"), QStringLiteral("--tags")},
                                         &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("tagPushAll")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Remotes ────────────────────────────────────────────

void PierGitClient::loadRemotes()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h]() {
        char *json = pier_git_remote_list(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onRemotesResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::onRemotesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) for (const auto &v : doc.array()) list.append(v.toObject().toVariantMap());
    m_remotes = list;
    emit remotesChanged();
}

void PierGitClient::addRemote(const QString &name, const QString &url)
{
    if (!m_handle || name.isEmpty() || url.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string n = name.toStdString(), u = url.toStdString();
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, n = std::move(n), u = std::move(u)]() {
        char *r = pier_git_remote_add(h, n.c_str(), u.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (ok && out.trimmed().isEmpty())
            out = QStringLiteral("Added remote '%1'.").arg(QString::fromStdString(n));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteAdd")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::setRemoteUrl(const QString &name, const QString &url)
{
    if (m_repoPath.isEmpty() || name.trimmed().isEmpty() || url.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString remoteName = name.trimmed();
    const QString remoteUrl = url.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, remoteName, remoteUrl]() {
        bool ok = false;
        QString output = runGitCommandAt(repoPath,
                                         {QStringLiteral("remote"), QStringLiteral("set-url"), remoteName, remoteUrl},
                                         &ok);
        if (ok && output.trimmed().isEmpty())
            output = QStringLiteral("Updated remote '%1'.").arg(remoteName);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteSetUrl")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::removeRemote(const QString &name)
{
    if (!m_handle || name.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string n = name.toStdString();
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, n = std::move(n)]() {
        char *r = pier_git_remote_remove(h, n.c_str());
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (ok && out.trimmed().isEmpty())
            out = QStringLiteral("Removed remote '%1'.").arg(QString::fromStdString(n));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteRemove")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::fetchRemote(const QString &name)
{
    if (m_repoPath.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString remoteName = name.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, remoteName]() {
        bool ok = false;
        QStringList args{QStringLiteral("fetch")};
        if (!remoteName.isEmpty())
            args.append(remoteName);
        QString output = runGitCommandAt(repoPath, args, &ok);
        if (ok && output.trimmed().isEmpty())
            output = remoteName.isEmpty()
                    ? QStringLiteral("Fetched all remotes.")
                    : QStringLiteral("Fetched remote '%1'.").arg(remoteName);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("remoteFetch")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

// ─── Config ─────────────────────────────────────────────

void PierGitClient::loadConfig()
{
    if (!m_handle) return;
    const quint64 requestId = ++m_nextRequestId;
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h]() {
        char *json = pier_git_config_list(h);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_git_free_string(json);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onConfigResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::onConfigResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) for (const auto &v : doc.array()) list.append(v.toObject().toVariantMap());
    m_configEntries = list;
    emit configChanged();
}

void PierGitClient::setConfigValue(const QString &key, const QString &value, bool global)
{
    if (!m_handle || key.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string k = key.toStdString(), v = value.toStdString();
    int g = global ? 1 : 0;
    auto w = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, k = std::move(k), v = std::move(v), g]() {
        char *r = pier_git_config_set(h, k.c_str(), v.c_str(), g);
        QString out = r ? QString::fromUtf8(r) : QString();
        if (r) pier_git_free_string(r);
        bool ok = !out.contains(QStringLiteral("\"error\""));
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("configSet")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(w));
}

void PierGitClient::unsetConfigValue(const QString &key, bool global)
{
    if (!m_handle || key.isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    ::PierGit *h = m_handle;
    std::string k = key.toStdString();
    const int g = global ? 1 : 0;
    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, h, k = std::move(k), g]() {
        const QString out = decodeGitOutput(pier_git_config_unset(h, k.c_str(), g));
        const bool ok = gitOutputOk(out);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("configUnset")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::resetToCommit(const QString &hash, const QString &mode)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    const QString resetMode = mode.trimmed().isEmpty() ? QStringLiteral("mixed") : mode.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash, resetMode]() {
        bool ok = false;
        const QString out = runGitCommandAt(repoPath,
                                            {QStringLiteral("reset"),
                                             QStringLiteral("--%1").arg(resetMode),
                                             commitHash},
                                            &ok);
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("commitReset")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::amendHeadCommitMessage(const QString &hash, const QString &message)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty() || message.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    const QString newMessage = message.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash, newMessage]() {
        bool ok = false;
        const QString headHash = runGitCommandAt(repoPath, {QStringLiteral("rev-parse"), QStringLiteral("HEAD")}, &ok).trimmed();
        QString out;
        if (!ok || headHash != commitHash) {
            ok = false;
            out = QStringLiteral("Only the current HEAD commit can be amended.");
        } else {
            out = runGitCommandAt(repoPath,
                                  {QStringLiteral("commit"), QStringLiteral("--amend"),
                                   QStringLiteral("-m"), newMessage},
                                  &ok);
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("commitEditMessage")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::dropCommit(const QString &hash, const QString &parentHash)
{
    if (m_repoPath.isEmpty() || hash.trimmed().isEmpty()) return;
    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QString commitHash = hash.trimmed();
    const QString parent = parentHash.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, commitHash, parent]() {
        bool ok = false;
        const QString headHash = runGitCommandAt(repoPath, {QStringLiteral("rev-parse"), QStringLiteral("HEAD")}, &ok).trimmed();
        QString out;
        if (!ok) {
            out = QStringLiteral("Failed to resolve HEAD");
        } else if (headHash == commitHash) {
            out = runGitCommandAt(repoPath,
                                  {QStringLiteral("reset"), QStringLiteral("--hard"), QStringLiteral("HEAD~1")},
                                  &ok);
        } else if (!parent.isEmpty()) {
            out = runGitCommandAt(repoPath,
                                  {QStringLiteral("rebase"), QStringLiteral("--onto"), parent, commitHash, QStringLiteral("HEAD")},
                                  &ok);
        } else {
            ok = false;
            out = QStringLiteral("Missing parent commit");
        }
        if (!selfWeak || (cancelFlag && cancelFlag->load())) return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("commitDrop")),
            Q_ARG(bool, ok), Q_ARG(QString, out));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::loadRebasePlan(int count)
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const int itemCount = qMax(1, count);

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, itemCount]() {
        bool mergeOk = false;
        const QString rebaseMergePath = gitPathForRepo(repoPath, QStringLiteral("rebase-merge"), &mergeOk);
        bool applyOk = false;
        const QString rebaseApplyPath = gitPathForRepo(repoPath, QStringLiteral("rebase-apply"), &applyOk);
        const bool inProgress = (mergeOk && QFileInfo::exists(rebaseMergePath))
                                || (applyOk && QFileInfo::exists(rebaseApplyPath));

        QJsonArray items;
        if (inProgress) {
            const QString todoPath = QFileInfo::exists(QDir(rebaseMergePath).filePath(QStringLiteral("git-rebase-todo")))
                    ? QDir(rebaseMergePath).filePath(QStringLiteral("git-rebase-todo"))
                    : QDir(rebaseApplyPath).filePath(QStringLiteral("git-rebase-todo"));
            QFile todoFile(todoPath);
            if (todoFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
                const QStringList lines = QString::fromUtf8(todoFile.readAll()).split(QLatin1Char('\n'));
                for (const QString &line : lines) {
                    const QVariantMap item = parseRebaseLine(line);
                    if (!item.isEmpty())
                        items.append(QJsonObject::fromVariantMap(item));
                }
            }
        } else {
            bool ok = false;
            const QString output = runGitCommandAt(repoPath,
                                                   {QStringLiteral("log"),
                                                    QStringLiteral("--format=%H%x1f%s"),
                                                    QStringLiteral("-n"),
                                                    QString::number(itemCount),
                                                    QStringLiteral("HEAD")},
                                                   &ok);
            if (ok) {
                const QStringList lines = output.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
                for (const QString &line : lines) {
                    const int sep = line.indexOf(QChar(0x1f));
                    if (sep <= 0)
                        continue;
                    const QString hash = line.left(sep).trimmed();
                    const QString message = line.mid(sep + 1).trimmed();
                    QJsonObject item;
                    item.insert(QStringLiteral("id"), hash);
                    item.insert(QStringLiteral("action"), QStringLiteral("pick"));
                    item.insert(QStringLiteral("hash"), hash);
                    item.insert(QStringLiteral("shortHash"), hash.left(7));
                    item.insert(QStringLiteral("message"), message);
                    items.append(item);
                }
            }
        }

        const QString json = QString::fromUtf8(QJsonDocument(items).toJson(QJsonDocument::Compact));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onRebasePlanResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(bool, inProgress), Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onRebasePlanResult(quint64 requestId, bool inProgress, const QString &json)
{
    if (requestId != m_nextRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList items;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array())
            items.append(value.toObject().toVariantMap());
    }
    m_rebaseInProgress = inProgress;
    m_rebaseTodoItems = items;
    emit rebaseChanged();
    setBusy(false);
}

void PierGitClient::executeRebase(const QVariantList &items, const QString &onto)
{
    if (m_repoPath.isEmpty() || items.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);

    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    const QVariantList todoItems = items;
    const QString ontoRef = onto.trimmed();

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, todoItems, ontoRef]() {
        bool ok = false;
        QString base = ontoRef;
        if (base.isEmpty()) {
            const QVariantMap oldestVisible = todoItems.isEmpty() ? QVariantMap() : todoItems.constLast().toMap();
            if (!oldestVisible.value(QStringLiteral("hash")).toString().isEmpty())
                base = oldestVisible.value(QStringLiteral("hash")).toString() + QStringLiteral("~1");
        }
        if (base.isEmpty()) {
            if (!selfWeak || (cancelFlag && cancelFlag->load()))
                return;
            QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
                Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("rebaseExecute")),
                Q_ARG(bool, false), Q_ARG(QString, QStringLiteral("Missing rebase base")));
            return;
        }

        QTemporaryFile todoFile(QDir::tempPath() + QStringLiteral("/pierx-rebase-todo-XXXXXX"));
        QTemporaryFile scriptFile(QDir::tempPath() + QStringLiteral("/pierx-sequence-editor-XXXXXX.sh"));
        if (!todoFile.open() || !scriptFile.open()) {
            if (!selfWeak || (cancelFlag && cancelFlag->load()))
                return;
            QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
                Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("rebaseExecute")),
                Q_ARG(bool, false), Q_ARG(QString, QStringLiteral("Failed to prepare rebase script")));
            return;
        }

        QString todoText;
        for (int i = todoItems.size() - 1; i >= 0; --i) {
            const QVariantMap item = todoItems.at(i).toMap();
            const QString action = item.value(QStringLiteral("action")).toString().trimmed().isEmpty()
                    ? QStringLiteral("pick")
                    : item.value(QStringLiteral("action")).toString().trimmed();
            const QString hash = item.value(QStringLiteral("hash")).toString().trimmed();
            const QString message = item.value(QStringLiteral("message")).toString().trimmed();
            if (hash.isEmpty())
                continue;
            todoText += QStringLiteral("%1 %2 %3\n").arg(action, hash, message);
        }
        todoFile.write(todoText.toUtf8());
        todoFile.flush();

        const QString scriptText = QStringLiteral("#!/bin/sh\ncat \"%1\" > \"$1\"\n")
                .arg(todoFile.fileName());
        scriptFile.write(scriptText.toUtf8());
        scriptFile.flush();
        scriptFile.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);

        QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
        env.insert(QStringLiteral("GIT_SEQUENCE_EDITOR"), scriptFile.fileName());
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("rebase"), QStringLiteral("-i"), base},
                                               &ok,
                                               env,
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("rebaseExecute")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::abortRebase()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("rebase"), QStringLiteral("--abort")},
                                               &ok,
                                               QProcessEnvironment::systemEnvironment(),
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("rebaseAbort")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::continueRebase()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("rebase"), QStringLiteral("--continue")},
                                               &ok,
                                               QProcessEnvironment::systemEnvironment(),
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("rebaseContinue")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::loadSubmodules()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        const QHash<QString, QString> urlsByPath = parseGitmodulesUrls(repoPath);
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("submodule"), QStringLiteral("status"), QStringLiteral("--recursive")},
                                               &ok);
        QJsonArray rows;
        if (ok) {
            const QStringList lines = output.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
            for (const QString &rawLine : lines) {
                const QString line = rawLine;
                if (line.isEmpty())
                    continue;
                const QString statusSymbol = line.left(1);
                const QString rest = line.mid(1).trimmed();
                const QStringList parts = rest.split(QLatin1Char(' '), Qt::SkipEmptyParts);
                if (parts.size() < 2)
                    continue;
                const QString hash = parts.at(0).trimmed();
                const QString path = parts.at(1).trimmed();
                QString status = QStringLiteral("ok");
                if (statusSymbol == QStringLiteral("-"))
                    status = QStringLiteral("uninitialized");
                else if (statusSymbol == QStringLiteral("+"))
                    status = QStringLiteral("modified");
                else if (statusSymbol == QStringLiteral("U"))
                    status = QStringLiteral("conflict");

                QJsonObject row;
                row.insert(QStringLiteral("path"), path);
                row.insert(QStringLiteral("commitHash"), hash);
                row.insert(QStringLiteral("shortHash"), hash.left(7));
                row.insert(QStringLiteral("status"), status);
                row.insert(QStringLiteral("statusSymbol"), statusSymbol);
                row.insert(QStringLiteral("url"), urlsByPath.value(path));
                rows.append(row);
            }
        }
        const QString json = QString::fromUtf8(QJsonDocument(rows).toJson(QJsonDocument::Compact));
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onSubmodulesResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, json));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::onSubmodulesResult(quint64 requestId, const QString &json)
{
    if (requestId != m_nextRequestId)
        return;

    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList rows;
    if (doc.isArray()) {
        for (const QJsonValue &value : doc.array())
            rows.append(value.toObject().toVariantMap());
    }
    m_submodules = rows;
    emit submodulesChanged();
    setBusy(false);
}

void PierGitClient::initSubmodules()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;
    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("submodule"), QStringLiteral("init")},
                                               &ok,
                                               QProcessEnvironment::systemEnvironment(),
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("submoduleInit")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::updateSubmodules(bool recursive)
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath, recursive]() {
        bool ok = false;
        QStringList args{QStringLiteral("submodule"), QStringLiteral("update"), QStringLiteral("--init")};
        if (recursive)
            args << QStringLiteral("--recursive");
        const QString output = runGitCommandAt(repoPath,
                                               args,
                                               &ok,
                                               QProcessEnvironment::systemEnvironment(),
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("submoduleUpdate")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

void PierGitClient::syncSubmodules()
{
    if (m_repoPath.isEmpty())
        return;

    const quint64 requestId = ++m_nextRequestId;
    setBusy(true);
    QPointer<PierGitClient> selfWeak(this);
    auto cancelFlag = m_cancelFlag;
    const QString repoPath = m_repoPath;

    auto worker = std::make_unique<std::thread>([selfWeak, cancelFlag, requestId, repoPath]() {
        bool ok = false;
        const QString output = runGitCommandAt(repoPath,
                                               {QStringLiteral("submodule"), QStringLiteral("sync"), QStringLiteral("--recursive")},
                                               &ok,
                                               QProcessEnvironment::systemEnvironment(),
                                               120000);
        if (!selfWeak || (cancelFlag && cancelFlag->load()))
            return;
        QMetaObject::invokeMethod(selfWeak.data(), "onOpResult", Qt::QueuedConnection,
            Q_ARG(quint64, requestId), Q_ARG(QString, QStringLiteral("submoduleSync")),
            Q_ARG(bool, ok), Q_ARG(QString, output));
    });
    m_workers.push_back(std::move(worker));
}

int PierGitClient::conflictFileIndexForPath(const QString &path) const
{
    const QString normalized = QDir::cleanPath(path);
    for (int i = 0; i < m_conflictFiles.size(); ++i) {
        const QVariantMap file = m_conflictFiles.at(i).toMap();
        const QString relative = QDir::cleanPath(file.value(QStringLiteral("path")).toString());
        const QString absolute = QDir::cleanPath(file.value(QStringLiteral("absolutePath")).toString());
        if (normalized == relative || normalized == absolute)
            return i;
    }
    return -1;
}

bool PierGitClient::writeResolvedConflictFile(const QVariantMap &file, const QString &defaultResolution, QString *errorMessage) const
{
    const QString absolutePath = file.value(QStringLiteral("absolutePath")).toString();
    if (absolutePath.isEmpty()) {
        if (errorMessage) *errorMessage = QStringLiteral("Missing file path");
        return false;
    }

    QFile sourceFile(absolutePath);
    if (!sourceFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
        if (errorMessage) *errorMessage = QStringLiteral("Failed to read %1").arg(absolutePath);
        return false;
    }
    const QString content = QString::fromUtf8(sourceFile.readAll());
    sourceFile.close();

    const QVariantList hunks = file.value(QStringLiteral("conflicts")).toList();
    const QStringList lines = content.split(QLatin1Char('\n'));
    QStringList result;
    int i = 0;
    int hunkIndex = 0;

    while (i < lines.size()) {
        if (!lines.at(i).startsWith(QStringLiteral("<<<<<<<"))) {
            result.append(lines.at(i));
            ++i;
            continue;
        }

        QStringList oursLines;
        QStringList theirsLines;
        ++i;
        while (i < lines.size() && !lines.at(i).startsWith(QStringLiteral("======="))) {
            oursLines.append(lines.at(i));
            ++i;
        }
        if (i < lines.size())
            ++i;
        while (i < lines.size() && !lines.at(i).startsWith(QStringLiteral(">>>>>>>"))) {
            theirsLines.append(lines.at(i));
            ++i;
        }

        const QVariantMap hunk = hunkIndex < hunks.size() ? hunks.at(hunkIndex).toMap() : QVariantMap();
        const QString resolution = hunk.value(QStringLiteral("resolution")).toString().isEmpty()
                ? defaultResolution
                : hunk.value(QStringLiteral("resolution")).toString();

        if (resolution == QStringLiteral("theirs")) {
            result.append(theirsLines);
        } else if (resolution == QStringLiteral("both")) {
            result.append(oursLines);
            result.append(theirsLines);
        } else {
            result.append(oursLines);
        }

        ++hunkIndex;
        if (i < lines.size())
            ++i;
    }

    QSaveFile outFile(absolutePath);
    if (!outFile.open(QIODevice::WriteOnly | QIODevice::Text)) {
        if (errorMessage) *errorMessage = QStringLiteral("Failed to write %1").arg(absolutePath);
        return false;
    }
    outFile.write(result.join(QLatin1Char('\n')).toUtf8());
    if (!outFile.commit()) {
        if (errorMessage) *errorMessage = QStringLiteral("Failed to save %1").arg(absolutePath);
        return false;
    }
    return true;
}

void PierGitClient::detectConflicts()
{
    if (m_repoPath.isEmpty()) {
        if (!m_conflictFiles.isEmpty()) {
            m_conflictFiles.clear();
            emit conflictFilesChanged();
        }
        return;
    }

    bool ok = false;
    const QString output = runGitCommandAt(m_repoPath,
                                           {QStringLiteral("diff"),
                                            QStringLiteral("--name-only"),
                                            QStringLiteral("--diff-filter=U")},
                                           &ok);
    QVariantList files;
    if (ok) {
        const QDir repoDir(m_repoPath);
        const QStringList entries = output.split(QLatin1Char('\n'), Qt::SkipEmptyParts);
        for (const QString &entry : entries) {
            const QString relativePath = QDir::cleanPath(entry.trimmed());
            if (relativePath.isEmpty())
                continue;
            const QString absolutePath = repoDir.filePath(relativePath);
            QFile file(absolutePath);
            if (!file.open(QIODevice::ReadOnly | QIODevice::Text))
                continue;
            const QString content = QString::fromUtf8(file.readAll());
            file.close();

            const QVariantList hunks = parseConflictHunksFromContent(content);
            if (hunks.isEmpty())
                continue;

            QVariantMap row;
            row[QStringLiteral("name")] = QFileInfo(relativePath).fileName();
            row[QStringLiteral("path")] = relativePath;
            row[QStringLiteral("absolutePath")] = absolutePath;
            row[QStringLiteral("conflicts")] = hunks;
            row[QStringLiteral("conflictCount")] = hunks.size();
            files.append(row);
        }
    }

    m_conflictFiles = files;
    emit conflictFilesChanged();
}

void PierGitClient::resolveConflict(const QString &path, int hunkIndex, const QString &resolution)
{
    const int index = conflictFileIndexForPath(path);
    if (index < 0 || hunkIndex < 0)
        return;

    QVariantMap file = m_conflictFiles.at(index).toMap();
    QVariantList hunks = file.value(QStringLiteral("conflicts")).toList();
    if (hunkIndex >= hunks.size())
        return;

    QVariantMap hunk = hunks.at(hunkIndex).toMap();
    hunk[QStringLiteral("resolution")] = resolution.trimmed();
    hunks[hunkIndex] = hunk;
    file[QStringLiteral("conflicts")] = hunks;
    m_conflictFiles[index] = file;
    emit conflictFilesChanged();
}

void PierGitClient::acceptAllOurs(const QString &path)
{
    const int index = conflictFileIndexForPath(path);
    if (index < 0)
        return;

    QVariantMap file = m_conflictFiles.at(index).toMap();
    QVariantList hunks = file.value(QStringLiteral("conflicts")).toList();
    for (int i = 0; i < hunks.size(); ++i) {
        QVariantMap hunk = hunks.at(i).toMap();
        hunk[QStringLiteral("resolution")] = QStringLiteral("ours");
        hunks[i] = hunk;
    }
    file[QStringLiteral("conflicts")] = hunks;
    m_conflictFiles[index] = file;
    emit conflictFilesChanged();

    QString error;
    const bool ok = writeResolvedConflictFile(file, QStringLiteral("ours"), &error);
    emit operationFinished(QStringLiteral("conflictResolve"),
                           ok,
                           ok ? QStringLiteral("Accepted ours for %1").arg(file.value(QStringLiteral("name")).toString())
                              : error);
}

void PierGitClient::acceptAllTheirs(const QString &path)
{
    const int index = conflictFileIndexForPath(path);
    if (index < 0)
        return;

    QVariantMap file = m_conflictFiles.at(index).toMap();
    QVariantList hunks = file.value(QStringLiteral("conflicts")).toList();
    for (int i = 0; i < hunks.size(); ++i) {
        QVariantMap hunk = hunks.at(i).toMap();
        hunk[QStringLiteral("resolution")] = QStringLiteral("theirs");
        hunks[i] = hunk;
    }
    file[QStringLiteral("conflicts")] = hunks;
    m_conflictFiles[index] = file;
    emit conflictFilesChanged();

    QString error;
    const bool ok = writeResolvedConflictFile(file, QStringLiteral("theirs"), &error);
    emit operationFinished(QStringLiteral("conflictResolve"),
                           ok,
                           ok ? QStringLiteral("Accepted theirs for %1").arg(file.value(QStringLiteral("name")).toString())
                              : error);
}

void PierGitClient::markConflictResolved(const QString &path)
{
    const int index = conflictFileIndexForPath(path);
    if (index < 0 || m_repoPath.isEmpty())
        return;

    const QVariantMap file = m_conflictFiles.at(index).toMap();
    QString error;
    bool ok = writeResolvedConflictFile(file, QStringLiteral("ours"), &error);
    if (ok) {
        const QString relativePath = file.value(QStringLiteral("path")).toString();
        QString gitOut = runGitCommandAt(m_repoPath, {QStringLiteral("add"), relativePath}, &ok);
        if (!ok)
            error = gitOut;
    }

    emit operationFinished(QStringLiteral("conflictResolve"),
                           ok,
                           ok ? QStringLiteral("Marked %1 as resolved").arg(file.value(QStringLiteral("name")).toString())
                              : error);
    if (ok) {
        detectConflicts();
        refresh();
    }
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
    m_commits.clear();
    m_graphRows.clear();
    m_graphBranches.clear();
    m_graphAuthors.clear();
    m_graphRepoFiles.clear();
    m_graphGitUserName.clear();
    m_stashes.clear();
    m_branches.clear();
    m_blameLines.clear();
    m_blameFilePath.clear();
    m_comparisonFiles.clear();
    m_comparisonDiff.clear();
    m_comparisonBaseHash.clear();
    m_comparisonPath.clear();
    m_commitDetail.clear();
    m_tags.clear();
    m_remotes.clear();
    m_configEntries.clear();
    m_conflictFiles.clear();
    m_rebaseTodoItems.clear();
    m_rebaseInProgress = false;
    m_submodules.clear();
    setStatus(Idle);
    emit repoChanged();
    emit branchChanged();
    emit filesChanged();
    emit diffChanged();
    emit commitsChanged();
    emit graphChanged();
    emit graphMetadataChanged();
    emit stashesChanged();
    emit branchesChanged();
    emit blameChanged();
    emit comparisonChanged();
    emit commitDetailChanged();
    emit tagsChanged();
    emit remotesChanged();
    emit configChanged();
    emit conflictFilesChanged();
    emit rebaseChanged();
    emit submodulesChanged();
}
