//! Layout primitives for the 3-pane shell that mirrors Pier's MainView.swift.
//!
//! Reference: `Pier/PierApp/Sources/Views/MainWindow/MainView.swift`
//!
//! ```text
//! ┌──────────┬────────────────────────┬──────────────┐
//! │  Left    │       Terminal         │  Right Panel │
//! │  Files / │       (or Welcome      │  (10 modes)  │
//! │  Servers │       cover state)     │              │
//! └──────────┴────────────────────────┴──────────────┘
//! ```

#![allow(dead_code)]

use gpui::{px, Pixels, SharedString};

use crate::app::route::DbKind;

// ─── Layout sizing (mirrors Pier's HSplitView min/ideal/max) ─────────────

pub const TOOLBAR_HEIGHT: Pixels = px(36.0);

pub const LEFT_PANEL_MIN_W: Pixels = px(180.0);
pub const LEFT_PANEL_DEFAULT_W: Pixels = px(260.0);
pub const LEFT_PANEL_MAX_W: Pixels = px(400.0);

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
            LeftTab::Files => "Files".into(),
            LeftTab::Servers => "Servers".into(),
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

// ─── Right panel: 10 modes ──────────────────────────────────────────────

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
            RightMode::Markdown => "Markdown".into(),
            RightMode::Monitor => "Monitor".into(),
            RightMode::Sftp => "SFTP".into(),
            RightMode::Docker => "Docker".into(),
            RightMode::Git => "Git".into(),
            RightMode::Mysql => "MySQL".into(),
            RightMode::Postgres => "PostgreSQL".into(),
            RightMode::Redis => "Redis".into(),
            RightMode::Sqlite => "SQLite".into(),
            RightMode::Logs => "Logs".into(),
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
            // Pier defaults to Markdown / Git as local-context tools. Pier-X
            // adds SQLite (no remote service required, just a file path) to
            // the local set so the cover-state demo doesn't look bare.
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

    /// Asset path for the SVG icon. Loaded via `gpui::Application::with_assets`
    /// → `assets::AppAssets`. None means the mode renders a built-in
    /// `gpui_component::IconName` instead (resolved in `mode_icon`).
    pub fn icon_asset(self) -> Option<&'static str> {
        match self {
            RightMode::Sftp | RightMode::Sqlite => Some("icons/server.svg"),
            RightMode::Docker => Some("icons/database.svg"),
            RightMode::Git => Some("icons/git-branch.svg"),
            RightMode::Mysql | RightMode::Postgres | RightMode::Redis => {
                Some("icons/database.svg")
            }
            RightMode::Logs => Some("icons/ellipsis.svg"),
            // Markdown + Monitor use built-in lucide icons via `mode_icon()`.
            RightMode::Markdown | RightMode::Monitor => None,
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

    /// Display order in the right panel's vertical icon bar.
    /// Mirrors Pier's `RightPanelMode` declaration order, with Pier-X's
    /// extra database modes (Postgres, SQLite) inserted near MySQL/Redis.
    pub const ALL: [RightMode; 10] = [
        RightMode::Markdown,
        RightMode::Monitor,
        RightMode::Sftp,
        RightMode::Docker,
        RightMode::Git,
        RightMode::Mysql,
        RightMode::Postgres,
        RightMode::Redis,
        RightMode::Sqlite,
        RightMode::Logs,
    ];
}
