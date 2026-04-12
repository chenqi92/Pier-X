#include "PierMySqlWorkspace.h"

#include <QDebug>
#include <QSettings>

namespace {

QString normalizedName(const QString &name)
{
    return name.trimmed();
}

} // namespace

PierMySqlWorkspace::PierMySqlWorkspace(QObject *parent)
    : QObject(parent)
{
    reload();
}

QStringList PierMySqlWorkspace::profileNames() const
{
    QStringList names;
    names.reserve(static_cast<qsizetype>(m_profiles.size()));
    for (const Profile &profile : m_profiles) {
        names.append(profile.name);
    }
    return names;
}

QStringList PierMySqlWorkspace::favoriteNames() const
{
    QStringList names;
    names.reserve(static_cast<qsizetype>(m_favorites.size()));
    for (const Favorite &favorite : m_favorites) {
        names.append(favorite.name);
    }
    return names;
}

void PierMySqlWorkspace::reload()
{
    QSettings settings;

    std::vector<Profile> profiles;
    const int profileCount = settings.beginReadArray(QStringLiteral("mysqlProfiles"));
    profiles.reserve(static_cast<size_t>(profileCount));
    for (int i = 0; i < profileCount; ++i) {
        settings.setArrayIndex(i);
        const QString name = normalizedName(settings.value(QStringLiteral("name")).toString());
        const QString host = settings.value(QStringLiteral("host")).toString();
        const QString user = settings.value(QStringLiteral("user")).toString();
        if (name.isEmpty() || host.isEmpty() || user.isEmpty()) {
            continue;
        }
        Profile profile;
        profile.name = name;
        profile.host = host;
        profile.port = settings.value(QStringLiteral("port"), 3306).toInt();
        profile.user = user;
        profile.database = settings.value(QStringLiteral("database")).toString();
        profile.credentialId = settings.value(QStringLiteral("credentialId")).toString();
        profiles.push_back(std::move(profile));
    }
    settings.endArray();

    std::vector<Favorite> favorites;
    const int favoriteCount = settings.beginReadArray(QStringLiteral("mysqlFavorites"));
    favorites.reserve(static_cast<size_t>(favoriteCount));
    for (int i = 0; i < favoriteCount; ++i) {
        settings.setArrayIndex(i);
        const QString name = normalizedName(settings.value(QStringLiteral("name")).toString());
        const QString sql = settings.value(QStringLiteral("sql")).toString();
        if (name.isEmpty() || sql.trimmed().isEmpty()) {
            continue;
        }
        Favorite favorite;
        favorite.name = name;
        favorite.sql = sql;
        favorite.database = settings.value(QStringLiteral("database")).toString();
        favorites.push_back(std::move(favorite));
    }
    settings.endArray();

    m_profiles = std::move(profiles);
    m_favorites = std::move(favorites);
    emit profilesChanged();
    emit favoritesChanged();
}

QVariantMap PierMySqlWorkspace::profileAt(int index) const
{
    if (index < 0 || index >= static_cast<int>(m_profiles.size())) {
        return {};
    }
    const Profile &profile = m_profiles[static_cast<size_t>(index)];
    QVariantMap map;
    map.insert(QStringLiteral("name"), profile.name);
    map.insert(QStringLiteral("host"), profile.host);
    map.insert(QStringLiteral("port"), profile.port);
    map.insert(QStringLiteral("user"), profile.user);
    map.insert(QStringLiteral("database"), profile.database);
    map.insert(QStringLiteral("credentialId"), profile.credentialId);
    map.insert(QStringLiteral("hasCredential"), !profile.credentialId.isEmpty());
    return map;
}

QVariantMap PierMySqlWorkspace::favoriteAt(int index) const
{
    if (index < 0 || index >= static_cast<int>(m_favorites.size())) {
        return {};
    }
    const Favorite &favorite = m_favorites[static_cast<size_t>(index)];
    QVariantMap map;
    map.insert(QStringLiteral("name"), favorite.name);
    map.insert(QStringLiteral("sql"), favorite.sql);
    map.insert(QStringLiteral("database"), favorite.database);
    return map;
}

int PierMySqlWorkspace::indexOfProfile(const QString &name) const
{
    return findProfileByName(normalizedName(name));
}

int PierMySqlWorkspace::indexOfFavorite(const QString &name) const
{
    return findFavoriteByName(normalizedName(name));
}

bool PierMySqlWorkspace::credentialReferencedElsewhere(const QString &credentialId, int excludingIndex) const
{
    if (credentialId.trimmed().isEmpty()) {
        return false;
    }
    for (size_t i = 0; i < m_profiles.size(); ++i) {
        if (static_cast<int>(i) == excludingIndex) {
            continue;
        }
        if (m_profiles[i].credentialId == credentialId) {
            return true;
        }
    }
    return false;
}

