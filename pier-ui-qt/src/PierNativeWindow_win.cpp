#include "PierNativeWindow.h"

// Windows stub — keep the default system title bar.
// A custom frameless implementation (DWM + Snap Layouts) can be
// added here later if needed without touching the header or
// any other platform's file.

namespace PierNativeWindow {

void applyFramelessChrome(QWindow * /* window */)
{
    // no-op on Windows
}

int titleBarHeight()
{
    return 0;
}

} // namespace PierNativeWindow
