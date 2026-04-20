//! Settings dialog — macOS-System-Settings flavoured pane with an
//! 'APPEARANCE / LANGUAGE / TYPOGRAPHY' sectioned layout ported
//! from the deleted Tauri shell (commit e9d7a65, file
//! `pier-ui-tauri/src/components/SettingsDialog.tsx`). The original
//! GPUI implementation wrapped every group in a Card and fixed
//! `SettingRow` labels at 216 px, which produced the "table in a
//! table" feel the user flagged as busy.
//!
//! This rewrite leans on two widgets from `src/widgets/`:
//!
//!   * `SettingsSection` — small UPPERCASE tertiary title over a
//!     column of rows, no chrome.
//!   * `SegmentedControl` — pill picker for 2–3-way choices
//!     (language, appearance, cursor style).
//!
//! Multi-option pickers (UI font, terminal font, theme palette)
//! fall back to flex-wrapped chips and, for the terminal palette,
//! an explicit 3-column grid of preview cards — both mirror the
//! Tauri reference.

use gpui::{
    div, prelude::*, px, size, App, AppContext, Bounds, ClickEvent, Context, FocusHandle,
    IntoElement, KeyDownEvent, SharedString, WeakEntity, Window, WindowBounds, WindowOptions,
};
use pier_core::settings::{AppSettings, AppearanceMode, TerminalCursorStyle, TerminalThemePreset};
use rust_i18n::t;

use gpui_component::{scroll::ScrollableElement, switch::Switch, IconName, Root};

use crate::app::keybindings::{format_keystroke, is_modifier_only, resolved_keystroke, ActionId};
use crate::components::{
    Button, ButtonSize, Dropdown, DropdownOption, IconButton, IconButtonSize, IconButtonVariant,
    SettingRow, StatusKind, StatusPill,
};
use crate::i18n::{self, LOCALE_ENGLISH, LOCALE_PREFERENCE_SYSTEM, LOCALE_ZH_CN};
use crate::theme::{
    available_terminal_font_families, available_ui_font_families,
    radius::{RADIUS_MD, RADIUS_SM},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_4, SP_5},
    terminal::{available_terminal_palettes, terminal_bg_color, terminal_hex_color},
    terminal_cursor_blink, terminal_cursor_style, terminal_font_for_family,
    terminal_font_ligatures, terminal_font_size, terminal_opacity, theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_H2, SIZE_SMALL, WEIGHT_EMPHASIS, WEIGHT_MEDIUM},
    update_settings, DEFAULT_UI_FONT_FAMILY,
};
use crate::widgets::{SegmentedControl, SegmentedItem, SettingsSection};

// ── Layout constants ─────────────────────────────────────────

/// Settings window footprint. Opens as a standalone OS window with
/// native titlebar (SwiftUI Pier reference does the same — users
/// close via the traffic light, not an X button drawn over content).
/// Sized so the 6-item terminal theme list fits without scrolling.
const SETTINGS_WINDOW_W: f32 = 960.0;
const SETTINGS_WINDOW_H: f32 = 680.0;
const SETTINGS_WINDOW_MIN_W: f32 = 760.0;
const SETTINGS_WINDOW_MIN_H: f32 = 560.0;
const SIDEBAR_W: f32 = 200.0;

// ── Section enum + dialog struct (unchanged public shape) ────

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSectionId {
    General,
    Terminal,
    Shortcuts,
    Updates,
}

impl SettingsSectionId {
    const ALL: [Self; 4] = [
        Self::General,
        Self::Terminal,
        Self::Shortcuts,
        Self::Updates,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Terminal => "terminal",
            Self::Shortcuts => "shortcuts",
            Self::Updates => "updates",
        }
    }

    fn title(self) -> SharedString {
        match self {
            Self::General => t!("App.Settings.Sections.general_title").into(),
            Self::Terminal => t!("App.Settings.Sections.terminal_title").into(),
            Self::Shortcuts => t!("App.Settings.Sections.shortcuts_title").into(),
            Self::Updates => t!("App.Settings.Sections.updates_title").into(),
        }
    }

    /// Leading icon for the sidebar nav item. Matches Pier's SwiftUI
    /// settings sidebar where each section leads with a contextual
    /// glyph — the label alone felt under-specified in zh-CN where
    /// the three titles are short compound words.
    fn icon(self) -> IconName {
        match self {
            // Pier's SettingsView leads General with `gearshape.fill`.
            Self::General => IconName::GearFill,
            // Pier uses plain `terminal` (no frame) for the terminal
            // section — `Terminal` is the Phosphor equivalent; frame-
            // wrapped `SquareTerminal` is reserved for tab icons.
            Self::Terminal => IconName::Terminal,
            // gpui_component has no Keyboard glyph — `Settings2` is a
            // different-looking gear that at least reads as "another
            // settings subtopic" rather than duplicating `Settings`.
            Self::Shortcuts => IconName::Settings2,
            Self::Updates => IconName::RefreshCw,
        }
    }
}

/// State machine for the version-update flow. Kept on `SettingsDialog`
/// so the result persists across sidebar switches within one session.
#[derive(Clone)]
enum UpdateCheckState {
    /// No check has run this session.
    Idle,
    /// Background task in flight.
    Checking,
    /// Check completed; UI branches on `outcome.is_newer`.
    Loaded(pier_core::updates::UpdateCheckOutcome),
    /// Error string from `pier_core::updates::UpdateError::Display`.
    Failed(SharedString),
}

