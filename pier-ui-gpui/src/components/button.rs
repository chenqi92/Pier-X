#![allow(dead_code)]

use gpui::{
    div, prelude::*, App, ClickEvent, ElementId, IntoElement, Pixels, Rgba, SharedString, Window,
};
use gpui_component::{tooltip::Tooltip, Icon as UiIcon, IconName};

use crate::theme::{
    heights::{BUTTON_MD_H, BUTTON_SM_H, ICON_MD, ICON_SM},
    radius::RADIUS_SM,
    spacing::{SP_2, SP_3},
    theme,
    typography::{SIZE_UI_LABEL, WEIGHT_MEDIUM},
    ui_font_with,
};

/// Visual button variants. Semantics per SKILL.md §5:
///
/// - `Primary` — the single most-expected action on the surface. Filled
///   accent.
/// - `Secondary` — a supporting action that still needs definition. Surface
///   filled with a subtle border. This is what the previous `Ghost` variant
///   used to be.
/// - `Ghost` — **truly transparent** (no background, no border). Only hover
///   reveals a tint. Use inside toolbars and for "quiet" inline actions.
/// - `Danger` — destructive actions (disconnect, delete, discard). Filled
///   error red.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// 22px high — compact toolbar / inline table row button.
    Sm,
    /// 28px high — default.
    Md,
}

impl ButtonSize {
    fn height(self) -> Pixels {
        match self {
            Self::Sm => BUTTON_SM_H,
            Self::Md => BUTTON_MD_H,
        }
    }

    fn icon_size(self) -> Pixels {
        match self {
            Self::Sm => ICON_SM,
            Self::Md => ICON_MD,
        }
    }

    /// Horizontal padding — Sm uses SP_2 (8px) so toolbar buttons read
    /// as tight as SwiftUI borderless buttons; Md uses SP_3 (12px) for
    /// dialog-primary weight.
    fn px(self) -> Pixels {
        match self {
            Self::Sm => SP_2,
            Self::Md => SP_3,
        }
    }
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    size: ButtonSize,
    width: Option<Pixels>,
    leading_icon: Option<IconName>,
    trailing_icon: Option<IconName>,
    tooltip: Option<SharedString>,
    disabled: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl Button {
    pub fn new(
        id: impl Into<ElementId>,
        variant: ButtonVariant,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant,
            // Default Sm — matches SwiftUI's default borderless button
            // height and the tighter density Pier ships. Dialogs that
            // want a chunkier primary CTA opt in via `.size(Md)`.
            size: ButtonSize::Sm,
            width: None,
            leading_icon: None,
            trailing_icon: None,
            tooltip: None,
            disabled: false,
            on_click: None,
        }
    }

    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Primary, label)
    }

    pub fn secondary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Secondary, label)
    }

    pub fn ghost(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Ghost, label)
    }

    pub fn danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Danger, label)
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn width(mut self, w: Pixels) -> Self {
        self.width = Some(w);
        self
    }

    pub fn leading_icon(mut self, icon: IconName) -> Self {
        self.leading_icon = Some(icon);
        self
    }

    pub fn trailing_icon(mut self, icon: IconName) -> Self {
        self.trailing_icon = Some(icon);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Attach a hover tooltip (SwiftUI `.help(…)` equivalent). Shown
    /// after the OS hover delay via gpui_component's shared tooltip
    /// infra — no coordinate math needed, it auto-positions.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Render the button in a disabled state — greyed fill, no
    /// hover, click handler suppressed.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

struct Palette {
    bg: Rgba,
    hover_bg: Rgba,
    fg: Rgba,
    border: Option<Rgba>,
}

fn palette_for(variant: ButtonVariant, t: &crate::theme::Theme) -> Palette {
    match variant {
        ButtonVariant::Primary => Palette {
            bg: t.color.accent,
            hover_bg: t.color.accent_hover,
            fg: t.color.text_inverse,
            border: None,
        },
        ButtonVariant::Secondary => Palette {
            bg: t.color.bg_surface,
            hover_bg: t.color.bg_hover,
            fg: t.color.text_primary,
            border: Some(t.color.border_default),
        },
        ButtonVariant::Ghost => Palette {
            // Rgba::default() == fully transparent (alpha 0). That's
            // what "true ghost" means: no fill until hover reveals one.
            bg: Rgba::default(),
            hover_bg: t.color.bg_hover,
            fg: t.color.text_primary,
            border: None,
        },
        ButtonVariant::Danger => Palette {
            bg: t.color.status_error,
            hover_bg: t.color.status_error,
            fg: t.color.text_inverse,
            border: None,
        },
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let palette = palette_for(self.variant, t);
        let icon_size = self.size.icon_size();
        let has_icon = self.leading_icon.is_some() || self.trailing_icon.is_some();
        let disabled = self.disabled;

        // Disabled buttons read as a greyed-out version of the same
        // palette: dim the fill + text, no hover, no click. Primary /
        // danger variants pick up an additional half-alpha muted bg
        // so they don't shout when disabled.
        let (bg, fg, hover_bg) = if disabled {
            (
                gpui::Rgba {
                    a: 0.35,
                    ..palette.bg
                },
                t.color.text_disabled,
                gpui::Rgba {
                    a: 0.35,
                    ..palette.bg
                },
            )
        } else {
            (palette.bg, palette.fg, palette.hover_bg)
        };

        let mut el = div()
            .id(self.id)
            .h(self.size.height())
            .px(self.size.px())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .bg(bg)
            .text_size(SIZE_UI_LABEL)
            .text_color(fg)
            .font(ui_font_with(&t.font_ui, &t.font_ui_features, WEIGHT_MEDIUM));

        if !disabled {
            el = el.cursor_pointer().hover(move |s| s.bg(hover_bg));
        }

        if has_icon {
            el = el.gap(SP_2);
        }
        if let Some(icon) = self.leading_icon {
            el = el.child(UiIcon::new(icon).size(icon_size));
        }
        el = el.child(div().child(self.label));
        if let Some(icon) = self.trailing_icon {
            el = el.child(UiIcon::new(icon).size(icon_size));
        }

        if let Some(w) = self.width {
            el = el.w(w);
        }
        if let Some(b) = palette.border {
            el = el.border_1().border_color(b);
        }
        if !disabled {
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
