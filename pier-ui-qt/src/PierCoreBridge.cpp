#include "PierCoreBridge.h"

#include "pier_core.h"

#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QSet>

PierCoreBridge::PierCoreBridge(QObject *parent)
    : QObject(parent)
{
}

QString PierCoreBridge::version() const
{
    // pier_core_version() returns a static UTF-8 C string owned by the Rust
    // crate; we copy into a QString so the QML side can treat it as a
    // normal value type.
    return QString::fromUtf8(pier_core_version());
}

QString PierCoreBridge::buildInfo() const
{
    return QString::fromUtf8(pier_core_build_info());
}

QString PierCoreBridge::qtVersion() const
{
    // qVersion() is the runtime Qt version the app was linked against,
    // which is what the status bar should show (vs QT_VERSION_STR which
    // is baked in at compile time of this translation unit).
    return QString::fromUtf8(qVersion());
}

bool PierCoreBridge::hasFeature(const QString &name) const
{
    const QByteArray utf8 = name.toUtf8();
    return pier_core_has_feature(utf8.constData()) != 0;
}

QStringList PierCoreBridge::localHistory() const
{
    QStringList history;
    const QString home = QDir::homePath();
    
    // Read bash history
    QFile bashFile(home + "/.bash_history");
    if (bashFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream in(&bashFile);
        while (!in.atEnd()) {
            QString line = in.readLine().trimmed();
            if (!line.isEmpty()) {
                history.append(line);
            }
        }
    }
    
    // Read zsh history (format: : 1612345678:0;command)
    QFile zshFile(home + "/.zsh_history");
    if (zshFile.open(QIODevice::ReadOnly | QIODevice::Text)) {
        QTextStream in(&zshFile);
        while (!in.atEnd()) {
            QString line = in.readLine().trimmed();
            if (line.startsWith(':')) {
                int semi = line.indexOf(';');
                if (semi > 0) {
                    line = line.mid(semi + 1).trimmed();
                }
            }
            if (!line.isEmpty()) {
                history.append(line);
            }
        }
    }
    
    // Deduplicate and reverse (newest first)
    // To keep simple, we'll reverse the list first, then deduplicate while maintaining order
    QStringList uniqueRes;
    QSet<QString> seen;
    for (int i = history.size() - 1; i >= 0; --i) {
        const QString &cmd = history[i];
        if (!seen.contains(cmd)) {
            seen.insert(cmd);
            uniqueRes.append(cmd);
        }
        if (uniqueRes.size() >= 500) break; // keep max 500
    }
    
    return uniqueRes;
}
