//! Floating context menu anchored to a viewport point.
//!
//! `gpui_component` ships a left-click–driven `Popover`, but has no
//! primitive for a right-click context menu: the popover's trigger
//! is an *element* that owns its own click handling, whereas a
//! context menu needs to open at the mouse position of a right-click
//! on any arbitrary target.
//!
//! This component covers that gap:
//!
//! - The caller tracks the menu's open state (usually
//!   `Option<(T, Point<Pixels>)>` on the view entity — `T` is
//!   whatever the target item is, e.g. `TabIdx`).
//! - On right-click the caller sets that state to `Some(…)`.
//! - The caller renders `ContextMenu::new(id, position, …)` as the
//!   **last** child of its element tree while the state is `Some`.
//!
//! The component renders a fullscreen transparent backdrop and,
//! absolutely positioned on top of it, the menu itself. A left-click
//! on the backdrop fires the caller's `on_dismiss`; clicks on the
//! menu stop propagation so they don't bubble to the backdrop.

use gpui::{
    div, prelude::*, px, App, ClickEvent, ElementId, IntoElement, ParentElement, Pixels, Point,
    SharedString, Window,
};

use crate::theme::{
    heights::ROW_SM_H,
    radius::RADIUS_SM,
    shadow,
    spacing::{SP_0_5, SP_1, SP_3},
    theme,
    typography::{SIZE_UI_LABEL, WEIGHT_REGULAR},
    ui_font_with,
};

pub struct ContextMenuItem {
    label: SharedString,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    danger: bool,
    disabled: bool,
    separator_before: bool,
}

impl ContextMenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            on_click: None,
            danger: false,
            disabled: false,
            separator_before: false,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draw a divider rule above this item. Use to group "destructive"
    /// items at the bottom of the list.
    pub fn with_separator(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

#[derive(IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    position: Point<Pixels>,
    items: Vec<ContextMenuItem>,
    on_dismiss: Option<Box<dyn Fn(&(), &mut Window, &mut App) + 'static>>,
}

impl ContextMenu {
    pub fn new(id: impl Into<ElementId>, position: Point<Pixels>) -> Self {
        Self {
            id: id.into(),
            position,
            items: Vec::new(),
            on_dismiss: None,
        }
    }

    pub fn item(mut self, item: ContextMenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Register the "clicked anywhere else" callback — matches the
    /// shape produced by `cx.listener(|this, _: &(), window, cx| …)`.
    pub fn on_dismiss(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx);
        let on_dismiss = self.on_dismiss;

        let mut menu = div()
            .id(self.id.clone())
            .absolute()
            .left(self.position.x)
            .top(self.position.y)
            .min_w(px(180.0))
            .py(SP_1)
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .bg(t.color.bg_elevated)
            .border_1()
            .border_color(t.color.border_default)
            .rounded(RADIUS_SM)
            .shadow(shadow::popover())
            // Swallow clicks so they don't reach the backdrop.
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            });

        for (i, item) in self.items.into_iter().enumerate() {
            if item.separator_before {
                menu = menu.child(
                    div()
                        .mx(SP_1)
                        .my(SP_0_5)
                        .h(px(1.0))
                        .bg(t.color.border_subtle),
                );
            }

            let label_color = if item.disabled {
                t.color.text_disabled
            } else if item.danger {
                t.color.status_error
            } else {
                t.color.text_primary
            };
            let hover_bg = if item.danger {
                t.color.status_error
            } else {
                t.color.bg_hover
            };
            let hover_fg = if item.danger {
                t.color.text_inverse
            } else {
                t.color.text_primary
            };

            let item_id = gpui::ElementId::Name(format!("ctxmenu-item-{i}").into());
            let mut row = div()
                .id(item_id)
                .h(ROW_SM_H)
                .px(SP_3)
                .flex()
                .flex_row()
                .items_center()
                .text_size(SIZE_UI_LABEL)
                .font(ui_font_with(
                    &t.font_ui,
                    &t.font_ui_features,
                    WEIGHT_REGULAR,
                ))
                .text_color(label_color)
                .child(item.label);

            if !item.disabled {
                row = row
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg).text_color(hover_fg));
                if let Some(on_click) = item.on_click {
                    row = row.on_click(move |ev, win, cx| on_click(ev, win, cx));
                }
            }
            menu = menu.child(row);
        }

        let mut backdrop = div().absolute().top(px(0.0)).left(px(0.0)).size_full();
        if let Some(on_dismiss) = on_dismiss {
            backdrop = backdrop.on_mouse_down(gpui::MouseButton::Left, move |_, win, cx| {
                on_dismiss(&(), win, cx);
            });
        }
        backdrop.child(menu)
    }
}
