// ─────────────────────────────────────────────────────────
// PierMySqlWorkspace — saved profiles + favorite SQL
// ─────────────────────────────────────────────────────────
//
// Small persistent workspace for the MySQL browser. Stores:
//
//   * connection profiles (host / port / user / db / credentialId)
//   * favorite queries (name / sql / database)
//
// Persistence uses QSettings so it rides the platform-native
// app config location and stays available across tabs and
// restarts. Secrets never live here — only the opaque
// credential id pointing at the OS keyring entry.

#pragma once

#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariantMap>
#include <qqml.h>

#include <vector>

class PierMySqlWorkspace : public QObject
{
    Q_OBJECT
    QML_NAMED_ELEMENT(PierMySqlWorkspace)

    Q_PROPERTY(QStringList profileNames READ profileNames NOTIFY profilesChanged FINAL)
    Q_PROPERTY(QStringList favoriteNames READ favoriteNames NOTIFY favoritesChanged FINAL)
    Q_PROPERTY(int profileCount READ profileCount NOTIFY profilesChanged FINAL)
    Q_PROPERTY(int favoriteCount READ favoriteCount NOTIFY favoritesChanged FINAL)

public:
    explicit PierMySqlWorkspace(QObject *parent = nullptr);

    QStringList profileNames() const;
    QStringList favoriteNames() const;
    int profileCount() const { return static_cast<int>(m_profiles.size()); }
    int favoriteCount() const { return static_cast<int>(m_favorites.size()); }

    Q_INVOKABLE void reload();
    Q_INVOKABLE QVariantMap profileAt(int index) const;
    Q_INVOKABLE QVariantMap favoriteAt(int index) const;
    Q_INVOKABLE int indexOfProfile(const QString &name) const;
    Q_INVOKABLE int indexOfFavorite(const QString &name) const;
    Q_INVOKABLE bool credentialReferencedElsewhere(const QString &credentialId, int excludingIndex) const;
    Q_INVOKABLE bool upsertProfile(const QString &name,
                                   const QString &host,
                                   int port,
                                   const QString &user,
                                   const QString &database,
                                   const QString &credentialId);
    Q_INVOKABLE bool removeProfile(int index);
    Q_INVOKABLE bool upsertFavorite(const QString &name,
                                    const QString &sql,
                                    const QString &database);
    Q_INVOKABLE bool removeFavorite(int index);

signals:
    void profilesChanged();
    void favoritesChanged();

private:
    struct Profile {
        QString name;
        QString host;
        int port = 3306;
        QString user;
        QString database;
        QString credentialId;
    };

    struct Favorite {
        QString name;
        QString sql;
        QString database;
    };

    bool persistProfiles();
    bool persistFavorites();
    int findProfileByName(const QString &name) const;
    int findFavoriteByName(const QString &name) const;

    std::vector<Profile> m_profiles;
    std::vector<Favorite> m_favorites;
};