pub struct SettingsDialog {
    selected_section: SettingsSectionId,
    entity: WeakEntity<SettingsDialog>,
    capturing: Option<ActionId>,
    capture_focus: FocusHandle,
    capture_error: Option<SharedString>,
    update_state: UpdateCheckState,
}

impl SettingsDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            selected_section: SettingsSectionId::General,
            entity: cx.entity().downgrade(),
            capturing: None,
            capture_focus: cx.focus_handle(),
            capture_error: None,
            update_state: UpdateCheckState::Idle,
        }
    }

    fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateCheckState::Checking) {
            return;
        }
        self.update_state = UpdateCheckState::Checking;
        cx.notify();

        // Blocking HTTP is pushed to the background executor so the UI
        // thread never blocks on the network. The weak-entity pattern
        // ensures a closed settings window (entity dropped) doesn't
        // produce a ghost write when the task eventually resolves.
        cx.spawn(
            move |weak: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let background = cx.background_executor().clone();
                let mut async_cx = cx.clone();
                async move {
                    let outcome = background
                        .spawn(async move {
                            pier_core::updates::check_latest_release(env!("CARGO_PKG_VERSION"))
                        })
                        .await;
                    let next = match outcome {
                        Ok(info) => UpdateCheckState::Loaded(info),
                        Err(err) => UpdateCheckState::Failed(err.to_string().into()),
                    };
                    let _ = weak.update(&mut async_cx, move |this, cx| {
                        this.update_state = next;
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    fn select_section(&mut self, section: SettingsSectionId, cx: &mut Context<Self>) {
        if self.selected_section != section {
            self.selected_section = section;
            self.capturing = None;
            self.capture_error = None;
            cx.notify();
        }
    }

    fn start_capture(&mut self, action: ActionId, window: &mut Window, cx: &mut Context<Self>) {
        self.capturing = Some(action);
        self.capture_error = None;
        self.capture_focus.focus(window);
        cx.notify();
    }

    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if self.capturing.is_none() {
            return;
        }
        self.capturing = None;
        self.capture_error = None;
        cx.notify();
    }

    fn reset_to_default(&mut self, action: ActionId, cx: &mut Context<Self>) {
        update_settings(cx, |settings| {
            settings.keybindings.remove(action.storage_id());
        });
        cx.notify();
    }

    fn handle_capture_keydown(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.capturing else {
            return;
        };
        let keystroke = &event.keystroke;

        if is_modifier_only(keystroke) {
            return;
        }

        // Bare Escape cancels — anything else with Esc + modifier is
        // a legitimate new binding.
        if keystroke.key == "escape"
            && !keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.platform
            && !keystroke.modifiers.function
        {
            self.cancel_capture(cx);
            return;
        }

        // Require at least one modifier so a bare letter press can't
        // steal basic typing.
        if !keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.platform
            && !keystroke.modifiers.function
        {
            self.capture_error = Some(t!("App.Settings.Shortcuts.capture_need_modifier").into());
            cx.notify();
            return;
        }

        let formatted = format_keystroke(keystroke);
        let storage_id = action.storage_id().to_string();
        self.capturing = None;
        self.capture_error = None;
        crate::theme::update_settings(cx, |settings| {
            settings.keybindings.insert(storage_id, formatted);
        });
        cx.notify();
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        // Column layout — macOS keeps the embedded title strip because
        // we hide the native titlebar there; Windows uses the native
        // caption so the content starts directly under it.
        let show_embedded_titlebar =
            crate::platform::window_chrome::shows_embedded_settings_titlebar();
        let root = div().size_full().flex().flex_col().bg(t.color.bg_canvas);

        let root = if show_embedded_titlebar {
            root.child(self.render_titlebar(&t))
        } else {
            root
        };

        root.child(
            div()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_row()
                .child(self.render_sidebar(&t, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .bg(t.color.bg_canvas)
                        .child(self.render_content(cx)),
                ),
        )
    }
}

impl SettingsDialog {
    fn render_titlebar(&self, t: &crate::theme::Theme) -> impl IntoElement {
        // 40 px strip — matches Pier (SwiftUI)'s settings window titlebar
        // height. Traffic lights occupy the left 76 px; title sits
        // centered so it reads like a native macOS settings window.
        let title: SharedString = t!("App.Settings.title").to_string().into();
        div()
            .w_full()
            .h(px(40.0))
            .flex()
            .flex_row()
            .items_center()
            .bg(t.color.bg_panel)
            .border_b_1()
            .border_color(t.color.border_subtle)
            .child(div().w(px(76.0)).flex_none())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_size(SIZE_BODY)
                    .font_weight(WEIGHT_EMPHASIS)
                    .text_color(t.color.text_primary)
                    .child(title),
            )
            .child(div().w(px(76.0)).flex_none())
    }
}

// ── Sidebar ──────────────────────────────────────────────────

impl SettingsDialog {
    fn render_sidebar(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        // Sidebar: native-feeling list of nav items. No brand block —
        // the OS window titlebar already shows "设置" / "Settings"
        // as the window title, so repeating it here (the previous
        // Pier-X / 设置 / theme · locale stack) was wasted chrome.
        let mut col = div()
            .w(px(SIDEBAR_W))
            .h_full()
            .px(SP_2)
            .py(SP_2)
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle);

        for section in SettingsSectionId::ALL {
            let is_active = section == self.selected_section;
            let on_click = cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_section(section, cx);
            });
            col = col.child(settings_sidebar_item(
                t,
                section.id(),
                section.title(),
                section.icon(),
                is_active,
                Box::new(on_click),
            ));
        }

        col
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.selected_section {
            SettingsSectionId::General => self.render_general(cx).into_any_element(),
            SettingsSectionId::Terminal => self.render_terminal(cx).into_any_element(),
            SettingsSectionId::Shortcuts => self.render_shortcuts(cx).into_any_element(),
            SettingsSectionId::Updates => self.render_updates(cx).into_any_element(),
        }
    }
}

