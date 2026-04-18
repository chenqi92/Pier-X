use gpui::{div, prelude::*, IntoElement, SharedString, Window};
use rust_i18n::t;

use crate::app::route::DbKind;
use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    spacing::{SP_3, SP_4},
    theme,
    typography::{SIZE_MONO_SMALL, SIZE_SMALL},
};

#[derive(IntoElement)]
pub struct DatabaseView {
    kind: DbKind,
}

impl DatabaseView {
    pub fn new(kind: DbKind) -> Self {
        Self { kind }
    }
}

impl RenderOnce for DatabaseView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let info = DbProfile::for_kind(self.kind);

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(SP_4)
            .p(SP_4)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(SP_3)
                    .child(text::h2(self.kind.label()))
                    .child(StatusPill::new(t!("App.Database.not_connected"), StatusKind::Warning)),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new(t!("App.Database.service")))
                    .child(text::body(info.description.clone())),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new(t!("App.Database.connection_template")))
                    .children(info.fields.iter().map(|(label, value)| {
                        config_row(
                            t,
                            SharedString::from(label.to_string()),
                            SharedString::from(value.to_string()),
                        )
                    })),
            )
            .child(
                Card::new()
                    .child(SectionLabel::new(t!("App.Database.backend")))
                    .child(
                        div()
                            .text_size(SIZE_MONO_SMALL)
                            .font_family(t.font_mono.clone())
                            .text_color(t.color.text_tertiary)
                            .child(SharedString::from(info.backend_module.to_string())),
                    )
                    .child(
                        text::body(t!("App.Database.follow_up")).secondary(),
                    ),
            )
    }
}

fn config_row(
    t: &crate::theme::Theme,
    label: SharedString,
    value: SharedString,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .gap(SP_3)
        .child(
            div()
                .w(gpui::px(120.0))
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(label),
        )
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_secondary)
                .child(value),
        )
}

struct DbProfile {
    description: SharedString,
    backend_module: &'static str,
    fields: Vec<(&'static str, String)>,
}

impl DbProfile {
    fn for_kind(kind: DbKind) -> Self {
        match kind {
            DbKind::Mysql => Self {
                description: t!("App.Database.Profiles.mysql").into(),
                backend_module: "pier_core::services::mysql::MysqlConfig",
                fields: vec![
                    ("host", "127.0.0.1".into()),
                    ("port", "3306".into()),
                    ("user", "root".into()),
                    ("password", "<empty>".into()),
                    ("database", "<default>".into()),
                ],
            },
            DbKind::Postgres => Self {
                description: t!("App.Database.Profiles.postgres").into(),
                backend_module: "pier_core::services::postgres::PostgresConfig",
                fields: vec![
                    ("host", "127.0.0.1".into()),
                    ("port", "5432".into()),
                    ("user", "postgres".into()),
                    ("password", "<empty>".into()),
                    ("database", "<user-default>".into()),
                ],
            },
            DbKind::Redis => Self {
                description: t!("App.Database.Profiles.redis").into(),
                backend_module: "pier_core::services::redis::RedisConfig",
                fields: vec![
                    ("host", "127.0.0.1".into()),
                    ("port", "6379".into()),
                    ("db", "0".into()),
                ],
            },
            DbKind::Sqlite => Self {
                description: t!("App.Database.Profiles.sqlite").into(),
                backend_module: "pier_core::services::sqlite::SqliteClient::open",
                fields: vec![("path", "/path/to/db.sqlite".into())],
            },
        }
    }
}
