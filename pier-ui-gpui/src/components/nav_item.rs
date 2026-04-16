use gpui::{
    div, prelude::*, App, ClickEvent, ElementId, IntoElement, SharedString, Window,
};

use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_2, SP_3},
    theme,
    typography::{SIZE_BODY, WEIGHT_MEDIUM},
};

#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    label: SharedString,
    active: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl NavItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active: false,
            on_click: None,
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
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

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let fg = if self.active {
            t.color.text_primary
        } else {
            t.color.text_secondary
        };
        let hover_bg = t.color.bg_hover;

        let mut el = div()
            .id(self.id)
            .h(gpui::px(28.0))
            .px(SP_3)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .rounded(RADIUS_SM)
            .text_size(SIZE_BODY)
            .font_weight(WEIGHT_MEDIUM)
            .font_family(t.font_ui.clone())
            .text_color(fg)
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .child(self.label);

        if self.active {
            el = el.bg(t.color.bg_selected);
        }
        if let Some(cb) = self.on_click {
            el = el.on_click(move |ev, win, cx| cb(ev, win, cx));
        }
        el
    }
}