// ── Page shell — header + scrollable body with consistent padding ─

/// Shared wrapper around every tab's content. Keeps the page
/// header (title + subtitle), vertical padding, and scroll
/// semantics identical across General / Terminal / Shortcuts.
fn page_shell(
    t: &crate::theme::Theme,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
) -> gpui_component::scroll::Scrollable<gpui::Div> {
    let title: SharedString = title.into();
    let subtitle: SharedString = subtitle.into();
    div()
        .h_full()
        .overflow_y_scrollbar()
        .px(SP_5)
        .py(SP_4)
        .flex()
        .flex_col()
        // SP_6 between sections gives grouped cards room to
        // breathe; with SP_4 the cards felt stacked on top of one
        // another when multiple sections are visible at once.
        .gap(SP_5)
        .child(
            // Page header — title + subtitle, no underline rule.
            // The rule was cluttering the transition into the
            // first section title.
            div()
                .pb(SP_1)
                .flex()
                .flex_col()
                .gap(SP_0_5)
                .child(
                    div()
                        .text_size(SIZE_H2)
                        .font_weight(WEIGHT_EMPHASIS)
                        .text_color(t.color.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(SIZE_CAPTION)
                        .text_color(t.color.text_secondary)
                        .child(subtitle),
                ),
        )
}

// ── Row primitives ───────────────────────────────────────────

/// Thin adapter from the dialog's call-sites onto
/// [`crate::components::SettingRow`]. Every row in every tab
/// composes its controls into a `SettingRow` via this helper so
/// we get the SettingRow layout (label stack flex_1 with a CJK-safe
/// min-width; controls flex-none, capped width, flush-right) for
/// free.
fn row(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    controls: gpui::AnyElement,
    align_top: bool,
) -> SettingRow {
    let mut r = SettingRow::new(label);
    if let Some(desc) = description {
        r = r.description(desc);
    }
    if align_top {
        r = r.align_top();
    }
    r.child(controls)
}

// ── General tab ──────────────────────────────────────────────

impl SettingsDialog {
    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let appearance = t.settings.appearance_mode;
        let locale_pref = i18n::normalize_locale_preference(&t.settings.ui_locale);
        let active_locale = localized_locale_label(&i18n::resolve_locale_preference(&locale_pref));
        let mono_font = t.settings.terminal_font_family.clone();
        let ui_font = t
            .settings
            .ui_font_family
            .clone()
            .unwrap_or_else(|| DEFAULT_UI_FONT_FAMILY.to_string());
        let dialog = self.entity.clone();

        // ── Appearance segmented ─────────────────────────
        let appearance_picker = SegmentedControl::new()
            .item(SegmentedItem::new(
                "appearance-system",
                t!("App.Settings.General.appearance_system"),
                appearance == AppearanceMode::System,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.appearance_mode = AppearanceMode::System;
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "appearance-dark",
                t!("App.Settings.General.dark"),
                appearance == AppearanceMode::Dark,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.appearance_mode = AppearanceMode::Dark;
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "appearance-light",
                t!("App.Settings.General.light"),
                appearance == AppearanceMode::Light,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.appearance_mode = AppearanceMode::Light;
                        });
                    }
                },
            ));

        // ── Language segmented ─────────────────────────
        let language_picker = SegmentedControl::new()
            .item(SegmentedItem::new(
                "locale-system",
                t!("App.Settings.General.language_system"),
                locale_pref == LOCALE_PREFERENCE_SYSTEM,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.ui_locale = LOCALE_PREFERENCE_SYSTEM.to_string();
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "locale-en",
                t!("App.Settings.General.language_english"),
                locale_pref == LOCALE_ENGLISH,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.ui_locale = LOCALE_ENGLISH.to_string();
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "locale-zh-cn",
                t!("App.Settings.General.language_simplified_chinese"),
                locale_pref == LOCALE_ZH_CN,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.ui_locale = LOCALE_ZH_CN.to_string();
                        });
                    }
                },
            ));

        // ── Font pickers — single-select dropdowns ──────
        // Switched from flex-wrap chip rows to Dropdowns because 6+
        // options in chips pushed the wrap row wide enough to
        // collapse the CJK label column to one character per line
        // (the "字符级纵向堆叠" bug).
        let ui_font_dropdown = Dropdown::new("settings-ui-font")
            .width(px(200.0))
            .value(ui_font.clone())
            .options(
                available_ui_font_families()
                    .iter()
                    .map(|f| DropdownOption::new(*f, *f)),
            )
            .on_change({
                let dialog = dialog.clone();
                move |value, _, app| {
                    let family = value.to_string();
                    apply_dialog_settings(&dialog, app, move |s| {
                        s.ui_font_family = Some(family);
                    });
                }
            });
        let mono_font_dropdown = Dropdown::new("settings-mono-font")
            .width(px(200.0))
            .value(mono_font.clone())
            .options(
                available_terminal_font_families()
                    .iter()
                    .map(|f| DropdownOption::new(*f, *f)),
            )
            .on_change({
                let dialog = dialog.clone();
                move |value, _, app| {
                    let family = value.to_string();
                    apply_dialog_settings(&dialog, app, move |s| {
                        s.terminal_font_family = family;
                    });
                }
            });

        page_shell(
            &t,
            t!("App.Settings.Sections.general_title"),
            t!("App.Settings.General.subtitle"),
        )
        // ── APPEARANCE — single-row section; `untitled` drops the
        //   duplicate section header so the row label alone reads as
        //   the group name (matching macOS 14 System Settings). ──
        .child(SettingsSection::untitled().child(row(
            t!("App.Settings.General.appearance"),
            None,
            appearance_picker.into_any_element(),
            false,
        )))
        // ── LANGUAGE — same pattern; description stays on the row ──
        .child(
            SettingsSection::untitled().child(row(
                t!("App.Settings.General.language"),
                Some(SharedString::from(
                    t!(
                        "App.Settings.General.language_description",
                        language = active_locale.as_str()
                    )
                    .to_string(),
                )),
                language_picker.into_any_element(),
                false,
            )),
        )
        // ── TYPOGRAPHY section (keeps title — two distinct rows) ──
        .child(
            SettingsSection::new(t!("App.Settings.Sections.typography"))
                .child(row(
                    t!("App.Settings.General.ui_font_family"),
                    Some(t!("App.Settings.General.ui_font_family_description").into()),
                    ui_font_dropdown.into_any_element(),
                    false,
                ))
                .child(row(
                    t!("App.Settings.General.terminal_font_family"),
                    Some(t!("App.Settings.General.terminal_font_family_description").into()),
                    mono_font_dropdown.into_any_element(),
                    false,
                )),
        )
    }
}

