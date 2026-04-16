use gpui::{
    div, prelude::*, px, App, ClickEvent, ElementId, IntoElement, Pixels, SharedString, Window,
};

use crate::theme::{
    radius::RADIUS_SM,
    spacing::SP_3,
    theme,
    typography::{SIZE_BODY, WEIGHT_MEDIUM},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Ghost,
    Icon,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    width: Option<Pixels>,
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
            width: None,
            on_click: None,
        }
    }

    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Primary, label)
    }

    pub fn ghost(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Ghost, label)
    }

    pub fn icon(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonVariant::Icon, label)
    }

    pub fn width(mut self, w: Pixels) -> Self {
        self.width = Some(w);
        self
    }

    pub fn on_click(
        mut self,
        f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let (bg, hover_bg, fg, border) = match self.variant {
            ButtonVariant::Primary => (t.color.accent, t.color.accent_hover, t.color.text_inverse, None),
            ButtonVariant::Ghost => (
                t.color.bg_surface,
                t.color.bg_hover,
                t.color.text_primary,
                Some(t.color.border_default),
            ),
            ButtonVariant::Icon => (
                t.color.bg_surface,
                t.color.bg_hover,
                t.color.text_secondary,
                None,
            ),
        };

        let mut el = div()
            .id(self.id)
            .h(px(28.0))
            .px(SP_3)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_SM)
            .bg(bg)
            .text_size(SIZE_BODY)
            .font_weight(WEIGHT_MEDIUM)
            .font_family(t.font_ui.clone())
            .text_color(fg)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(self.label);

        if let Some(w) = self.width {
            el = el.w(w);
        }
        if let Some(b) = border {
            el = el.border_1().border_color(b);
        }
        if let Some(cb) = self.on_click {
            el = el.on_click(move |ev, win, cx| cb(ev, win, cx));
        }
        el
    }
}
