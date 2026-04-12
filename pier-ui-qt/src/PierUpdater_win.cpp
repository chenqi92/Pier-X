#include "PierUpdater.h"

// ─────────────────────────────────────────────────────────
// WinSparkle integration for Windows
// ─────────────────────────────────────────────────────────
//
// Only active when WinSparkle is found by CMake
// (PIER_HAS_WINSPARKLE is defined). Otherwise falls through
// to a no-op implementation.

#ifdef PIER_HAS_WINSPARKLE

#include <winsparkle.h>
#include <QCoreApplication>

namespace PierUpdater {

void initialize()
{
    win_sparkle_set_app_details(
        L"kkape",
        L"Pier-X",
        QCoreApplication::applicationVersion().toStdWString().c_str()
    );
    win_sparkle_set_appcast_url(appcastUrl());
    win_sparkle_init();
}

void checkForUpdates()
{
    win_sparkle_check_update_with_ui();
}

void setAutomaticChecks(bool enabled)
{
    win_sparkle_set_automatic_check_for_updates(enabled ? 1 : 0);
}

bool automaticChecksEnabled()
{
    return win_sparkle_get_automatic_check_for_updates() != 0;
}

bool available()
{
    return true;
}

void cleanup()
{
    win_sparkle_cleanup();
}

} // namespace PierUpdater

#else // !PIER_HAS_WINSPARKLE

namespace PierUpdater {

void initialize() {}
void checkForUpdates() {}
void setAutomaticChecks(bool) {}
bool automaticChecksEnabled() { return false; }
bool available() { return false; }
void cleanup() {}

} // namespace PierUpdater

#endif // PIER_HAS_WINSPARKLE
