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
    div, prelude::*, px, App, ClickEvent, Context, FocusHandle, IntoElement, KeyDownEvent,
    SharedString, WeakEntity, Window,
};
use pier_core::settings::{AppSettings, AppearanceMode, TerminalCursorStyle, TerminalThemePreset};
use rust_i18n::t;

use gpui_component::{scroll::ScrollableElement, switch::Switch, WindowExt as _};

use gpui_component::IconName;

use crate::app::keybindings::{format_keystroke, is_modifier_only, resolved_keystroke, ActionId};
use crate::components::{
    Button, ButtonSize, Dropdown, DropdownOption, IconButton, IconButtonSize, IconButtonVariant,
    SettingRow,
};
use crate::i18n::{self, LOCALE_ENGLISH, LOCALE_PREFERENCE_SYSTEM, LOCALE_ZH_CN};
use crate::theme::{
    available_terminal_font_families, available_ui_font_families,
    radius::{RADIUS_MD, RADIUS_SM},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3, SP_4, SP_5},
    terminal::{available_terminal_palettes, terminal_bg_color, terminal_hex_color},
    terminal_cursor_blink, terminal_cursor_style, terminal_font_for_family,
    terminal_font_ligatures, terminal_font_size, terminal_opacity, theme,
    typography::{SIZE_BODY, SIZE_CAPTION, SIZE_H2, SIZE_SMALL, WEIGHT_EMPHASIS, WEIGHT_MEDIUM},
    update_settings, DEFAULT_UI_FONT_FAMILY,
};
use crate::widgets::{SegmentedControl, SegmentedItem, SettingsSection};

// ── Layout constants ─────────────────────────────────────────

/// Overall dialog footprint. The content pane is 1088−200 = 888 px
/// wide, enough for a 3-column theme grid with comfortable gutters.
const SETTINGS_DIALOG_W: f32 = 1088.0;
const SETTINGS_DIALOG_H: f32 = 672.0;
const SIDEBAR_W: f32 = 200.0;

// ── Section enum + dialog struct (unchanged public shape) ────

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSectionId {
    General,
    Terminal,
    Shortcuts,
}

impl SettingsSectionId {
    const ALL: [Self; 3] = [Self::General, Self::Terminal, Self::Shortcuts];

    fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Terminal => "terminal",
            Self::Shortcuts => "shortcuts",
        }
    }

    fn title(self) -> SharedString {
        match self {
            Self::General => t!("App.Settings.Sections.general_title").into(),
            Self::Terminal => t!("App.Settings.Sections.terminal_title").into(),
            Self::Shortcuts => t!("App.Settings.Sections.shortcuts_title").into(),
        }
    }
}

pub struct SettingsDialog {
    selected_section: SettingsSectionId,
    entity: WeakEntity<SettingsDialog>,
    capturing: Option<ActionId>,
    capture_focus: FocusHandle,
    capture_error: Option<SharedString>,
}

impl SettingsDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            selected_section: SettingsSectionId::General,
            entity: cx.entity().downgrade(),
            capturing: None,
            capture_focus: cx.focus_handle(),
            capture_error: None,
        }
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
        div()
            .w(px(SETTINGS_DIALOG_W))
            .h(px(SETTINGS_DIALOG_H))
            .flex()
            .flex_row()
            .bg(t.color.bg_canvas)
            .child(self.render_sidebar(&t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    // Content pane sits on bg_canvas so the inner
                    // grouped-card sections (bg_panel) can pop.
                    .bg(t.color.bg_canvas)
                    .child(self.render_content(cx)),
            )
    }
}

// ── Sidebar ──────────────────────────────────────────────────

