#![allow(dead_code)]

//! Square icon-only button. Use for toolbar rails, inline list actions,
//! and anywhere a labeled `Button` would feel bulky. When you want a
//! labeled button, reach for [`super::Button`] instead.
//!
//! Two variants:
//! - `Ghost` — no fill, hover reveals a subtle tint. Default on toolbar
//!   rails where the icon grid should recede visually.
//! - `Filled` — always-visible surface fill. Use when the button needs
//!   to feel "present" (e.g. a detached FAB-style control).

use gpui::{
    div, prelude::*, App, ClickEvent, ElementId, IntoElement, Pixels, Rgba, SharedString, Window,
};
use gpui_component::{tooltip::Tooltip, Icon as UiIcon, IconName};

use crate::theme::{
    heights::{BUTTON_MD_H, BUTTON_SM_H, BUTTON_XS_H, GLYPH_2XS, ICON_MD, ICON_SM},
    radius::RADIUS_SM,
    theme,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconButtonVariant {
    /// True ghost — transparent; hover reveals a tint. Default on
    /// toolbar rails / row actions that should feel quiet.
    Ghost,
    /// Surface-tinted fill with neutral fg. The "show me a button" shape.
    Filled,
    /// Accent blue fill — primary action in a row (e.g. Start container).
    Primary,
    /// Error red fill — destructive action (Stop running container,
    /// Delete image/volume). Fg stays white for readability; hover
    /// flashes the solid red so the cue is unmissable.
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconButtonSize {
    /// 18px tappable, 11px glyph — for inline list-row actions (e.g.
    /// SFTP row hover buttons). Default variant is Ghost; row actions
    /// should feel near-invisible until hover.
    Xs,
    Sm,
    Md,
}

impl IconButtonSize {
    fn square(self) -> Pixels {
        match self {
            Self::Xs => BUTTON_XS_H,
            Self::Sm => BUTTON_SM_H,
            Self::Md => BUTTON_MD_H,
        }
    }

    fn icon(self) -> Pixels {
        match self {
            Self::Xs => GLYPH_2XS,
            Self::Sm => ICON_SM,
            Self::Md => ICON_MD,
        }
    }
}

#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    variant: IconButtonVariant,
    size: IconButtonSize,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            id: id.into(),
            icon,
            variant: IconButtonVariant::Ghost,
            // Default Sm (22px) — matches SwiftUI's standard toolbar
            // icon-button size. Rare `.size(Md)` for emphasis only.
            size: IconButtonSize::Sm,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    /// Attach a hover tooltip — mirrors SwiftUI's `.help(…)` on
    /// toolbar icons. Shown after the OS-standard hover delay via
    /// gpui_component's shared tooltip infrastructure.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn variant(mut self, variant: IconButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: IconButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Render the button in a disabled state — greyed icon, no hover,
    /// no click handler fires. Prefer over "conditional on_click" so
    /// the visual cue matches the behaviour.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let (bg, hover_bg, fg): (Rgba, Rgba, Rgba) = match (self.variant, self.disabled) {
            (_, true) => (Rgba::default(), Rgba::default(), t.color.text_disabled),
            (IconButtonVariant::Ghost, false) => {
                (Rgba::default(), t.color.bg_hover, t.color.text_secondary)
            }
            (IconButtonVariant::Filled, false) => {
                (t.color.bg_surface, t.color.bg_hover, t.color.text_primary)
            }
            (IconButtonVariant::Primary, false) => {
                (t.color.accent, t.color.accent_hover, t.color.text_inverse)
            }
            (IconButtonVariant::Danger, false) => {
                // Hover stays solid error red; only the brightness
                // shift reads, but the click target itself does not
                // change color — mirrors Pier's Swift reference.
                (
                    t.color.status_error,
                    t.color.status_error,
                    t.color.text_inverse,
                )
            }
        };
        let square = self.size.square();

        let mut el = div()
            .id(self.id)
            .w(square)
            .h(square)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .bg(bg)
            .text_color(fg)
            .child(UiIcon::new(self.icon).size(self.size.icon()).text_color(fg));

        if !self.disabled {
            el = el.cursor_pointer().hover(move |s| s.bg(hover_bg));
            if let Some(cb) = self.on_click {
                el = el.on_click(move |ev, win, cx| cb(ev, win, cx));
            }
        }
        if let Some(text) = self.tooltip {
            el = el.tooltip(move |win, cx| Tooltip::new(text.clone()).build(win, cx));
        }
        el
    }
}