// ── Terminal tab ─────────────────────────────────────────────

impl SettingsDialog {
    fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let font_size = terminal_font_size(cx);
        let cursor_style = terminal_cursor_style(cx);
        let cursor_blink = terminal_cursor_blink(cx);
        let font_ligatures = terminal_font_ligatures(cx);
        let shell_integration_on = t.settings.terminal_shell_integration;
        let theme_preset = t.settings.terminal_theme_preset;
        let opacity_pct = t.settings.terminal_opacity_pct;
        let opacity = terminal_opacity(cx);
        let palette = crate::theme::terminal::terminal_palette(theme_preset);
        let preview_font = terminal_font_for_family(&t.font_mono, font_ligatures);
        let preview_size = px(font_size);
        let preview_bg = terminal_bg_color(palette.background_hex, opacity);
        let preview_fg = terminal_hex_color(palette.foreground_hex);
        let dialog = self.entity.clone();

        // ── Preview card ──
        let preview = div()
            .w_full()
            .p(SP_4)
            .rounded(RADIUS_MD)
            .bg(preview_bg)
            .border_1()
            .border_color(t.color.border_subtle)
            .flex()
            .flex_col()
            .gap(SP_2)
            .child(
                div()
                    .text_size(SIZE_CAPTION)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_secondary)
                    .child(SharedString::from(palette.name)),
            )
            .child(
                div()
                    .font(preview_font)
                    .text_size(preview_size)
                    .line_height(px(font_size * 1.38))
                    .text_color(preview_fg)
                    .child(
                        "ssh root@prod-box\nprintf \"!= => <= ===\"\ncargo check -p pier-ui-gpui",
                    ),
            );

        // ── Theme 3-column grid ──
        let theme_grid = theme_palette_grid(&t, theme_preset, dialog.clone());

        // ── Cursor segmented ──
        let cursor_picker = SegmentedControl::new()
            .item(SegmentedItem::new(
                "cursor-block",
                t!("App.Settings.Terminal.cursor_block"),
                cursor_style == TerminalCursorStyle::Block,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.terminal_cursor_style = TerminalCursorStyle::Block;
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "cursor-underline",
                t!("App.Settings.Terminal.cursor_underline"),
                cursor_style == TerminalCursorStyle::Underline,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.terminal_cursor_style = TerminalCursorStyle::Underline;
                        });
                    }
                },
            ))
            .item(SegmentedItem::new(
                "cursor-bar",
                t!("App.Settings.Terminal.cursor_bar"),
                cursor_style == TerminalCursorStyle::Bar,
                {
                    let dialog = dialog.clone();
                    move |_, _, app| {
                        apply_dialog_settings(&dialog, app, |s| {
                            s.terminal_cursor_style = TerminalCursorStyle::Bar;
                        });
                    }
                },
            ));

        // ── Steppers ──
        let font_size_stepper = stepper_control(
            &t,
            "terminal-font-size",
            format!("{} px", font_size as u16).into(),
            Box::new({
                let dialog = dialog.clone();
                move |_, _, app| {
                    apply_dialog_settings(&dialog, app, |s| {
                        s.terminal_font_size = s.terminal_font_size.saturating_sub(1).max(10);
                    });
                }
            }),
            Box::new({
                let dialog = dialog.clone();
                move |_, _, app| {
                    apply_dialog_settings(&dialog, app, |s| {
                        s.terminal_font_size = s.terminal_font_size.saturating_add(1).min(24);
                    });
                }
            }),
        );
        let opacity_stepper = stepper_control(
            &t,
            "terminal-opacity",
            format!("{opacity_pct}%").into(),
            Box::new({
                let dialog = dialog.clone();
                move |_, _, app| {
                    apply_dialog_settings(&dialog, app, |s| {
                        s.terminal_opacity_pct = s.terminal_opacity_pct.saturating_sub(5).max(30);
                    });
                }
            }),
            Box::new({
                let dialog = dialog.clone();
                move |_, _, app| {
                    apply_dialog_settings(&dialog, app, |s| {
                        s.terminal_opacity_pct = s.terminal_opacity_pct.saturating_add(5).min(100);
                    });
                }
            }),
        );

        // ── Switches ──
        let ligatures_toggle = settings_switch(
            "settings-toggle-ligatures",
            font_ligatures,
            Box::new({
                let dialog = dialog.clone();
                move |value, _, app| {
                    apply_dialog_settings(&dialog, app, move |s| {
                        s.terminal_font_ligatures = *value;
                    });
                }
            }),
        );
        let blink_toggle = settings_switch(
            "settings-toggle-cursor-blink",
            cursor_blink,
            Box::new({
                let dialog = dialog.clone();
                move |value, _, app| {
                    apply_dialog_settings(&dialog, app, move |s| {
                        s.terminal_cursor_blink = *value;
                    });
                }
            }),
        );
        let integration_toggle = settings_switch(
            "settings-toggle-shell-integration",
            shell_integration_on,
            Box::new({
                let dialog = dialog.clone();
                move |value, _, app| {
                    let enable = *value;
                    // Platform-aware: bash profile on Unix, PowerShell
                    // profile on Windows. See pier-core's
                    // `install_local_integration`.
                    let io_result = if enable {
                        pier_core::terminal::install_local_integration()
                    } else {
                        pier_core::terminal::uninstall_local_integration()
                    };
                    if let Err(err) = &io_result {
                        log::error!(
                            "shell integration {}: {err}",
                            if enable { "install" } else { "uninstall" }
                        );
                    }
                    let persist = io_result.is_ok() && enable;
                    apply_dialog_settings(&dialog, app, move |s| {
                        s.terminal_shell_integration = persist;
                    });
                }
            }),
        );

        page_shell(
            &t,
            t!("App.Settings.Sections.terminal_title"),
            t!("App.Settings.Terminal.subtitle"),
        )
        // ── PREVIEW ──
        .child(SettingsSection::new(t!("App.Settings.Terminal.preview")).child(preview))
        // ── THEME ──
        .child(SettingsSection::new(t!("App.Settings.Terminal.theme")).child(theme_grid))
        // ── TYPOGRAPHY ──
        .child(
            SettingsSection::new(t!("App.Settings.Sections.typography"))
                .child(row(
                    t!("App.Settings.Terminal.font_size_label"),
                    None,
                    font_size_stepper.into_any_element(),
                    false,
                ))
                .child(row(
                    t!("App.Settings.Terminal.font_ligatures"),
                    Some(t!("App.Settings.Terminal.font_ligatures_description").into()),
                    ligatures_toggle.into_any_element(),
                    false,
                )),
        )
        // ── CURSOR — drop section title; two distinct row labels
        //   (光标 / 光标闪烁) already group on their own ──
        .child(
            SettingsSection::untitled()
                .child(row(
                    t!("App.Settings.Terminal.cursor"),
                    None,
                    cursor_picker.into_any_element(),
                    false,
                ))
                .child(row(
                    t!("App.Settings.Terminal.cursor_blink"),
                    Some(t!("App.Settings.Terminal.cursor_blink_description").into()),
                    blink_toggle.into_any_element(),
                    false,
                )),
        )
        // ── BACKGROUND — single row, untitled ──
        .child(SettingsSection::untitled().child(row(
            t!("App.Settings.Terminal.background_opacity"),
            None,
            opacity_stepper.into_any_element(),
            false,
        )))
        // ── SHELL INTEGRATION — single row, untitled ──
        .child(SettingsSection::untitled().child(row(
            t!("App.Settings.Terminal.shell_integration"),
            Some(t!("App.Settings.Terminal.shell_integration_description").into()),
            integration_toggle.into_any_element(),
            true,
        )))
    }
}

