#pragma once

#include <QObject>
#include <QPointer>
#include <QString>
#include <QVariantList>
#include <qqml.h>

#include <atomic>
#include <memory>
#include <thread>
#include <vector>

/// File and content search — wraps pier_search_* FFI.
class PierSearch : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierSearch)

    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged FINAL)
    Q_PROPERTY(QVariantList results READ results NOTIFY resultsChanged FINAL)
    Q_PROPERTY(QString searchMode READ searchMode NOTIFY searchModeChanged FINAL)

public:
    explicit PierSearch(QObject *parent = nullptr);
    ~PierSearch() override;

    bool busy() const { return m_busy; }
    QVariantList results() const { return m_results; }
    QString searchMode() const { return m_searchMode; }

public slots:
    /// Search file names.
    void searchFiles(const QString &root, const QString &pattern, int maxResults = 200);
    /// Search file contents.
    void searchContent(const QString &root, const QString &pattern, int maxResults = 200);

signals:
    void busyChanged();
    void resultsChanged();
    void searchModeChanged();

private slots:
    void onResult(quint64 id, const QString &json);

private:
    bool m_busy = false;
    QVariantList m_results;
    QString m_searchMode = QStringLiteral("files");
    quint64 m_nextId = 0;
    std::shared_ptr<std::atomic<bool>> m_cancel;
    std::vector<std::unique_ptr<std::thread>> m_workers;
};
