//! Settings page section — macOS 14 style "grouped card" built from
//! a title (outside the card) over a rounded panel container that
//! holds its rows with hairline dividers between them.
//!
//! Why grouped cards: the prior chrome-less design (flush rows,
//! UPPERCASE caption title) read as a single flat list in zh-CN
//! where the title can't lean on uppercasing to create a landmark.
//! The grouped card turns every section into a distinct visual
//! block, the way macOS 14 System Settings does, which lets the
//! eye parse the page without having to read section titles.
//!
//! Layout:
//!
//! ```text
//! TYPOGRAPHY           ← title (outside the card)
//! ┌─────────────────────────────────── bg_panel ──┐
//! │  UI Font                          [Inter ▾]   │  ← row 1
//! │  ─────────────────────────── border_subtle ── │  ← auto divider
//! │  Terminal Font                    [Menlo ▾]   │  ← row 2
//! └───────────────────────────────────────────────┘
//! ```
//!
//! Use with [`crate::components::SettingRow`] for labeled rows. For
//! content that stands alone (a preview card, a 3-column theme
//! grid), use [`SettingsSection::untitled`] and pass the element
//! as a child — dividers are still auto-inserted between multiple
//! children, so mixing a preview and a row works as expected.

use gpui::{div, AnyElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window};

use crate::components::Separator;
use crate::theme::{
    radius::RADIUS_LG,
    spacing::{SP_1, SP_2, SP_4},
    theme,
    typography::{SIZE_BODY, WEIGHT_EMPHASIS},
};

#[derive(IntoElement, Default)]
pub struct SettingsSection {
    title: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl SettingsSection {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            children: Vec::new(),
        }
    }

    /// Section without a header. Use for a top-of-page live preview
    /// or any block that stands on its own semantically. The card
    /// chrome still applies so it visually groups with titled
    /// sections below.
    pub fn untitled() -> Self {
        Self {
            title: None,
            children: Vec::new(),
        }
    }
}

impl ParentElement for SettingsSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for SettingsSection {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);

        let mut col = div().w_full().flex().flex_col().gap(SP_2);

        if let Some(title) = self.title {
            // Title sits outside the card. 13 px emphasis weight +
            // secondary color makes it register as a landmark
            // without being as loud as a page header. Skip
            // uppercase transform when the label contains non-ASCII
            // (CJK etc.) — `.to_uppercase()` on Chinese is a no-op
            // but on mixed-script labels it looks strange.
            let label: SharedString = if title.as_ref().is_ascii() {
                SharedString::from(title.as_ref().to_uppercase())
            } else {
                title
            };
            col = col.child(
                div()
                    .pl(SP_1)
                    .text_size(SIZE_BODY)
                    .font_weight(WEIGHT_EMPHASIS)
                    .text_color(t.color.text_secondary)
                    .child(label),
            );
        }

        // Card body — rounded bg_panel surface with 1px hairline.
        // Rows inside get natural py(SP_2) from SettingRow; the
        // card itself only adds horizontal padding so dividers can
        // run edge-to-edge inside the rounded box.
        let mut body = div()
            .w_full()
            .flex()
            .flex_col()
            .px(SP_4)
            .py(SP_1)
            .rounded(RADIUS_LG)
            .bg(t.color.bg_panel)
            .border_1()
            .border_color(t.color.border_subtle);

        // Interleave dividers between children. Skip before the
        // first child — the card's top edge already separates it
        // from the title/previous section.
        for (ix, child) in self.children.into_iter().enumerate() {
            if ix > 0 {
                body = body.child(Separator::horizontal());
            }
            body = body.child(child);
        }

        col.child(body)
    }
}
