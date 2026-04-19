//! System accent color discovery.
//!
//! Returns the user's system-wide accent color on platforms that expose
//! one. The UI shell uses this to override the default `#3574F0` in the
//! theme palette so Pier-X blends in with the rest of the desktop —
//! matching what SwiftUI gets for free via `Color.accentColor`.
//!
//! Platform status:
//! - **macOS**: reads `NSColor.controlAccentColor`, converted to sRGB.
//! - **Windows**: not yet implemented — a future impl can read
//!   `HKCU\SOFTWARE\Microsoft\Windows\DWM\AccentColor` (DWORD, ABGR).
//! - **Linux / other**: returns `None` (no standard API).

/// An sRGB triplet, 0–255 per channel. Alpha is always implicit 255 at
/// this layer — callers blend against theme tokens as needed.
pub type Rgb = (u8, u8, u8);

/// Read the system accent color, if the platform exposes one.
///
/// This is cheap enough to call at startup and on appearance-change
/// events. It never panics; any error (framework absent, value out of
/// range) maps to `None` and callers fall back to the brand default.
pub fn system_accent() -> Option<Rgb> {
    platform_impl::system_accent()
}

#[cfg(target_os = "macos")]
mod platform_impl {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, ClassType};
    use objc2_app_kit::{NSColor, NSColorSpace};

    use super::Rgb;

    pub fn system_accent() -> Option<Rgb> {
        // Safety: all calls below stay on the main thread (the caller
        // runs inside gpui's `Application::run` context which is main-
        // thread-pinned), and we only retain autoreleased AppKit
        // objects until we're done reading them.
        unsafe {
            // `+[NSColor controlAccentColor]` is the user's chosen
            // system accent (macOS 10.14+). Always available on our
            // minimum supported version.
            let accent_cls = NSColor::class();
            let accent_obj: *mut AnyObject =
                msg_send![accent_cls, controlAccentColor];
            if accent_obj.is_null() {
                return None;
            }
            let accent = Retained::<NSColor>::retain(accent_obj.cast())?;

            // Force into sRGB so `-redComponent` / etc. are defined on
            // the resulting color (they panic-equivalent on, e.g.,
            // catalog-backed colors in device gray space).
            let srgb_cls = NSColorSpace::class();
            let srgb: *mut AnyObject = msg_send![srgb_cls, sRGBColorSpace];
            let srgb_space = Retained::<NSColorSpace>::retain(srgb.cast())?;

            let converted_obj: *mut AnyObject = msg_send![
                &*accent,
                colorUsingColorSpace: &*srgb_space,
            ];
            if converted_obj.is_null() {
                return None;
            }
            let color = Retained::<NSColor>::retain(converted_obj.cast())?;

            let r: f64 = msg_send![&*color, redComponent];
            let g: f64 = msg_send![&*color, greenComponent];
            let b: f64 = msg_send![&*color, blueComponent];

            Some((to_u8(r), to_u8(g), to_u8(b)))
        }
    }

    fn to_u8(component: f64) -> u8 {
        let clamped = component.clamp(0.0, 1.0);
        (clamped * 255.0).round() as u8
    }
}

#[cfg(not(target_os = "macos"))]
mod platform_impl {
    use super::Rgb;

    pub fn system_accent() -> Option<Rgb> {
        // TODO(accent): Windows should read
        //   HKCU\SOFTWARE\Microsoft\Windows\DWM\AccentColor (DWORD, ABGR).
        // Until that lands, the default `#3574F0` brand accent wins on
        // every non-macOS platform. The plumbing above is ready for
        // the impl — just extend this module.
        None
    }
}
