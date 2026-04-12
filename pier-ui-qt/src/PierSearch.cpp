#include "PierSearch.h"
#include "pier_search.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>

PierSearch::PierSearch(QObject *parent) : QObject(parent) {}

PierSearch::~PierSearch()
{
    if (m_cancel) m_cancel->store(true);
    for (auto &t : m_workers) if (t && t->joinable()) t->detach();
}

void PierSearch::searchFiles(const QString &root, const QString &pattern, int maxResults)
{
    if (root.isEmpty() || pattern.isEmpty()) {
        m_results.clear(); emit resultsChanged(); return;
    }
    const quint64 id = ++m_nextId;
    m_cancel = std::make_shared<std::atomic<bool>>(false);
    m_busy = true; emit busyChanged();
    m_searchMode = QStringLiteral("files"); emit searchModeChanged();

    std::string r = root.toStdString();
    std::string p = pattern.toStdString();
    uint32_t max = static_cast<uint32_t>(maxResults);
    QPointer<PierSearch> self(this);
    auto cancel = m_cancel;

    auto w = std::make_unique<std::thread>([self, cancel, id, r = std::move(r), p = std::move(p), max]() {
        char *json = pier_search_files(r.c_str(), p.c_str(), max);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_search_free_string(json);
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierSearch::searchContent(const QString &root, const QString &pattern, int maxResults)
{
    if (root.isEmpty() || pattern.isEmpty()) {
        m_results.clear(); emit resultsChanged(); return;
    }
    const quint64 id = ++m_nextId;
    m_cancel = std::make_shared<std::atomic<bool>>(false);
    m_busy = true; emit busyChanged();
    m_searchMode = QStringLiteral("content"); emit searchModeChanged();

    std::string r = root.toStdString();
    std::string p = pattern.toStdString();
    uint32_t max = static_cast<uint32_t>(maxResults);
    QPointer<PierSearch> self(this);
    auto cancel = m_cancel;

    auto w = std::make_unique<std::thread>([self, cancel, id, r = std::move(r), p = std::move(p), max]() {
        char *json = pier_search_content(r.c_str(), p.c_str(), max);
        QString result = json ? QString::fromUtf8(json) : QStringLiteral("[]");
        if (json) pier_search_free_string(json);
        if (!self || (cancel && cancel->load())) return;
        QMetaObject::invokeMethod(self.data(), "onResult", Qt::QueuedConnection,
            Q_ARG(quint64, id), Q_ARG(QString, result));
    });
    m_workers.push_back(std::move(w));
}

void PierSearch::onResult(quint64 id, const QString &json)
{
    if (id != m_nextId) return;
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    QVariantList list;
    if (doc.isArray()) {
        for (const auto &v : doc.array()) {
            list.append(v.toObject().toVariantMap());
        }
    }
    m_results = list;
    emit resultsChanged();
    m_busy = false; emit busyChanged();
}
