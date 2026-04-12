#include "PierUpdater.h"

// ─────────────────────────────────────────────────────────
// Stub implementation for platforms without an update framework
// (Linux, FreeBSD, etc.)
// ─────────────────────────────────────────────────────────
//
// This file is only compiled on non-Apple, non-Windows via CMake,
// so no preprocessor guard is needed.

namespace PierUpdater {

void initialize() {}
void checkForUpdates() {}
void setAutomaticChecks(bool) {}
bool automaticChecksEnabled() { return false; }
bool available() { return false; }
void cleanup() {}

} // namespace PierUpdater