impl SettingsDialog {
    fn render_sidebar(&self, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let locale_label =
            localized_locale_label(&i18n::resolve_locale_preference(&t.settings.ui_locale));
        let theme_label = match t.settings.appearance_mode {
            AppearanceMode::System => t!("App.Settings.General.appearance_system").to_string(),
            AppearanceMode::Dark => t!("App.Settings.General.dark").to_string(),
            AppearanceMode::Light => t!("App.Settings.General.light").to_string(),
        };

        let mut col = div()
            .w(px(SIDEBAR_W))
            .h_full()
            .px(SP_4)
            .py(SP_4)
            .flex()
            .flex_col()
            .gap(SP_0_5)
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle);

        // Brand + dialog title + current status (theme · locale).
        // Stacked tight so the group reads as a single metadata
        // block rather than three unrelated rows.
        col = col.child(
            div()
                .pb(SP_4)
                .mb(SP_2)
                .flex()
                .flex_col()
                .gap(SP_0_5)
                .border_b_1()
                .border_color(t.color.border_subtle)
                .child(
                    div()
                        .text_size(SIZE_CAPTION)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from("Pier-X")),
                )
                .child(
                    div()
                        .text_size(SIZE_H2)
                        .font_weight(WEIGHT_EMPHASIS)
                        .text_color(t.color.text_primary)
                        .child(SharedString::from(t!("App.Settings.title").to_string())),
                )
                .child(
                    div()
                        .pt(SP_0_5)
                        .text_size(SIZE_CAPTION)
                        .text_color(t.color.text_secondary)
                        .child(SharedString::from(format!(
                            "{theme_label} · {locale_label}"
                        ))),
                ),
        );

        for section in SettingsSectionId::ALL {
            let is_active = section == self.selected_section;
            let on_click = cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_section(section, cx);
            });
            col = col.child(settings_sidebar_item(
                t,
                section.id(),
                section.title(),
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
        // ── APPEARANCE section ──
        .child(
            SettingsSection::new(t!("App.Settings.General.appearance")).child(row(
                t!("App.Settings.General.appearance"),
                None,
                appearance_picker.into_any_element(),
                false,
            )),
        )
        // ── LANGUAGE section ──
        .child(
            SettingsSection::new(t!("App.Settings.General.language")).child(row(
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
        // ── TYPOGRAPHY section ──
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
        // ── CURSOR ──
        .child(
            SettingsSection::new(t!("App.Settings.Terminal.cursor"))
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
        // ── BACKGROUND ──
        .child(
            SettingsSection::new(t!("App.Settings.Terminal.background_opacity")).child(row(
                t!("App.Settings.Terminal.background_opacity"),
                None,
                opacity_stepper.into_any_element(),
                false,
            )),
        )
        // ── SHELL INTEGRATION ──
        .child(
            SettingsSection::new(t!("App.Settings.Terminal.shell_integration")).child(row(
                t!("App.Settings.Terminal.shell_integration"),
                Some(t!("App.Settings.Terminal.shell_integration_description").into()),
                integration_toggle.into_any_element(),
                true,
            )),
        )
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

// ── Opener ──────────────────────────────────────────────────

pub fn open(window: &mut Window, cx: &mut App) {
    log::info!("dialog: opening settings dialog");
    let view = cx.new(SettingsDialog::new);
    window.open_dialog(cx, move |dialog, _w, _app| {
        dialog
            .w(px(SETTINGS_DIALOG_W))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(view.clone())
    });
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
    active: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .w_full()
        .h(px(30.0))
        .px(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .rounded(RADIUS_SM)
        .bg(if active {
            t.color.accent_subtle
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
        .text_color(if active {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .child(label)
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
    let mut wrap = div().w_full().flex().flex_row().flex_wrap().gap(SP_3);
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
                // width on wider windows. When the pane shrinks,
                // flex_wrap drops to 2 columns automatically.
                .flex_1()
                .min_w(px(240.0))
                .max_w(px(320.0))
                .p(SP_4)
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_3)
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
            .px(SP_2)
            .py(px(3.0))
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
        .px(SP_2)
        .py(px(3.0))
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
