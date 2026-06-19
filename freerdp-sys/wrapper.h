/* bindgen entry point: the subset of the libfreerdp3 / libwinpr3 public API
 * Pier-X's RDP backend drives. Kept deliberately small — the allowlist in
 * build.rs narrows generation to these symbols so the bindings stay tractable
 * and stable across FreeRDP point releases. */

#include <winpr/winpr.h>
#include <winpr/synch.h>
#include <winpr/collections.h>

#include <freerdp/freerdp.h>
#include <freerdp/client.h>
#include <freerdp/client/cmdline.h>
#include <freerdp/settings.h>
#include <freerdp/event.h>
#include <freerdp/input.h>
#include <freerdp/scancode.h>
#include <freerdp/codec/color.h>
#include <freerdp/gdi/gdi.h>
#include <freerdp/gdi/gfx.h>
#include <freerdp/channels/channels.h>
#include <freerdp/channels/rdpgfx.h>

/* A few values Pier-X needs come from function-like or multi-line macros that
 * bindgen can't reliably fold to constants. Re-expose them as a plain enum so
 * the C compiler evaluates them and bindgen emits clean integer consts. */
enum PierxConst {
	/* Memory byte order R,G,B,X — matches the WebView canvas RGBA tile, so the
	 * GDI primary buffer needs no channel swap, only alpha forced to 0xFF. */
	PIERX_PIXEL_FORMAT_RGBX32 = (int)PIXEL_FORMAT_RGBX32,
	PIERX_PIXEL_FORMAT_RGBA32 = (int)PIXEL_FORMAT_RGBA32,
	PIERX_KBD_FLAGS_RELEASE = (int)KBD_FLAGS_RELEASE,
	PIERX_KBDEXT = (int)KBDEXT,
};
