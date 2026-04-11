#include "PierCredentials.h"

#include "pier_credentials.h"

#include <QByteArray>
#include <QDebug>
#include <QUuid>

PierCredentials::PierCredentials(QObject *parent)
    : QObject(parent)
{
}

bool PierCredentials::setEntry(const QString &id, const QString &value)
{
    if (id.isEmpty()) {
        qWarning() << "PierCredentials::setEntry rejected empty id";
        return false;
    }
    const QByteArray idUtf8 = id.toUtf8();
    const QByteArray valUtf8 = value.toUtf8();
    const int32_t rc = pier_credential_set(idUtf8.constData(), valUtf8.constData());
    if (rc != 0) {
        qWarning() << "pier_credential_set failed rc=" << rc << "id=" << id;
        return false;
    }
    return true;
}

bool PierCredentials::deleteEntry(const QString &id)
{
    if (id.isEmpty()) {
        return false;
    }
    const QByteArray idUtf8 = id.toUtf8();
    const int32_t rc = pier_credential_delete(idUtf8.constData());
    if (rc != 0) {
        qWarning() << "pier_credential_delete failed rc=" << rc << "id=" << id;
        return false;
    }
    return true;
}

QString PierCredentials::freshId() const
{
    // Use Qt's UUID generator rather than rolling our own RNG;
    // it's already seeded properly per-process. The "pier-x."
    // prefix scopes the keychain entries so they're easy to
    // identify in OS-level keychain managers.
    const QString uuid = QUuid::createUuid()
        .toString(QUuid::WithoutBraces)
        .remove(QChar('-'));
    return QStringLiteral("pier-x.") + uuid;
}