// ── Shortcuts tab ────────────────────────────────────────────

impl SettingsDialog {
    fn render_shortcuts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let settings = t.settings.clone();
        let capturing = self.capturing;
        let capture_error = self.capture_error.clone();
        let focus = self.capture_focus.clone();

        let mut section = SettingsSection::new(t!("App.Settings.Sections.shortcuts_title"));
        for action in ActionId::ALL {
            let stroke = resolved_keystroke(&settings, action);
            let is_capturing = capturing == Some(action);
            let is_overridden = settings.keybindings.contains_key(action.storage_id());
            let row_error = if is_capturing {
                capture_error.clone()
            } else {
                None
            };
            section = section.child(
                shortcut_row(
                    &t,
                    action,
                    stroke.into(),
                    is_capturing,
                    is_overridden,
                    row_error,
                    focus.clone(),
                    cx,
                )
                .into_any_element(),
            );
        }

        page_shell(
            &t,
            t!("App.Settings.Sections.shortcuts_title"),
            t!("App.Settings.Shortcuts.subtitle"),
        )
        .child(section)
    }
}

// ── Updates tab ──────────────────────────────────────────────

impl SettingsDialog {
    fn render_updates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let current_version: SharedString = env!("CARGO_PKG_VERSION").to_string().into();

        // Current / latest version row.
        let version_row = div()
            .w_full()
            .py(SP_1_5)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(SIZE_BODY)
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(t.color.text_primary)
                    .child(SharedString::from(
                        t!("App.Settings.Updates.current_version").to_string(),
                    )),
            )
            .child(
                div()
                    .font_family(t.font_mono.clone())
                    .text_size(SIZE_CAPTION)
                    .text_color(t.color.text_secondary)
                    .child(current_version.clone()),
            );

        // Action + status block depends on state machine.
        let (button_label, button_enabled): (SharedString, bool) = match &self.update_state {
            UpdateCheckState::Idle => (
                t!("App.Settings.Updates.check_button").to_string().into(),
                true,
            ),
            UpdateCheckState::Checking => (
                t!("App.Settings.Updates.checking").to_string().into(),
                false,
            ),
            UpdateCheckState::Loaded(_) | UpdateCheckState::Failed(_) => (
                t!("App.Settings.Updates.check_again").to_string().into(),
                true,
            ),
        };

        let check_button = {
            let mut b = Button::primary("settings-updates-check", button_label);
            if button_enabled {
                b = b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.start_update_check(cx);
                }));
            }
            b
        };

        // Status surface — a status pill + optional release metadata.
        let status = match &self.update_state {
            UpdateCheckState::Idle => None,
            UpdateCheckState::Checking => Some(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .child(StatusPill::new(
                        SharedString::from(t!("App.Settings.Updates.checking").to_string()),
                        StatusKind::Info,
                    ))
                    .into_any_element(),
            ),
            UpdateCheckState::Loaded(outcome) => Some(update_result_block(&t, outcome, cx)),
            UpdateCheckState::Failed(err) => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(SP_1)
                    .child(StatusPill::new(
                        SharedString::from(t!("App.Settings.Sections.updates_title").to_string()),
                        StatusKind::Error,
                    ))
                    .child(
                        div()
                            .text_size(SIZE_SMALL)
                            .text_color(t.color.text_secondary)
                            .child(SharedString::from(format!(
                                "{}{err}",
                                t!("App.Settings.Updates.error_prefix")
                            ))),
                    )
                    .into_any_element(),
            ),
        };

        let mut body = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .child(SettingsSection::untitled().child(version_row))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_2)
                    .child(check_button),
            );
        if let Some(status_el) = status {
            body = body.child(status_el);
        }

        page_shell(
            &t,
            t!("App.Settings.Sections.updates_title"),
            t!("App.Settings.Updates.subtitle"),
        )
        .child(body)
    }
}

