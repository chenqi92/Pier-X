use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, FocusHandle, IntoElement, KeyDownEvent,
    SharedString, WeakEntity, Window,
};
use pier_core::settings::{AppSettings, AppearanceMode, TerminalCursorStyle, TerminalThemePreset};
use rust_i18n::t;

use gpui_component::{scroll::ScrollableElement, switch::Switch, WindowExt as _};

use crate::app::keybindings::{format_keystroke, is_modifier_only, resolved_keystroke, ActionId};
use crate::components::{Button, ButtonSize, Card, SectionLabel, SettingRow};
use crate::i18n::{self, LOCALE_ENGLISH, LOCALE_PREFERENCE_SYSTEM, LOCALE_ZH_CN};
use crate::theme::{
    available_terminal_font_families, available_ui_font_families,
    heights::BUTTON_MD_H,
    radius::{RADIUS_MD, RADIUS_SM},
    spacing::{SP_0_5, SP_1, SP_1_5, SP_2, SP_3, SP_4, SP_5, SP_6, SP_8},
    terminal::{available_terminal_palettes, terminal_bg_color, terminal_hex_color},
    terminal_cursor_blink, terminal_cursor_style, terminal_font_for_family,
    terminal_font_ligatures, terminal_font_size, terminal_opacity, theme,
    typography::{
        SIZE_BODY, SIZE_CAPTION, SIZE_H1, SIZE_H3, SIZE_SMALL, WEIGHT_EMPHASIS, WEIGHT_MEDIUM,
    },
    update_settings, DEFAULT_UI_FONT_FAMILY,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    General,
    Terminal,
    Shortcuts,
}

const SETTINGS_DIALOG_W: f32 = 1088.0;
const SETTINGS_DIALOG_H: f32 = 672.0;
const SETTINGS_DIALOG_OUTER_H: f32 = 760.0;
const SETTINGS_DIALOG_MIN_TOP: f32 = 20.0;

impl SettingsSection {
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
    selected_section: SettingsSection,
    entity: WeakEntity<SettingsDialog>,
    /// Which shortcut is waiting for a captured keystroke, if any.
    capturing: Option<ActionId>,
    /// Focus target for the capture div. Focused when `capturing`
    /// goes `Some` so `on_key_down` fires.
    capture_focus: FocusHandle,
    /// Last capture error message (e.g. "no modifier"). Cleared
    /// when capture exits or a new capture starts.
    capture_error: Option<SharedString>,
}

