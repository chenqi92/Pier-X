use gpui::SharedString;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Welcome,
    Dashboard,
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
    pub fn label(self) -> SharedString {
        match self {
            Route::Welcome => "Welcome".into(),
            Route::Dashboard => "Dashboard".into(),
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
            Route::Terminal => "terminal",
            Route::Git => "git",
            Route::Ssh => "ssh",
            Route::Database(DbKind::Mysql) => "db-mysql",
            Route::Database(DbKind::Postgres) => "db-postgres",
            Route::Database(DbKind::Redis) => "db-redis",
            Route::Database(DbKind::Sqlite) => "db-sqlite",
        }
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

    pub const ALL: [DbKind; 4] = [DbKind::Mysql, DbKind::Postgres, DbKind::Redis, DbKind::Sqlite];
}
