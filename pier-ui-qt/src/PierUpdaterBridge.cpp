#include "PierUpdaterBridge.h"
#include "PierUpdater.h"

PierUpdaterBridge::PierUpdaterBridge(QObject *parent)
    : QObject(parent)
{
}

bool PierUpdaterBridge::available() const
{
    return PierUpdater::available();
}

bool PierUpdaterBridge::autoCheck() const
{
    return PierUpdater::automaticChecksEnabled();
}

void PierUpdaterBridge::setAutoCheck(bool enabled)
{
    if (enabled == autoCheck())
        return;
    PierUpdater::setAutomaticChecks(enabled);
    emit autoCheckChanged();
}

void PierUpdaterBridge::checkForUpdates()
{
    PierUpdater::checkForUpdates();
}
