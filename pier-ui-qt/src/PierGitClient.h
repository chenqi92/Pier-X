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
    Q_PROPERTY(QVariantList graphRows READ graphRows NOTIFY graphChanged FINAL)
    Q_PROPERTY(QStringList graphBranches READ graphBranches NOTIFY graphMetadataChanged FINAL)
    Q_PROPERTY(QStringList graphAuthors READ graphAuthors NOTIFY graphMetadataChanged FINAL)
    Q_PROPERTY(QStringList graphRepoFiles READ graphRepoFiles NOTIFY graphMetadataChanged FINAL)
    Q_PROPERTY(QString graphGitUserName READ graphGitUserName NOTIFY graphMetadataChanged FINAL)

    // Stash
    Q_PROPERTY(QVariantList stashes READ stashes NOTIFY stashesChanged FINAL)

    // Branches
    Q_PROPERTY(QStringList branches READ branches NOTIFY branchesChanged FINAL)

    // Blame
    Q_PROPERTY(QVariantList blameLines READ blameLines NOTIFY blameChanged FINAL)
    Q_PROPERTY(QString blameFilePath READ blameFilePath NOTIFY blameChanged FINAL)

    // Compare
    Q_PROPERTY(QVariantList comparisonFiles READ comparisonFiles NOTIFY comparisonChanged FINAL)
    Q_PROPERTY(QString comparisonDiff READ comparisonDiff NOTIFY comparisonChanged FINAL)
    Q_PROPERTY(QString comparisonBaseHash READ comparisonBaseHash NOTIFY comparisonChanged FINAL)
    Q_PROPERTY(QString comparisonPath READ comparisonPath NOTIFY comparisonChanged FINAL)

    // Commit detail
    Q_PROPERTY(QVariantMap commitDetail READ commitDetail NOTIFY commitDetailChanged FINAL)

    // Tags
    Q_PROPERTY(QVariantList tags READ tags NOTIFY tagsChanged FINAL)

    // Remotes
    Q_PROPERTY(QVariantList remotes READ remotes NOTIFY remotesChanged FINAL)

    // Config
    Q_PROPERTY(QVariantList configEntries READ configEntries NOTIFY configChanged FINAL)
    Q_PROPERTY(QVariantList conflictFiles READ conflictFiles NOTIFY conflictFilesChanged FINAL)

    // Rebase / submodules
    Q_PROPERTY(QVariantList rebaseTodoItems READ rebaseTodoItems NOTIFY rebaseChanged FINAL)
    Q_PROPERTY(bool rebaseInProgress READ rebaseInProgress NOTIFY rebaseChanged FINAL)
    Q_PROPERTY(QVariantList submodules READ submodules NOTIFY submodulesChanged FINAL)

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
    QVariantList graphRows() const { return m_graphRows; }
    QStringList graphBranches() const { return m_graphBranches; }
    QStringList graphAuthors() const { return m_graphAuthors; }
    QStringList graphRepoFiles() const { return m_graphRepoFiles; }
    QString graphGitUserName() const { return m_graphGitUserName; }
    QVariantList stashes() const { return m_stashes; }
    QStringList branches() const { return m_branches; }
    QVariantList blameLines() const { return m_blameLines; }
    QString blameFilePath() const { return m_blameFilePath; }
    QVariantList comparisonFiles() const { return m_comparisonFiles; }
    QString comparisonDiff() const { return m_comparisonDiff; }
    QString comparisonBaseHash() const { return m_comparisonBaseHash; }
    QString comparisonPath() const { return m_comparisonPath; }
    QVariantMap commitDetail() const { return m_commitDetail; }
    QVariantList tags() const { return m_tags; }
    QVariantList remotes() const { return m_remotes; }
    QVariantList configEntries() const { return m_configEntries; }
    QVariantList conflictFiles() const { return m_conflictFiles; }
    QVariantList rebaseTodoItems() const { return m_rebaseTodoItems; }
    bool rebaseInProgress() const { return m_rebaseInProgress; }
    QVariantList submodules() const { return m_submodules; }
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
    void commitAndPush(const QString &message);

    /// Push to remote.
    void push();

    /// Pull from remote.
    void pull();

    /// Load commit history.
    void loadHistory(int limit = 100);
    void loadGraphHistory(int limit = 180,
                          int skip = 0,
                          const QString &branch = QString(),
                          const QString &author = QString(),
                          const QString &searchText = QString(),
                          bool firstParent = false,
                          bool noMerges = false,
                          qint64 afterTimestamp = 0,
                          const QString &pathFilter = QString(),
                          bool topoOrder = true,
                          bool showLongEdges = true);
    void loadGraphMetadata();
    void loadComparisonFiles(const QString &hash);
    void loadComparisonDiff(const QString &hash, const QString &path);
    void clearComparison();

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
    void checkoutTarget(const QString &target, const QString &tracking = QString());
    void createBranch(const QString &name);
    void createBranchAt(const QString &name, const QString &startPoint);
    void deleteBranch(const QString &name);
    void renameBranch(const QString &oldName, const QString &newName);
    void renameRemoteBranch(const QString &remoteName, const QString &oldBranch, const QString &newName);
    void deleteRemoteBranch(const QString &remoteName, const QString &branchName);
    void mergeBranch(const QString &name);
    void setBranchTracking(const QString &branchName, const QString &upstream);
    void unsetBranchTracking(const QString &branchName);

    /// Load blame for a file.
    void loadBlame(const QString &path);
    void loadCommitDetail(const QString &hash);
    void loadCommitFileDiff(const QString &hash, const QString &path);

    /// Load tags.
    void loadTags();
    void createTag(const QString &name, const QString &message);
    void deleteTag(const QString &name);
    void pushTag(const QString &name);
    void pushAllTags();

    /// Load remotes.
    void loadRemotes();
    void addRemote(const QString &name, const QString &url);
    void setRemoteUrl(const QString &name, const QString &url);
    void removeRemote(const QString &name);
    void fetchRemote(const QString &name = QString());

    /// Load git config.
    void loadConfig();
    void setConfigValue(const QString &key, const QString &value, bool global);
    void unsetConfigValue(const QString &key, bool global);

    /// Interactive rebase planning.
    void loadRebasePlan(int count = 10);
    void executeRebase(const QVariantList &items, const QString &onto = QString());
    void abortRebase();
    void continueRebase();

    /// Submodules.
    void loadSubmodules();
    void initSubmodules();
    void updateSubmodules(bool recursive = true);
    void syncSubmodules();

    /// Commit history actions.
    void createTagAt(const QString &name, const QString &target, const QString &message = QString());
    void resetToCommit(const QString &hash, const QString &mode);
    void amendHeadCommitMessage(const QString &hash, const QString &message);
    void dropCommit(const QString &hash, const QString &parentHash = QString());

    /// Merge conflict helpers.
    void detectConflicts();
    void resolveConflict(const QString &path, int hunkIndex, const QString &resolution);
    void acceptAllOurs(const QString &path);
    void acceptAllTheirs(const QString &path);
    void markConflictResolved(const QString &path);

    /// Tear down. Releases the handle.
    void close();