fn update_result_block(
    t: &crate::theme::Theme,
    outcome: &pier_core::updates::UpdateCheckOutcome,
    cx: &mut Context<SettingsDialog>,
) -> gpui::AnyElement {
    let (pill_label, pill_kind): (SharedString, StatusKind) = if outcome.is_newer {
        (
            SharedString::from(t!("App.Settings.Updates.available").to_string()),
            StatusKind::Info,
        )
    } else {
        (
            SharedString::from(t!("App.Settings.Updates.up_to_date").to_string()),
            StatusKind::Success,
        )
    };

    let latest_line: SharedString = format!(
        "{}: {}",
        t!("App.Settings.Updates.latest_version"),
        outcome.latest_version
    )
    .into();

    let mut col = div()
        .w_full()
        .flex()
        .flex_col()
        .gap(SP_2)
        .child(StatusPill::new(pill_label, pill_kind))
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(latest_line),
        );

    if let Some(published) = outcome.published_at.as_ref() {
        let formatted: SharedString = format!(
            "{}",
            t!(
                "App.Settings.Updates.released_at",
                date = published.as_str()
            )
        )
        .into();
        col = col.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(formatted),
        );
    }

    if let Some(notes) = outcome.release_notes.as_ref() {
        // Truncate release notes — full markdown rendering is out of
        // scope for this surface; we just show the first 600 chars so
        // users get a taste and open the browser for the rest.
        let trimmed: SharedString = if notes.chars().count() > 600 {
            let mut s: String = notes.chars().take(600).collect();
            s.push('…');
            s.into()
        } else {
            notes.clone().into()
        };
        col = col.child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_secondary)
                .child(trimmed),
        );
    }

    // Release-page link — always shown so the user can always open
    // notes / assets in the browser (not just when there's an update).
    let url = outcome.release_url.clone();
    col = col.child(
        div().child(
            Button::secondary(
                "settings-updates-open",
                t!("App.Settings.Updates.open_release").to_string(),
            )
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.open_url(&url);
            })),
        ),
    );
    col.into_any_element()
}

// ── Opener ──────────────────────────────────────────────────

pub fn open(_window: &mut Window, cx: &mut App) {
    log::info!("settings: opening standalone window");

    // Standalone OS window — not a modal overlay. Native titlebar
    // with a title, traffic lights for close/min/max, resizable.
    // Matches the Pier SwiftUI reference where settings is its own
    // window (never clipped by the main app bounds, never has its
    // close button obscured by dialog chrome).
    let bounds = Bounds::centered(None, size(px(SETTINGS_WINDOW_W), px(SETTINGS_WINDOW_H)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(SETTINGS_WINDOW_MIN_W), px(SETTINGS_WINDOW_MIN_H))),
        // macOS keeps the in-theme transparent strip; Windows uses the
        // native caption so close/min/max and the system menu stay
        // available without custom hit-testing.
        titlebar: Some(crate::platform::window_chrome::settings_window_titlebar()),
        app_id: Some("com.pier-x.settings".into()),
        ..Default::default()
    };

    if let Err(err) = cx.open_window(options, |window, cx| {
        let view = cx.new(SettingsDialog::new);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        log::error!("settings: failed to open window: {err}");
    }
}