impl SettingsDialog {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            selected_section: SettingsSection::General,
            entity: cx.entity().downgrade(),
            capturing: None,
            capture_focus: cx.focus_handle(),
            capture_error: None,
        }
    }

    fn select_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
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
        crate::theme::update_settings(cx, |settings| {
            settings.keybindings.remove(action.storage_id());
        });
        self.capturing = None;
        self.capture_error = None;
        cx.notify();
    }

    fn handle_capture_keydown(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.capturing else {
            return;
        };

        let keystroke = &event.keystroke;

        // Still in the middle of pressing the combo.
        if is_modifier_only(keystroke) {
            return;
        }

        // Escape with no modifiers cancels capture.
        if keystroke.key == "escape"
            && !keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.shift
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
            .bg(t.color.bg_surface)
            .child(self.render_sidebar(&t, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .bg(t.color.bg_surface)
                    .child(self.render_content(cx)),
            )
    }
}

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
            .w(px(192.0))
            .h_full()
            .px(SP_3)
            .py(SP_4)
            .flex()
            .flex_col()
            .gap(SP_1)
            .bg(t.color.bg_panel)
            .border_r_1()
            .border_color(t.color.border_subtle)
            .child(
                div()
                    .pb(SP_3)
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
                            .pt(SP_1)
                            .text_size(SIZE_H3)
                            .font_weight(WEIGHT_EMPHASIS)
                            .text_color(t.color.text_primary)
                            .child(SharedString::from(t!("App.Settings.title").to_string())),
                    )
                    .child(
                        div()
                            .pt(SP_1)
                            .text_size(SIZE_CAPTION)
                            .text_color(t.color.text_secondary)
                            .child(SharedString::from(format!(
                                "{theme_label} · {locale_label}"
                            ))),
                    ),
            );

        for section in SettingsSection::ALL {
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
            SettingsSection::General => self.render_general(cx).into_any_element(),
            SettingsSection::Terminal => self.render_terminal(cx).into_any_element(),
            SettingsSection::Shortcuts => self.render_shortcuts(cx).into_any_element(),
        }
    }

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

        section_shell(
            &t,
            t!("App.Settings.Sections.general_title"),
            t!("App.Settings.General.subtitle"),
        )
        .child(
            settings_card()
                .child(
                    SettingRow::new(t!("App.Settings.General.language"))
                        .description(SharedString::from(
                            t!(
                                "App.Settings.General.language_description",
                                language = active_locale.as_str()
                            )
                            .to_string(),
                        ))
                        .align_top()
                        .child(choice_wrap(vec![
                            choice_chip(
                                &t,
                                "locale-system",
                                t!("App.Settings.General.language_system"),
                                locale_pref == LOCALE_PREFERENCE_SYSTEM,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.ui_locale =
                                                LOCALE_PREFERENCE_SYSTEM.to_string();
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "locale-en",
                                t!("App.Settings.General.language_english"),
                                locale_pref == LOCALE_ENGLISH,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.ui_locale = LOCALE_ENGLISH.to_string();
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "locale-zh-cn",
                                t!("App.Settings.General.language_simplified_chinese"),
                                locale_pref == LOCALE_ZH_CN,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.ui_locale = LOCALE_ZH_CN.to_string();
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                        ])),
                )
                .child(
                    SettingRow::new(t!("App.Settings.General.appearance"))
                        .align_top()
                        .child(choice_wrap(vec![
                            choice_chip(
                                &t,
                                "appearance-system",
                                t!("App.Settings.General.appearance_system"),
                                appearance == AppearanceMode::System,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.appearance_mode = AppearanceMode::System;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "appearance-dark",
                                t!("App.Settings.General.dark"),
                                appearance == AppearanceMode::Dark,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.appearance_mode = AppearanceMode::Dark;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "appearance-light",
                                t!("App.Settings.General.light"),
                                appearance == AppearanceMode::Light,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.appearance_mode = AppearanceMode::Light;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                        ])),
                ),
        )
        .child(
            settings_card()
                .child(
                    SettingRow::new(t!("App.Settings.General.ui_font_family"))
                        .description(t!("App.Settings.General.ui_font_family_description"))
                        .align_top()
                        .child(choice_wrap(
                            available_ui_font_families()
                                .iter()
                                .map(|family| {
                                    let family_name = (*family).to_string();
                                    let id = format!(
                                        "ui-font-family-{}",
                                        family_name.to_ascii_lowercase().replace([' ', '.'], "-")
                                    );
                                    choice_chip(
                                        &t,
                                        id,
                                        *family,
                                        ui_font == family_name,
                                        Box::new({
                                            let dialog = dialog.clone();
                                            move |_, _, app| {
                                                let family_name = family_name.clone();
                                                apply_dialog_settings(
                                                    &dialog,
                                                    app,
                                                    move |settings| {
                                                        if family_name == DEFAULT_UI_FONT_FAMILY {
                                                            settings.ui_font_family = None;
                                                        } else {
                                                            settings.ui_font_family =
                                                                Some(family_name.clone());
                                                        }
                                                    },
                                                );
                                            }
                                        }),
                                    )
                                    .into_any_element()
                                })
                                .collect(),
                        )),
                )
                .child(
                    SettingRow::new(t!("App.Settings.General.terminal_font_family"))
                        .description(t!("App.Settings.General.terminal_font_family_description"))
                        .align_top()
                        .child(choice_wrap(
                            available_terminal_font_families()
                                .iter()
                                .map(|family| {
                                    let family_name = (*family).to_string();
                                    let id = format!(
                                        "font-family-{}",
                                        family_name.to_ascii_lowercase().replace(' ', "-")
                                    );
                                    choice_chip(
                                        &t,
                                        id,
                                        *family,
                                        mono_font == family_name,
                                        Box::new({
                                            let dialog = dialog.clone();
                                            move |_, _, app| {
                                                let family_name = family_name.clone();
                                                apply_dialog_settings(
                                                    &dialog,
                                                    app,
                                                    move |settings| {
                                                        settings.terminal_font_family =
                                                            family_name.clone();
                                                    },
                                                );
                                            }
                                        }),
                                    )
                                    .into_any_element()
                                })
                                .collect(),
                        )),
                ),
        )
    }

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

        section_shell(
            &t,
            t!("App.Settings.Sections.terminal_title"),
            t!("App.Settings.Terminal.subtitle"),
        )
        .child(
            settings_card()
                .child(SectionLabel::new(t!("App.Settings.Terminal.preview")))
                .child(
                    div()
                        .p(SP_3)
                        .rounded(RADIUS_MD)
                        .bg(preview_bg)
                        .border_1()
                        .border_color(t.color.border_subtle)
                        .child(
                            div()
                                .pb(SP_2)
                                .text_size(SIZE_CAPTION)
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(t.color.text_secondary)
                                .child(SharedString::from(
                                    t!("App.Settings.Terminal.Palettes.default").to_string(),
                                )),
                        )
                        .child(
                            div()
                                .font(preview_font)
                                .text_size(preview_size)
                                .line_height(px(font_size * 1.38))
                                .text_color(preview_fg)
                                .child("ssh root@prod-box\nprintf \"!= => <= ===\"\ncargo check -p pier-ui-gpui"),
                        ),
                ),
        )
        .child(
            settings_card()
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.theme"))
                        .align_top()
                        .child(
                            choice_wrap(
                                available_terminal_palettes()
                                    .iter()
                                    .map(|palette| {
                                        let palette = *palette;
                                        let preset = palette.preset;
                                        terminal_theme_option(
                                            &t,
                                            palette,
                                            theme_preset == preset,
                                            Box::new({
                                                let dialog = dialog.clone();
                                                move |_, _, app| {
                                                    apply_dialog_settings(
                                                        &dialog,
                                                        app,
                                                        move |settings| {
                                                            settings.terminal_theme_preset =
                                                                preset;
                                                        },
                                                    );
                                                }
                                            }),
                                        )
                                        .into_any_element()
                                    })
                                    .collect(),
                            ),
                        ),
                )
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.background_opacity")).child(
                        stepper_control(
                            &t,
                            "terminal-opacity",
                            format!("{opacity_pct}%").into(),
                            Box::new({
                                let dialog = dialog.clone();
                                move |_, _, app| {
                                    apply_dialog_settings(&dialog, app, |settings| {
                                        settings.terminal_opacity_pct = settings
                                            .terminal_opacity_pct
                                            .saturating_sub(5)
                                            .max(30);
                                    });
                                }
                            }),
                            Box::new({
                                let dialog = dialog.clone();
                                move |_, _, app| {
                                    apply_dialog_settings(&dialog, app, |settings| {
                                        settings.terminal_opacity_pct = settings
                                            .terminal_opacity_pct
                                            .saturating_add(5)
                                            .min(100);
                                    });
                                }
                            }),
                        ),
                    ),
                ),
        )
        .child(
            settings_card()
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.font_size_label")).child(
                        stepper_control(
                            &t,
                            "terminal-font-size",
                            format!("{} px", font_size as u16).into(),
                            Box::new({
                                let dialog = dialog.clone();
                                move |_, _, app| {
                                    apply_dialog_settings(&dialog, app, |settings| {
                                        settings.terminal_font_size = settings
                                            .terminal_font_size
                                            .saturating_sub(1)
                                            .max(10);
                                    });
                                }
                            }),
                            Box::new({
                                let dialog = dialog.clone();
                                move |_, _, app| {
                                    apply_dialog_settings(&dialog, app, |settings| {
                                        settings.terminal_font_size = settings
                                            .terminal_font_size
                                            .saturating_add(1)
                                            .min(24);
                                    });
                                }
                            }),
                        ),
                    ),
                )
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.font_ligatures"))
                        .description(t!("App.Settings.Terminal.font_ligatures_description"))
                        .child(settings_switch(
                            "settings-toggle-ligatures",
                            font_ligatures,
                            Box::new({
                                let dialog = dialog.clone();
                                move |value, _, app| {
                                    apply_dialog_settings(&dialog, app, move |settings| {
                                        settings.terminal_font_ligatures = *value;
                                    });
                                }
                            }),
                        )),
                )
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.cursor"))
                        .align_top()
                        .child(choice_wrap(vec![
                            choice_chip(
                                &t,
                                "cursor-block",
                                t!("App.Settings.Terminal.cursor_block"),
                                cursor_style == TerminalCursorStyle::Block,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.terminal_cursor_style =
                                                TerminalCursorStyle::Block;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "cursor-underline",
                                t!("App.Settings.Terminal.cursor_underline"),
                                cursor_style == TerminalCursorStyle::Underline,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.terminal_cursor_style =
                                                TerminalCursorStyle::Underline;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                            choice_chip(
                                &t,
                                "cursor-bar",
                                t!("App.Settings.Terminal.cursor_bar"),
                                cursor_style == TerminalCursorStyle::Bar,
                                Box::new({
                                    let dialog = dialog.clone();
                                    move |_, _, app| {
                                        apply_dialog_settings(&dialog, app, |settings| {
                                            settings.terminal_cursor_style =
                                                TerminalCursorStyle::Bar;
                                        });
                                    }
                                }),
                            )
                            .into_any_element(),
                        ])),
                )
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.cursor_blink"))
                        .description(t!("App.Settings.Terminal.cursor_blink_description"))
                        .child(settings_switch(
                            "settings-toggle-cursor-blink",
                            cursor_blink,
                            Box::new({
                                let dialog = dialog.clone();
                                move |value, _, app| {
                                    apply_dialog_settings(&dialog, app, move |settings| {
                                        settings.terminal_cursor_blink = *value;
                                    });
                                }
                            }),
                        )),
                )
                .child(
                    SettingRow::new(t!("App.Settings.Terminal.shell_integration"))
                        .description(t!("App.Settings.Terminal.shell_integration_description"))
                        .child(settings_switch(
                            "settings-toggle-shell-integration",
                            shell_integration_on,
                            Box::new({
                                let dialog = dialog.clone();
                                move |value, _, app| {
                                    let enable = *value;
                                    // Install / uninstall first so any
                                    // file-IO error can short-circuit
                                    // the persisted flag — we don't
                                    // want settings.json to claim
                                    // "enabled" while the rc files
                                    // were never written.
                                    // Platform-aware: routes to bash
                                    // on Unix and PowerShell profile
                                    // on Windows. See pier-core's
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
                                    let persist_flag = io_result.is_ok() && enable;
                                    apply_dialog_settings(&dialog, app, move |settings| {
                                        settings.terminal_shell_integration = persist_flag;
                                    });
                                }
                            }),
                        )),
                ),
        )
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let settings = t.settings.clone();
        let capturing = self.capturing;
        let capture_error = self.capture_error.clone();
        let focus = self.capture_focus.clone();

        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(ActionId::ALL.len());
        for action in ActionId::ALL {
            let stroke = resolved_keystroke(&settings, action);
            let is_capturing = capturing == Some(action);
            let is_overridden = settings.keybindings.contains_key(action.storage_id());
            let row_error = if is_capturing {
                capture_error.clone()
            } else {
                None
            };
            rows.push(
                shortcut_row_interactive(
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

        // Wrap rows in their own flex_col so we can control the row
        // gap independently of the section-level gap (SP_4). SP_0_5
        // is correct for rowless shortcut lines — bigger gaps leave
        // too much empty air between rows once the per-row border is
        // gone.
        section_shell(
            &t,
            t!("App.Settings.Sections.shortcuts_title"),
            t!("App.Settings.Shortcuts.subtitle"),
        )
        .child(settings_card().gap(SP_1).children(rows))
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    log::info!("dialog: opening settings dialog");
    let view = cx.new(SettingsDialog::new);
    window.open_dialog(cx, move |dialog, window, _app| {
        dialog
            .title(SharedString::from(t!("App.Settings.title").to_string()))
            .w(px(SETTINGS_DIALOG_W))
            .margin_top(settings_dialog_margin_top(window))
            .close_button(true)
            .overlay_closable(true)
            .keyboard(true)
            .child(view.clone())
    });
}

fn settings_dialog_margin_top(window: &Window) -> gpui::Pixels {
    let viewport_height = f32::from(window.viewport_size().height);
    px(((viewport_height - SETTINGS_DIALOG_OUTER_H) / 2.0).max(SETTINGS_DIALOG_MIN_TOP))
}

fn apply_dialog_settings(
    dialog: &WeakEntity<SettingsDialog>,
    app: &mut App,
    update: impl FnOnce(&mut AppSettings),
) {
    update_settings(app, update);
    let _ = dialog.update(app, |_, cx| cx.notify());
}

fn section_shell(
    t: &crate::theme::Theme,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
) -> gpui_component::scroll::Scrollable<gpui::Div> {
    let title: SharedString = title.into();
    let subtitle: SharedString = subtitle.into();
    div()
        .h_full()
        .overflow_y_scrollbar()
        .pl(SP_6)
        .pr(SP_8)
        .py(SP_5)
        .flex()
        .flex_col()
        .gap(SP_5)
        .child(
            div()
                .pb(SP_3)
                .border_b_1()
                .border_color(t.color.border_subtle)
                .child(
                    div()
                        .text_size(SIZE_H1)
                        .font_weight(WEIGHT_EMPHASIS)
                        .text_color(t.color.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .pt(SP_1)
                        .text_size(SIZE_CAPTION)
                        .text_color(t.color.text_secondary)
                        .child(subtitle),
                ),
        )
}

fn settings_card() -> Card {
    Card::new().padding(SP_4).gap(SP_4)
}

fn choice_wrap(children: Vec<gpui::AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(SP_2)
        .children(children)
}

fn choice_chip(
    t: &crate::theme::Theme,
    id_suffix: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let id_suffix: SharedString = id_suffix.into();
    let id: SharedString = format!("settings-chip-{}", id_suffix.as_ref()).into();
    div()
        .id(gpui::ElementId::Name(id))
        .min_w(px(96.0))
        .h(BUTTON_MD_H)
        .px(SP_3)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if selected {
            t.color.accent
        } else {
            t.color.border_subtle
        })
        .bg(if selected {
            t.color.accent_subtle
        } else {
            t.color.bg_canvas
        })
        .text_color(if selected {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(t.color.bg_hover)
                .border_color(t.color.border_default)
        })
        .on_click(on_click)
        .child(
            div()
                .text_size(SIZE_CAPTION)
                .font_weight(WEIGHT_MEDIUM)
                .child(label),
        )
}

fn stepper_control(
    t: &crate::theme::Theme,
    id_prefix: &'static str,
    value: SharedString,
    on_minus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    on_plus: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .px(SP_1)
        .py(SP_1)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_1_5)
        .rounded(RADIUS_SM)
        .bg(t.color.bg_canvas)
        .border_1()
        .border_color(t.color.border_subtle)
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
        .w(BUTTON_MD_H)
        .h(BUTTON_MD_H)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .bg(t.color.bg_surface)
        .text_color(t.color.text_secondary)
        .cursor_pointer()
        .hover(|style| style.bg(t.color.bg_hover).text_color(t.color.text_primary))
        .on_click(on_click)
        .child(
            div()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .child(label),
        )
}

fn shortcut_row_interactive(
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

    let mut row = div()
        .px(SP_2)
        .py(SP_2)
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_3)
        .rounded(RADIUS_SM)
        .hover(|s| s.bg(t.color.bg_hover))
        .when(is_capturing, |el| el.bg(t.color.accent_subtle))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(SIZE_BODY)
                .font_weight(WEIGHT_MEDIUM)
                .text_color(t.color.text_primary)
                .child(label),
        );

    if is_capturing {
        row = row.child(shortcut_capture_pad(t, capture_error, capture_focus, cx));
    } else {
        row = row.child(
            div()
                .flex_none()
                .px(SP_2)
                .py(SP_1_5)
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
                .child(keystroke),
        );
    }

    let change_label: SharedString = if is_capturing {
        t!("App.Common.cancel").into()
    } else {
        t!("App.Settings.Shortcuts.change").into()
    };
    let change_id = format!("shortcut-change-{}", action.storage_id());
    row = row.child(
        Button::secondary(gpui::ElementId::Name(change_id.into()), change_label)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if this.capturing == Some(action) {
                    this.cancel_capture(cx);
                } else {
                    this.start_capture(action, window, cx);
                }
            })),
    );

    if is_overridden {
        let reset_id = format!("shortcut-reset-{}", action.storage_id());
        row = row.child(
            Button::ghost(
                gpui::ElementId::Name(reset_id.into()),
                SharedString::from(t!("App.Settings.Shortcuts.reset").to_string()),
            )
            .size(ButtonSize::Sm)
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.reset_to_default(action, cx);
            })),
        );
    }

    row
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
        .py(SP_1_5)
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

