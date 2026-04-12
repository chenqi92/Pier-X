#include "PierUpdater.h"

// ─────────────────────────────────────────────────────────
// Sparkle 2.x integration for macOS
// ─────────────────────────────────────────────────────────
//
// Only active when Sparkle.framework is found by CMake
// (PIER_HAS_SPARKLE is defined). Otherwise falls through to
// a no-op implementation identical to the stub.

#ifdef PIER_HAS_SPARKLE

#import <Cocoa/Cocoa.h>
#import <Sparkle/Sparkle.h>

static SPUStandardUpdaterController *s_updaterController = nil;

namespace PierUpdater {

void initialize()
{
    @autoreleasepool {
        // SPUStandardUpdaterController is the high-level Sparkle 2.x
        // entry point.  Setting startingUpdater:YES tells it to start
        // its background check schedule immediately based on the
        // user's persisted preferences.
        s_updaterController = [[SPUStandardUpdaterController alloc]
            initWithStartingUpdater:YES
            updaterDelegate:nil
            userDriverDelegate:nil];
    }
}

void checkForUpdates()
{
    @autoreleasepool {
        if (s_updaterController)
            [s_updaterController checkForUpdates:nil];
    }
}

void setAutomaticChecks(bool enabled)
{
    @autoreleasepool {
        if (s_updaterController)
            s_updaterController.updater.automaticallyChecksForUpdates = enabled;
    }
}

bool automaticChecksEnabled()
{
    @autoreleasepool {
        if (s_updaterController)
            return s_updaterController.updater.automaticallyChecksForUpdates;
    }
    return false;
}

bool available()
{
    return true;
}

void cleanup()
{
    s_updaterController = nil;
}

} // namespace PierUpdater

#else // !PIER_HAS_SPARKLE — Sparkle not found

namespace PierUpdater {

void initialize() {}
void checkForUpdates() {}
void setAutomaticChecks(bool) {}
bool automaticChecksEnabled() { return false; }
bool available() { return false; }
void cleanup() {}

} // namespace PierUpdater

#endif // PIER_HAS_SPARKLE
