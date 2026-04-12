// ─────────────────────────────────────────────────────────
// PierUpdater — cross-platform auto-update API
// ─────────────────────────────────────────────────────────
//
// Thin abstraction over Sparkle (macOS) and WinSparkle (Windows).
// Both frameworks provide their own native update dialogs, so
// the Qt side only needs to call initialize/check/cleanup.
//
// Platform files:
//   PierUpdater_mac.mm  — Sparkle 2.x via SPUStandardUpdaterController
//   PierUpdater_win.cpp — WinSparkle via win_sparkle_* C API
//
// On unsupported platforms all functions are no-ops.

#pragma once

#include <QString>

namespace PierUpdater {

/// Call once after the QML engine has loaded.
/// macOS: creates SPUStandardUpdaterController.
/// Windows: calls win_sparkle_init().
void initialize();

/// Trigger a user-initiated update check with UI feedback.
void checkForUpdates();

/// Enable or disable automatic background checks.
void setAutomaticChecks(bool enabled);

/// Returns true if background checks are currently enabled.
bool automaticChecksEnabled();

/// Returns true on platforms where updates are supported
/// (macOS with Sparkle, Windows with WinSparkle).
bool available();

/// Clean up resources (called on app exit).
/// Windows: calls win_sparkle_cleanup().
void cleanup();

/// The appcast feed URL. Compiled into the platform implementations.
inline const char *appcastUrl()
{
    return "https://releases.kkape.com/pier-x/appcast.xml";
}

} // namespace PierUpdater