fn apply_dialog_settings(
    dialog: &WeakEntity<SettingsDialog>,
    app: &mut App,
    update: impl FnOnce(&mut AppSettings),
) {
    update_settings(app, update);
    let _ = dialog.update(app, |_, cx| cx.notify());
}

// ── Sidebar item ────────────────────────────────────────────

fn settings_sidebar_item(
    t: &crate::theme::Theme,
    id: &'static str,
    label: SharedString,
    icon: IconName,
    active: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    // Nav item — icon + label. Matches macOS System Settings / Pier
    // SwiftUI sidebar rhythm: compact (28px), neutral selection fill
    // (bg_selected, not accent_subtle) so the sidebar feels like a
    // list, not a strip of activated buttons.
    let fg = if active {
        t.color.text_primary
    } else {
        t.color.text_secondary
    };
    div()
        .id(gpui::ElementId::Name(id.into()))
        .w_full()
        .h(crate::theme::heights::ROW_MD_H)
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .bg(if active {
            t.color.bg_selected
        } else {
            t.color.bg_panel
        })
        .cursor_pointer()
        .when(!active, |style| style.hover(|s| s.bg(t.color.bg_hover)))
        .on_click(on_click)
        .text_size(SIZE_BODY)
        .font_weight(if active {
            WEIGHT_EMPHASIS
        } else {
            WEIGHT_MEDIUM
        })
        .text_color(fg)
        .child(
            gpui_component::Icon::new(icon)
                .size(crate::theme::heights::ICON_SM)
                .text_color(fg),
        )
        .child(div().flex_1().min_w(px(0.0)).child(label))
}

// ── Theme palette grid (3 columns) ──────────────────────────

fn theme_palette_grid(
    t: &crate::theme::Theme,
    selected: TerminalThemePreset,
    dialog: WeakEntity<SettingsDialog>,
) -> impl IntoElement {
    // 3-column grid built with flex-wrap + fixed-percentage widths.
    // GPUI doesn't ship a native grid primitive yet — computing
    // `calc((100% - 2*gap) / 3)` with flex works identically for
    // our row count and keeps the look consistent across widths.
    let palettes = available_terminal_palettes();
    let mut wrap = div().w_full().flex().flex_row().flex_wrap().gap(SP_2);
    for palette in palettes {
        let palette = *palette;
        let preset = palette.preset;
        let is_active = preset == selected;
        let dialog_for_click = dialog.clone();
        let id: SharedString = format!("settings-theme-{:?}", preset).into();
        wrap = wrap.child(
            div()
                .id(gpui::ElementId::Name(id))
                // Responsive: claim a flex share with a minimum so
                // three cards fit a ~880 px content pane, and a
                // maximum so cards don't stretch past readable
                // width on wider windows. Tightened from 240/320 to
                // 200/260 + smaller padding so the 3x2 grid reads as
                // compact chips, not oversized splash cards.
                .flex_1()
                .min_w(px(200.0))
                .max_w(px(260.0))
                .p(SP_2)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(if is_active {
                    t.color.accent
                } else {
                    t.color.border_subtle
                })
                .bg(if is_active {
                    t.color.accent_subtle
                } else {
                    t.color.bg_surface
                })
                .cursor_pointer()
                .hover(|s| s.bg(t.color.bg_hover))
                .on_click(move |_, _, app| {
                    apply_dialog_settings(&dialog_for_click, app, move |s| {
                        s.terminal_theme_preset = preset;
                    });
                })
                // Color dots cluster.
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(SP_0_5)
                        .child(color_dot(palette.background_hex))
                        .child(color_dot(palette.foreground_hex))
                        .child(color_dot(palette.cursor_bg_hex))
                        .child(color_dot(palette.ansi_16[2]))
                        .child(color_dot(palette.ansi_16[1])),
                )
                // Name + description stack.
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(SIZE_BODY)
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(if is_active {
                                    t.color.accent
                                } else {
                                    t.color.text_primary
                                })
                                .child(SharedString::from(palette.name)),
                        )
                        .child(
                            div()
                                .text_size(SIZE_SMALL)
                                .text_color(t.color.text_tertiary)
                                .child(SharedString::from(palette_description_key(preset))),
                        ),
                ),
        );
    }
    wrap
}

fn palette_description_key(preset: TerminalThemePreset) -> String {
    match preset {
        TerminalThemePreset::DefaultDark | TerminalThemePreset::DefaultLight => {
            t!("App.Settings.Terminal.Palettes.default").to_string()
        }
        TerminalThemePreset::SolarizedDark => {
            t!("App.Settings.Terminal.Palettes.solarized_dark").to_string()
        }
        TerminalThemePreset::Dracula => t!("App.Settings.Terminal.Palettes.dracula").to_string(),
        TerminalThemePreset::Monokai => t!("App.Settings.Terminal.Palettes.monokai").to_string(),
        TerminalThemePreset::Nord => t!("App.Settings.Terminal.Palettes.nord").to_string(),
    }
}

fn color_dot(hex: u32) -> impl IntoElement {
    div()
        .w(px(12.0))
        .h(px(12.0))
        .rounded(px(999.0))
        .bg(terminal_hex_color(hex))
}

