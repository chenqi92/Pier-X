#![allow(dead_code)]

use gpui::SharedString;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Welcome,
    Dashboard,
    Inspector,
    Terminal,
    Git,
    Ssh,
    Database(DbKind),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DbKind {
    Mysql,
    Postgres,
    Redis,
    Sqlite,
}

impl Route {
    pub const fn mysql() -> Self {
        Self::Database(DbKind::Mysql)
    }

    pub const fn postgres() -> Self {
        Self::Database(DbKind::Postgres)
    }

    pub const fn redis() -> Self {
        Self::Database(DbKind::Redis)
    }

    pub const fn sqlite() -> Self {
        Self::Database(DbKind::Sqlite)
    }

    pub fn label(self) -> SharedString {
        match self {
            Route::Welcome => "Welcome".into(),
            Route::Dashboard => "Dashboard".into(),
            Route::Inspector => "Inspector".into(),
            Route::Terminal => "Terminal".into(),
            Route::Git => "Git".into(),
            Route::Ssh => "SSH".into(),
            Route::Database(k) => k.label(),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Route::Welcome => "welcome",
            Route::Dashboard => "dashboard",
            Route::Inspector => "inspector",
            Route::Terminal => "terminal",
            Route::Git => "git",
            Route::Ssh => "ssh",
            Route::Database(DbKind::Mysql) => "db-mysql",
            Route::Database(DbKind::Postgres) => "db-postgres",
            Route::Database(DbKind::Redis) => "db-redis",
            Route::Database(DbKind::Sqlite) => "db-sqlite",
        }
    }

    pub fn panel_name(self) -> &'static str {
        match self {
            Route::Welcome => "welcome-panel",
            Route::Dashboard => "dashboard-panel",
            Route::Inspector => "inspector-panel",
            Route::Terminal => "terminal-panel",
            Route::Git => "git-panel",
            Route::Ssh => "ssh-panel",
            Route::Database(DbKind::Mysql) => "db-mysql-panel",
            Route::Database(DbKind::Postgres) => "db-postgres-panel",
            Route::Database(DbKind::Redis) => "db-redis-panel",
            Route::Database(DbKind::Sqlite) => "db-sqlite-panel",
        }
    }

    pub fn is_primary(self) -> bool {
        matches!(
            self,
            Route::Dashboard | Route::Terminal | Route::Git | Route::Ssh
        )
    }
}

impl DbKind {
    pub fn label(self) -> SharedString {
        match self {
            DbKind::Mysql => "MySQL".into(),
            DbKind::Postgres => "PostgreSQL".into(),
            DbKind::Redis => "Redis".into(),
            DbKind::Sqlite => "SQLite".into(),
        }
    }

    pub const ALL: [DbKind; 4] = [
        DbKind::Mysql,
        DbKind::Postgres,
        DbKind::Redis,
        DbKind::Sqlite,
    ];

    pub const fn index(self) -> usize {
        match self {
            DbKind::Mysql => 0,
            DbKind::Postgres => 1,
            DbKind::Redis => 2,
            DbKind::Sqlite => 3,
        }
    }
}