fn terminal_theme_option(
    t: &crate::theme::Theme,
    palette: crate::theme::terminal::TerminalPalette,
    selected: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let id: SharedString = format!("settings-terminal-theme-{:?}", palette.preset).into();

    div()
        .id(gpui::ElementId::Name(id))
        .min_w(px(220.0))
        .px(SP_2)
        .py(SP_2)
        .flex()
        .flex_row()
        .items_start()
        .gap(SP_2)
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if selected {
            t.color.accent
        } else {
            t.color.border_subtle
        })
        .bg(if selected {
            t.color.accent_subtle
        } else {
            t.color.bg_canvas
        })
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(t.color.bg_hover)
                .border_color(t.color.border_default)
        })
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_1)
                .child(color_dot(palette.background_hex))
                .child(color_dot(palette.foreground_hex))
                .child(color_dot(palette.cursor_bg_hex))
                .child(color_dot(palette.ansi_16[2]))
                .child(color_dot(palette.ansi_16[1])),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(if selected {
                            t.color.accent
                        } else {
                            t.color.text_primary
                        })
                        .child(palette.name),
                )
                .child(
                    div()
                        .pt(SP_0_5)
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(SharedString::from(match palette.preset {
                            TerminalThemePreset::DefaultDark
                            | TerminalThemePreset::DefaultLight => {
                                t!("App.Settings.Terminal.Palettes.default").to_string()
                            }
                            TerminalThemePreset::SolarizedDark => {
                                t!("App.Settings.Terminal.Palettes.solarized_dark").to_string()
                            }
                            TerminalThemePreset::Dracula => {
                                t!("App.Settings.Terminal.Palettes.dracula").to_string()
                            }
                            TerminalThemePreset::Monokai => {
                                t!("App.Settings.Terminal.Palettes.monokai").to_string()
                            }
                            TerminalThemePreset::Nord => {
                                t!("App.Settings.Terminal.Palettes.nord").to_string()
                            }
                        })),
                ),
        )
}

