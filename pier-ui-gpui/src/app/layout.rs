//! Layout primitives for the 3-pane shell that mirrors Pier's MainView.swift.
//!
//! Reference: `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`
//!
//! ```text
//! ┌──────────┬────────────────────────┬──────────────┐
//! │  Left    │       Terminal         │  Right Panel │
//! │  Files / │       (or Welcome      │  (Pier modes)│
//! │  Servers │       cover state)     │              │
//! └──────────┴────────────────────────┴──────────────┘
//! ```

#![allow(dead_code)]

use gpui::{px, Pixels, SharedString};
use rust_i18n::t;

use crate::app::route::DbKind;

// ─── Layout sizing (mirrors Pier's HSplitView min/ideal/max) ─────────────

pub const TOOLBAR_HEIGHT: Pixels = px(36.0);

pub const LEFT_PANEL_MIN_W: Pixels = px(180.0);
pub const LEFT_PANEL_DEFAULT_W: Pixels = px(260.0);
pub const LEFT_PANEL_MAX_W: Pixels = px(400.0);

pub const CENTER_PANEL_MIN_W: Pixels = px(360.0);

pub const RIGHT_PANEL_MIN_W: Pixels = px(320.0);
pub const RIGHT_PANEL_DEFAULT_W: Pixels = px(360.0);
pub const RIGHT_PANEL_MAX_W: Pixels = px(600.0);

pub const RIGHT_ICON_BAR_W: Pixels = px(36.0);

// ─── Left panel: Files / Servers ────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LeftTab {
    Files,
    Servers,
}

impl LeftTab {
    pub fn label(self) -> SharedString {
        match self {
            LeftTab::Files => t!("App.Common.files").into(),
            LeftTab::Servers => t!("App.Common.servers").into(),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            LeftTab::Files => "left-files",
            LeftTab::Servers => "left-servers",
        }
    }

    pub const ALL: [LeftTab; 2] = [LeftTab::Files, LeftTab::Servers];
}

// ─── Right panel: Pier-aligned shell modes ──────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RightMode {
    Markdown,
    Monitor,
    Sftp,
    Docker,
    Git,
    Mysql,
    Postgres,
    Redis,
    Sqlite,
    Logs,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RightContext {
    /// Always available, no SSH session required.
    Local,
    /// Requires an active SSH session and (eventually) a detected service.
    Remote,
}

impl RightMode {
    pub fn label(self) -> SharedString {
        match self {
            RightMode::Markdown => t!("App.RightPanel.Modes.markdown").into(),
            RightMode::Monitor => t!("App.RightPanel.Modes.monitor").into(),
            RightMode::Sftp => t!("App.RightPanel.Modes.sftp").into(),
            RightMode::Docker => t!("App.RightPanel.Modes.docker").into(),
            RightMode::Git => t!("App.RightPanel.Modes.git").into(),
            RightMode::Mysql => "MySQL".into(),
            RightMode::Postgres => "PostgreSQL".into(),
            RightMode::Redis => "Redis".into(),
            RightMode::Sqlite => "SQLite".into(),
            RightMode::Logs => t!("App.RightPanel.Modes.logs").into(),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            RightMode::Markdown => "right-markdown",
            RightMode::Monitor => "right-monitor",
            RightMode::Sftp => "right-sftp",
            RightMode::Docker => "right-docker",
            RightMode::Git => "right-git",
            RightMode::Mysql => "right-mysql",
            RightMode::Postgres => "right-postgres",
            RightMode::Redis => "right-redis",
            RightMode::Sqlite => "right-sqlite",
            RightMode::Logs => "right-logs",
        }
    }

    pub fn context(self) -> RightContext {
        match self {
            // Keep the standard sidebar aligned with Pier: local tooling is
            // Markdown + Git. SQLite stays implemented as an internal panel,
            // but it is not exposed in the default right-sidebar flow.
            RightMode::Markdown | RightMode::Git | RightMode::Sqlite => RightContext::Local,
            RightMode::Monitor
            | RightMode::Sftp
            | RightMode::Docker
            | RightMode::Mysql
            | RightMode::Postgres
            | RightMode::Redis
            | RightMode::Logs => RightContext::Remote,
        }
    }

    pub fn required_service_name(self) -> Option<&'static str> {
        match self {
            RightMode::Docker => Some("docker"),
            RightMode::Mysql => Some("mysql"),
            RightMode::Postgres => Some("postgresql"),
            RightMode::Redis => Some("redis"),
            _ => None,
        }
    }

    pub fn from_service_name(name: &str) -> Option<Self> {
        match name {
            "docker" => Some(RightMode::Docker),
            "mysql" => Some(RightMode::Mysql),
            "postgresql" => Some(RightMode::Postgres),
            "redis" => Some(RightMode::Redis),
            _ => None,
        }
    }

    /// Asset path for the SVG icon. Loaded via `gpui::Application::with_assets`.
    pub fn icon_asset(self) -> Option<&'static str> {
        match self {
            RightMode::Markdown => Some("icons/file.svg"),
            RightMode::Monitor => Some("icons/layout-dashboard.svg"),
            RightMode::Sftp => Some("icons/folder.svg"),
            RightMode::Docker => Some("icons/server.svg"),
            RightMode::Git => Some("icons/git-branch.svg"),
            RightMode::Mysql | RightMode::Postgres | RightMode::Redis | RightMode::Sqlite => {
                Some("icons/database.svg")
            }
            RightMode::Logs => Some("icons/square-terminal.svg"),
        }
    }

    /// Map MySQL/PG/Redis/SQLite to the existing [`DbKind`] so the
    /// already-built [`crate::views::database::DatabaseView`] can be reused
    /// inside the right panel without a parallel switch.
    pub fn db_kind(self) -> Option<DbKind> {
        match self {
            RightMode::Mysql => Some(DbKind::Mysql),
            RightMode::Postgres => Some(DbKind::Postgres),
            RightMode::Redis => Some(DbKind::Redis),
            RightMode::Sqlite => Some(DbKind::Sqlite),
            _ => None,
        }
    }

    /// Display order in the right panel's vertical icon bar and the default
    /// availability set for live sessions. Keep this aligned with Pier's
    /// standard shell instead of exposing every internal panel by default.
    pub const ALL: [RightMode; 9] = [
        RightMode::Markdown,
        RightMode::Monitor,
        RightMode::Sftp,
        RightMode::Docker,
        RightMode::Git,
        RightMode::Mysql,
        RightMode::Postgres,
        RightMode::Redis,
        RightMode::Logs,
    ];

    pub const LOCAL_ONLY: [RightMode; 2] = [RightMode::Markdown, RightMode::Git];
}

#[cfg(test)]
mod tests {
    use super::RightMode;

    #[test]
    fn sqlite_is_not_exposed_in_default_sidebar_sets() {
        assert!(!RightMode::ALL.contains(&RightMode::Sqlite));
        assert!(!RightMode::LOCAL_ONLY.contains(&RightMode::Sqlite));
    }
}