signals:
    void statusChanged();
    void repoChanged();
    void branchChanged();
    void filesChanged();
    void diffChanged();
    void commitsChanged();
    void graphChanged();
    void graphMetadataChanged();
    void comparisonChanged();
    void stashesChanged();
    void branchesChanged();
    void blameChanged();
    void commitDetailChanged();
    void tagsChanged();
    void remotesChanged();
    void configChanged();
    void conflictFilesChanged();
    void rebaseChanged();
    void submodulesChanged();
    void busyChanged();
    void operationFinished(const QString &operation, bool success, const QString &message);

private slots:
    void onOpenResult(quint64 requestId, void *handle, const QString &repoPath, const QString &error);
    void onStatusResult(quint64 requestId, const QString &json);
    void onBranchResult(quint64 requestId, const QString &json);
    void onDiffResult(quint64 requestId, const QString &path, const QString &text);
    void onHistoryResult(quint64 requestId, const QString &json);
    void onGraphResult(quint64 requestId, const QString &json);
    void onGraphBranchesResult(quint64 requestId, const QString &json);
    void onGraphAuthorsResult(quint64 requestId, const QString &json);
    void onGraphRepoFilesResult(quint64 requestId, const QString &json);
    void onGraphGitUserResult(quint64 requestId, const QString &value);
    void onStashResult(quint64 requestId, const QString &json);
    void onBranchesResult(quint64 requestId, const QString &json);
    void onBlameResult(quint64 requestId, const QString &path, const QString &json);
    void onCommitDetailResult(quint64 requestId, const QString &json);
    void onTagsResult(quint64 requestId, const QString &json);
    void onRemotesResult(quint64 requestId, const QString &json);
    void onConfigResult(quint64 requestId, const QString &json);
    void onRebasePlanResult(quint64 requestId, bool inProgress, const QString &json);
    void onSubmodulesResult(quint64 requestId, const QString &json);
    void onOpResult(quint64 requestId, const QString &operation, bool success, const QString &message);

private:
    void setStatus(Status s);
    void setBusy(bool b);
    void parseStatusJson(const QString &json);
    void parseBranchJson(const QString &json);
    int conflictFileIndexForPath(const QString &path) const;
    bool writeResolvedConflictFile(const QVariantMap &file, const QString &defaultResolution, QString *errorMessage = nullptr) const;

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
    QVariantList m_graphRows;
    QStringList  m_graphBranches;
    QStringList  m_graphAuthors;
    QStringList  m_graphRepoFiles;
    QString      m_graphGitUserName;

    // Stash
    QVariantList m_stashes;

    // Branches
    QStringList m_branches;

    // Blame / Tags / Remotes / Config
    QVariantList m_blameLines;
    QString m_blameFilePath;
    QVariantList m_comparisonFiles;
    QString m_comparisonDiff;
    QString m_comparisonBaseHash;
    QString m_comparisonPath;
    QVariantMap m_commitDetail;
    QVariantList m_tags;
    QVariantList m_remotes;
    QVariantList m_configEntries;
    QVariantList m_conflictFiles;
    QVariantList m_rebaseTodoItems;
    bool m_rebaseInProgress = false;
    QVariantList m_submodules;

    quint64 m_nextRequestId = 0;
    quint64 m_nextGraphRequestId = 0;
    quint64 m_nextGraphMetadataRequestId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancelFlag;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