fn color_dot(hex: u32) -> impl IntoElement {
    div()
        .w(px(10.0))
        .h(px(10.0))
        .rounded(px(999.0))
        .bg(terminal_hex_color(hex))
}

fn settings_switch(
    id: impl Into<SharedString>,
    checked: bool,
    on_toggle: Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    Switch::new(gpui::ElementId::Name(id.into()))
        .checked(checked)
        .on_click(on_toggle)
}

fn settings_sidebar_item(
    t: &crate::theme::Theme,
    id: &'static str,
    label: SharedString,
    active: bool,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .px(SP_3)
        .py(SP_2)
        .rounded(RADIUS_MD)
        .bg(if active {
            t.color.accent_subtle
        } else {
            t.color.bg_panel
        })
        .border_1()
        .border_color(if active {
            t.color.accent_muted
        } else {
            t.color.bg_panel
        })
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(t.color.bg_hover)
                .border_color(t.color.border_default)
        })
        .on_click(on_click)
        .text_size(SIZE_BODY)
        .font_weight(WEIGHT_MEDIUM)
        .text_color(if active {
            t.color.accent
        } else {
            t.color.text_secondary
        })
        .child(label)
}

fn localized_locale_label(locale: &str) -> String {
    match locale {
        LOCALE_ENGLISH => t!("App.Settings.General.language_english").to_string(),
        LOCALE_ZH_CN => t!("App.Settings.General.language_simplified_chinese").to_string(),
        _ => t!("App.Settings.General.language_system").to_string(),
    }
}
