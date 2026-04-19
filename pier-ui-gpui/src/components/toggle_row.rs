//! A settings-style "title + description + switch" row.
//!
//! Consolidates a recurring shape that used to live as a private
//! helper inside [`settings_dialog.rs`] (and was buggy — the switch
//! was missing `flex_none` and got clipped at narrow widths). Once
//! you compose this component, the three-slot invariant is enforced:
//!
//! ```text
//! ┌──────────────────────────────────── [○/●]
//! │ Title                                    ^
//! │ Description body wrapping freely        flex_none
//! └──── flex_1 min_w_0 ────────────────────
//! ```

use gpui::{
    div, prelude::*, px, App, ClickEvent, IntoElement, ParentElement, SharedString, Window,
};
use gpui_component::switch::Switch;

use crate::components::text;
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_0_5, SP_2},
    theme,
};

#[derive(IntoElement)]
pub struct ToggleRow {
    id: SharedString,
    title: SharedString,
    description: Option<SharedString>,
    checked: bool,
    on_toggle: Option<Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>>,
}

impl ToggleRow {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            checked: false,
            on_toggle: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_toggle(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ToggleRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let switch_id = self.id.clone();
        let on_toggle = self.on_toggle;

        let mut label = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .child(text::ui_label(self.title));

        if let Some(description) = self.description {
            label = label.child(text::caption(description).secondary());
        }

        // The switch *must* be flex_none or it renders half-clipped when
        // the row is in a narrow container (see the original bug in
        // `settings_dialog.rs` — toggle was missing the wrapper).
        let switch = div().flex_none().child({
            let mut sw = Switch::new(switch_id).checked(self.checked);
            if let Some(cb) = on_toggle {
                sw = sw.on_click(cb);
            }
            sw
        });

        div()
            .p(SP_2)
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .rounded(RADIUS_SM)
            .bg(t.color.bg_surface)
            .border_1()
            .border_color(t.color.border_subtle)
            .child(label)
            .child(switch)
    }
}
