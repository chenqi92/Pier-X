//! Input events forwarded from the viewer (frontend) to the remote host.
//!
//! Coordinates are always in **remote framebuffer pixels** — the frontend
//! scales canvas coordinates to the remote resolution before sending, so the
//! backends never need to know the canvas size.

/// Mouse buttons, in the order the frontend reports `MouseEvent.button`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// Primary (left) button.
    Left,
    /// Auxiliary (middle / wheel) button.
    Middle,
    /// Secondary (right) button.
    Right,
}

/// A single input action. The keyboard variant carries **both** an X11
/// keysym (consumed by VNC) and a PC/AT set-1 scancode (consumed by RDP);
/// each backend reads only the field its protocol speaks.
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// Pointer moved to `(x, y)` with no button-state change.
    PointerMove {
        /// Remote X in pixels.
        x: u16,
        /// Remote Y in pixels.
        y: u16,
    },
    /// A button transitioned at `(x, y)`.
    PointerButton {
        /// Remote X in pixels.
        x: u16,
        /// Remote Y in pixels.
        y: u16,
        /// Which button changed.
        button: MouseButton,
        /// `true` = pressed, `false` = released.
        pressed: bool,
    },
    /// Wheel scroll at `(x, y)`. `dy > 0` scrolls down, `dy < 0` up; `dx`
    /// is horizontal (rarely used). Units are "notches" (≈120 per tick on
    /// the frontend, normalised to ±1 here).
    PointerScroll {
        /// Remote X in pixels.
        x: u16,
        /// Remote Y in pixels.
        y: u16,
        /// Horizontal notches.
        dx: i16,
        /// Vertical notches (positive = down).
        dy: i16,
    },
    /// A key transitioned. `keysym` is the X11 keysym (VNC); `scancode`
    /// is the PC AT set-1 make code without the 0x80 release bit, and
    /// `extended` marks the 0xE0-prefixed keys (RDP).
    Key {
        /// X11 keysym (for VNC `KeyEvent`).
        keysym: u32,
        /// PC AT set-1 scancode (for RDP `ironrdp_input::Scancode`).
        scancode: u16,
        /// `true` when the scancode needs the 0xE0 extended prefix.
        extended: bool,
        /// `true` = key down, `false` = key up.
        pressed: bool,
    },
    /// Inject a Unicode character directly (IME composition / paste).
    KeyUnicode {
        /// The character to type.
        ch: char,
        /// `true` = press, `false` = release.
        pressed: bool,
    },
}