bool PierMySqlWorkspace::upsertProfile(
    const QString &name,
    const QString &host,
    int port,
    const QString &user,
    const QString &database,
    const QString &credentialId)
{
    const QString normalized = normalizedName(name);
    if (normalized.isEmpty() || host.isEmpty() || user.isEmpty()) {
        qWarning() << "PierMySqlWorkspace::upsertProfile rejected empty field";
        return false;
    }

    Profile profile;
    profile.name = normalized;
    profile.host = host;
    profile.port = port > 0 ? port : 3306;
    profile.user = user;
    profile.database = database.trimmed();
    profile.credentialId = credentialId.trimmed();

    const auto previous = m_profiles;
    const int existing = findProfileByName(normalized);
    if (existing >= 0) {
        m_profiles[static_cast<size_t>(existing)] = std::move(profile);
    } else {
        m_profiles.push_back(std::move(profile));
    }

    if (!persistProfiles()) {
        m_profiles = previous;
        return false;
    }
    emit profilesChanged();
    return true;
}

bool PierMySqlWorkspace::removeProfile(int index)
{
    if (index < 0 || index >= static_cast<int>(m_profiles.size())) {
        return false;
    }
    const auto previous = m_profiles;
    m_profiles.erase(m_profiles.begin() + index);
    if (!persistProfiles()) {
        m_profiles = previous;
        return false;
    }
    emit profilesChanged();
    return true;
}

bool PierMySqlWorkspace::upsertFavorite(
    const QString &name,
    const QString &sql,
    const QString &database)
{
    const QString normalized = normalizedName(name);
    if (normalized.isEmpty() || sql.trimmed().isEmpty()) {
        qWarning() << "PierMySqlWorkspace::upsertFavorite rejected empty field";
        return false;
    }

    Favorite favorite;
    favorite.name = normalized;
    favorite.sql = sql;
    favorite.database = database.trimmed();

    const auto previous = m_favorites;
    const int existing = findFavoriteByName(normalized);
    if (existing >= 0) {
        m_favorites[static_cast<size_t>(existing)] = std::move(favorite);
    } else {
        m_favorites.push_back(std::move(favorite));
    }

    if (!persistFavorites()) {
        m_favorites = previous;
        return false;
    }
    emit favoritesChanged();
    return true;
}

bool PierMySqlWorkspace::removeFavorite(int index)
{
    if (index < 0 || index >= static_cast<int>(m_favorites.size())) {
        return false;
    }
    const auto previous = m_favorites;
    m_favorites.erase(m_favorites.begin() + index);
    if (!persistFavorites()) {
        m_favorites = previous;
        return false;
    }
    emit favoritesChanged();
    return true;
}

bool PierMySqlWorkspace::persistProfiles()
{
    QSettings settings;
    settings.remove(QStringLiteral("mysqlProfiles"));
    settings.beginWriteArray(QStringLiteral("mysqlProfiles"));
    for (int i = 0; i < static_cast<int>(m_profiles.size()); ++i) {
        settings.setArrayIndex(i);
        const Profile &profile = m_profiles[static_cast<size_t>(i)];
        settings.setValue(QStringLiteral("name"), profile.name);
        settings.setValue(QStringLiteral("host"), profile.host);
        settings.setValue(QStringLiteral("port"), profile.port);
        settings.setValue(QStringLiteral("user"), profile.user);
        settings.setValue(QStringLiteral("database"), profile.database);
        settings.setValue(QStringLiteral("credentialId"), profile.credentialId);
    }
    settings.endArray();
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning() << "PierMySqlWorkspace failed to persist profiles";
        return false;
    }
    return true;
}

bool PierMySqlWorkspace::persistFavorites()
{
    QSettings settings;
    settings.remove(QStringLiteral("mysqlFavorites"));
    settings.beginWriteArray(QStringLiteral("mysqlFavorites"));
    for (int i = 0; i < static_cast<int>(m_favorites.size()); ++i) {
        settings.setArrayIndex(i);
        const Favorite &favorite = m_favorites[static_cast<size_t>(i)];
        settings.setValue(QStringLiteral("name"), favorite.name);
        settings.setValue(QStringLiteral("sql"), favorite.sql);
        settings.setValue(QStringLiteral("database"), favorite.database);
    }
    settings.endArray();
    settings.sync();
    if (settings.status() != QSettings::NoError) {
        qWarning() << "PierMySqlWorkspace failed to persist favorites";
        return false;
    }
    return true;
}

int PierMySqlWorkspace::findProfileByName(const QString &name) const
{
    for (size_t i = 0; i < m_profiles.size(); ++i) {
        if (m_profiles[i].name.compare(name, Qt::CaseSensitive) == 0) {
            return static_cast<int>(i);
        }
    }
    return -1;
}

int PierMySqlWorkspace::findFavoriteByName(const QString &name) const
{
    for (size_t i = 0; i < m_favorites.size(); ++i) {
        if (m_favorites[i].name.compare(name, Qt::CaseSensitive) == 0) {
            return static_cast<int>(i);
        }
    }
    return -1;
}
