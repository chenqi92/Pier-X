#include "PierLocalSystem.h"

#include <QClipboard>
#include <QDesktopServices>
#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QProcess>
#include <QUrl>
#include <QDebug>

PierLocalSystem::PierLocalSystem(QObject *parent)
    : QObject(parent)
{
}

bool PierLocalSystem::copyText(const QString &text) const
{
    if (auto *clipboard = QGuiApplication::clipboard()) {
        clipboard->setText(text);
        return true;
    }
    qWarning() << "PierLocalSystem::copyText: clipboard unavailable";
    return false;
}

QString PierLocalSystem::readText() const
{
    if (auto *clipboard = QGuiApplication::clipboard()) {
        return clipboard->text();
    }
    qWarning() << "PierLocalSystem::readText: clipboard unavailable";
    return {};
}

bool PierLocalSystem::openPath(const QString &path) const
{
    if (path.isEmpty()) {
        return false;
    }

    const QFileInfo info(path);
    const QString localPath = info.exists()
        ? info.absoluteFilePath()
        : QDir::cleanPath(path);
    return QDesktopServices::openUrl(QUrl::fromLocalFile(localPath));
}

bool PierLocalSystem::revealPath(const QString &path) const
{
    if (path.isEmpty()) {
        return false;
    }

    const QFileInfo info(path);
    const QString localPath = info.exists()
        ? info.absoluteFilePath()
        : QDir::cleanPath(path);

#if defined(Q_OS_MACOS)
    if (info.exists() && info.isFile()) {
        return QProcess::startDetached(QStringLiteral("/usr/bin/open"),
                                       {QStringLiteral("-R"), localPath});
    }
    const QString dirPath = info.isDir() ? localPath : QFileInfo(localPath).absolutePath();
    return QProcess::startDetached(QStringLiteral("/usr/bin/open"), {dirPath});
#elif defined(Q_OS_WIN)
    QStringList args;
    if (info.exists() && info.isFile()) {
        args << QStringLiteral("/select,") << QDir::toNativeSeparators(localPath);
    } else {
        const QString dirPath = info.isDir() ? localPath : QFileInfo(localPath).absolutePath();
        args << QDir::toNativeSeparators(dirPath);
    }
    return QProcess::startDetached(QStringLiteral("explorer.exe"), args);
#else
    const QString dirPath = info.isDir() ? localPath : QFileInfo(localPath).absolutePath();
    return QDesktopServices::openUrl(QUrl::fromLocalFile(dirPath));
#endif
}

bool PierLocalSystem::initGitRepository(const QString &path) const
{
    if (path.isEmpty()) {
        return false;
    }

    const QFileInfo info(path);
    const QString localPath = info.exists()
        ? info.absoluteFilePath()
        : QDir::cleanPath(path);
    const QString targetDir = info.isDir() ? localPath : QFileInfo(localPath).absolutePath();
    if (targetDir.isEmpty()) {
        return false;
    }

    QProcess process;
    process.setProgram(QStringLiteral("git"));
    process.setArguments({QStringLiteral("init"), targetDir});
    process.start();
    if (!process.waitForStarted(3000)) {
        qWarning() << "PierLocalSystem::initGitRepository: failed to start git";
        return false;
    }
    if (!process.waitForFinished(10000)) {
        qWarning() << "PierLocalSystem::initGitRepository: git init timed out";
        process.kill();
        return false;
    }

    return process.exitStatus() == QProcess::NormalExit && process.exitCode() == 0;
}