// ── Steppers + step buttons ─────────────────────────────────

fn stepper_control(
    t: &crate::theme::Theme,
    id_prefix: &'static str,
    value: SharedString,
    on_minus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    on_plus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_canvas)
        .border_1()
        .border_color(t.color.border_subtle)
        .p(px(2.0))
        .child(icon_step_button(t, id_prefix, "-", on_minus))
        .child(
            div()
                .min_w(px(56.0))
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .text_center()
                .child(value),
        )
        .child(icon_step_button(t, id_prefix, "+", on_plus))
}

fn icon_step_button(
    t: &crate::theme::Theme,
    id_prefix: &'static str,
    label: &'static str,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let id: SharedString = format!("settings-step-{id_prefix}-{label}").into();
    div()
        .id(gpui::ElementId::Name(id))
        .w(px(22.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .text_color(t.color.text_secondary)
        .text_size(SIZE_BODY)
        .font_weight(WEIGHT_MEDIUM)
        .cursor_pointer()
        .hover(|s| s.bg(t.color.bg_hover).text_color(t.color.text_primary))
        .on_click(on_click)
        .child(label)
}

// ── Shortcut row ─────────────────────────────────────────────

fn shortcut_row(
    t: &crate::theme::Theme,
    action: ActionId,
    keystroke: SharedString,
    is_capturing: bool,
    is_overridden: bool,
    capture_error: Option<SharedString>,
    capture_focus: FocusHandle,
    cx: &mut Context<SettingsDialog>,
) -> impl IntoElement {
    let label = action.label();

    let label_stack = div()
        .flex_1()
        .min_w(px(0.0))
        .truncate()
        .text_size(SIZE_BODY)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(t.color.text_primary)
        .child(label);

    let keystroke_el = if is_capturing {
        shortcut_capture_pad(t, capture_error, capture_focus, cx).into_any_element()
    } else {
        div()
            .flex_none()
            // Match button height so kbd pill and "更改" button sit
            // on the same baseline rather than floating mid-row.
            .h(crate::theme::heights::BUTTON_SM_H)
            .px(SP_2)
            .flex()
            .items_center()
            .rounded(RADIUS_SM)
            .bg(t.color.bg_canvas)
            .border_1()
            .border_color(if is_overridden {
                t.color.accent_muted
            } else {
                t.color.border_subtle
            })
            .font_family(t.font_mono.clone())
            .text_size(SIZE_CAPTION)
            .text_color(t.color.text_primary)
            .child(keystroke)
            .into_any_element()
    };

    let change_label: SharedString = if is_capturing {
        t!("App.Common.cancel").into()
    } else {
        t!("App.Settings.Shortcuts.change").into()
    };
    let change_id = format!("shortcut-change-{}", action.storage_id());
    let change_btn = Button::secondary(gpui::ElementId::Name(change_id.into()), change_label)
        .size(ButtonSize::Sm)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            if this.capturing == Some(action) {
                this.cancel_capture(cx);
            } else {
                this.start_capture(action, window, cx);
            }
        }))
        .into_any_element();

    let mut right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .child(keystroke_el)
        .child(change_btn);

    if is_overridden {
        let reset_id = format!("shortcut-reset-{}", action.storage_id());
        // Icon button instead of a text button to keep the row
        // from overflowing when the keystroke pill and Change
        // button are already eating horizontal space.
        right = right.child(
            IconButton::new(gpui::ElementId::Name(reset_id.into()), IconName::Undo)
                .variant(IconButtonVariant::Ghost)
                .size(IconButtonSize::Sm)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.reset_to_default(action, cx);
                })),
        );
    }

    div()
        .w_full()
        .min_h(px(40.0))
        .py(SP_1_5)
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(SP_4)
        .rounded(RADIUS_SM)
        .when(is_capturing, |el| el.bg(t.color.accent_subtle).px(SP_2))
        .child(label_stack)
        .child(right)
}

fn shortcut_capture_pad(
    t: &crate::theme::Theme,
    capture_error: Option<SharedString>,
    focus: FocusHandle,
    cx: &mut Context<SettingsDialog>,
) -> impl IntoElement {
    let prompt: SharedString = capture_error.unwrap_or_else(|| {
        SharedString::from(t!("App.Settings.Shortcuts.capture_prompt").to_string())
    });
    div()
        // Same height as the idle kbd pill so entering capture mode
        // doesn't shift the row vertically.
        .h(crate::theme::heights::BUTTON_SM_H)
        .px(SP_2)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_canvas)
        .border_1()
        .border_color(t.color.accent)
        .font_family(t.font_mono.clone())
        .text_size(SIZE_CAPTION)
        .text_color(t.color.accent)
        .child(prompt)
        .track_focus(&focus)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            this.handle_capture_keydown(event, window, cx);
        }))
}

// ── Switch ──────────────────────────────────────────────────

fn settings_switch(
    id: impl Into<SharedString>,
    checked: bool,
    on_toggle: Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    Switch::new(gpui::ElementId::Name(id.into()))
        .checked(checked)
        .on_click(on_toggle)
}

// ── Locale label helper ─────────────────────────────────────

fn localized_locale_label(locale: &str) -> String {
    match locale {
        LOCALE_ENGLISH => t!("App.Settings.General.language_english").to_string(),
        LOCALE_ZH_CN => t!("App.Settings.General.language_simplified_chinese").to_string(),
        _ => t!("App.Settings.General.language_system").to_string(),
    }
}
