// ─────────────────────────────────────────────────────────
// PierGitClient — Qt-side Git panel
// ─────────────────────────────────────────────────────────
//
// QObject wrapper around the pier_git_* FFI surface. Exposes
// repository status, branch info, staging, commit, and diff
// operations to QML.
//
// Threading
// ─────────
//   Every pier_git_* call is blocking. We dispatch each
//   request on a dedicated std::thread per in-flight op and
//   post the result back via QMetaObject::invokeMethod, same
//   pattern as PierMySqlClient / PierRedisClient.

#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantList>
#include <qqml.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

// Forward-declare the opaque Rust handle.
struct PierGit;

/// Panel backend. One instance per Git panel.
class PierGitClient : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierGitClient)

public:
    enum Status {
        Idle = 0,
        Loading = 1,
        Ready = 2,
        Failed = 3
    };
    Q_ENUM(Status)

    Q_PROPERTY(Status status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY statusChanged FINAL)

    // Repository info
    Q_PROPERTY(QString repoPath READ repoPath NOTIFY repoChanged FINAL)
    Q_PROPERTY(bool isGitRepo READ isGitRepo NOTIFY repoChanged FINAL)

    // Branch info
    Q_PROPERTY(QString currentBranch READ currentBranch NOTIFY branchChanged FINAL)
    Q_PROPERTY(QString trackingBranch READ trackingBranch NOTIFY branchChanged FINAL)
    Q_PROPERTY(int aheadCount READ aheadCount NOTIFY branchChanged FINAL)
    Q_PROPERTY(int behindCount READ behindCount NOTIFY branchChanged FINAL)

    // File lists
    Q_PROPERTY(QVariantList stagedFiles READ stagedFiles NOTIFY filesChanged FINAL)
    Q_PROPERTY(QVariantList unstagedFiles READ unstagedFiles NOTIFY filesChanged FINAL)

    // Diff
    Q_PROPERTY(QString diffText READ diffText NOTIFY diffChanged FINAL)
    Q_PROPERTY(QString diffPath READ diffPath NOTIFY diffChanged FINAL)

    // History
    Q_PROPERTY(QVariantList commits READ commits NOTIFY commitsChanged FINAL)

    // Stash
    Q_PROPERTY(QVariantList stashes READ stashes NOTIFY stashesChanged FINAL)

    // Branches
    Q_PROPERTY(QStringList branches READ branches NOTIFY branchesChanged FINAL)

    // Busy flag
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)

    explicit PierGitClient(QObject *parent = nullptr);
    ~PierGitClient() override;

    PierGitClient(const PierGitClient &) = delete;
    PierGitClient &operator=(const PierGitClient &) = delete;

    // Property getters
    Status status() const { return m_status; }
    QString errorMessage() const { return m_errorMessage; }
    QString repoPath() const { return m_repoPath; }
    bool isGitRepo() const { return m_handle != nullptr; }
    QString currentBranch() const { return m_currentBranch; }
    QString trackingBranch() const { return m_trackingBranch; }
    int aheadCount() const { return m_aheadCount; }
    int behindCount() const { return m_behindCount; }
    QVariantList stagedFiles() const { return m_stagedFiles; }
    QVariantList unstagedFiles() const { return m_unstagedFiles; }
    QString diffText() const { return m_diffText; }
    QString diffPath() const { return m_diffPath; }
    QVariantList commits() const { return m_commits; }
    QVariantList stashes() const { return m_stashes; }
    QStringList branches() const { return m_branches; }
    bool busy() const { return m_busy; }

public slots:
    /// Open a repository. Resolves git root from the given path.
    void open(const QString &path);

    /// Reload status + branch info.
    void refresh();

    /// Stage a specific file.
    void stageFile(const QString &path);

    /// Unstage a specific file.
    void unstageFile(const QString &path);

    /// Stage all changes.
    void stageAll();

    /// Unstage all changes.
    void unstageAll();

    /// Discard working tree changes for a file.
    void discardFile(const QString &path);

    /// Load diff for a specific file.
    void loadDiff(const QString &path, bool staged);

    /// Create a commit with the given message.
    void commit(const QString &message);

    /// Push to remote.
    void push();

    /// Pull from remote.
    void pull();

    /// Load commit history.
    void loadHistory(int limit = 100);

    /// Load stash list.
    void loadStashes();

    /// Stash current changes.
    void stashPush(const QString &message);

    /// Apply a stash by index.
    void stashApply(const QString &index);

    /// Pop a stash by index.
    void stashPop(const QString &index);

    /// Drop a stash by index.
    void stashDrop(const QString &index);

    /// Load local branch list.
    void loadBranches();

    /// Switch to a branch.
    void checkoutBranch(const QString &name);

    /// Tear down. Releases the handle.
    void close();

signals:
    void statusChanged();
    void repoChanged();
    void branchChanged();
    void filesChanged();
    void diffChanged();
    void commitsChanged();
    void stashesChanged();
    void branchesChanged();
    void busyChanged();
    void operationFinished(const QString &operation, bool success, const QString &message);

private slots:
    void onOpenResult(quint64 requestId, void *handle, const QString &repoPath, const QString &error);
    void onStatusResult(quint64 requestId, const QString &json);
    void onBranchResult(quint64 requestId, const QString &json);
    void onDiffResult(quint64 requestId, const QString &path, const QString &text);
    void onHistoryResult(quint64 requestId, const QString &json);
    void onStashResult(quint64 requestId, const QString &json);
    void onBranchesResult(quint64 requestId, const QString &json);
    void onOpResult(quint64 requestId, const QString &operation, bool success, const QString &message);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void parseStatusJson(const QString &json);
    void parseBranchJson(const QString &json);

    ::PierGit *m_handle = nullptr;

    Status  m_status = Status::Idle;
    QString m_errorMessage;
    QString m_repoPath;
    bool    m_busy = false;

    // Branch
    QString m_currentBranch;
    QString m_trackingBranch;
    int     m_aheadCount = 0;
    int     m_behindCount = 0;

    // Files
    QVariantList m_stagedFiles;
    QVariantList m_unstagedFiles;

    // Diff
    QString m_diffText;
    QString m_diffPath;

    // History
    QVariantList m_commits;

    // Stash
    QVariantList m_stashes;

    // Branches
    QStringList m_branches;

    quint64 m_nextRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
