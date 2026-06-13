//! InfluxDB client backend for the unified database panel.
//!
//! Time-series store reached over its HTTP API rather than a wire
//! protocol — so unlike the SQL clients this is a thin `ureq` wrapper
//! around the `/query` endpoint speaking InfluxQL. Works against
//! InfluxDB 1.x and the 1.x-compatible `/query` endpoint exposed by
//! 2.x. Connects through the SSH tunnel like the other clients, so the
//! HTTP traffic is plaintext on `127.0.0.1:<tunnelPort>`.
//!
//! Auth: an API token (2.x: `Authorization: Token …`) when provided,
//! else `u`/`p` form params (1.x basic credentials). Passwordless dev
//! instances need neither.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Same cap as the SQL clients — 10k rows per result.
pub const MAX_ROWS: usize = 10_000;

/// Errors surfaced by the InfluxDB client.
#[derive(Debug, thiserror::Error)]
pub enum InfluxError {
    /// Transport / non-2xx HTTP failure.
    #[error("influx: {0}")]
    Http(String),
    /// InfluxQL error returned in the JSON body.
    #[error("influx query: {0}")]
    Query(String),
    /// Caller supplied invalid config.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Result alias for InfluxDB ops.
pub type Result<T, E = InfluxError> = std::result::Result<T, E>;

/// Connection config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfluxConfig {
    /// Hostname or IP (the tunnel local endpoint in practice).
    pub host: String,
    /// HTTP port (default 8086).
    pub port: u16,
    /// Default database / bucket. Empty = none (some queries need it).
    pub database: Option<String>,
    /// 1.x username (optional).
    pub user: String,
    /// 1.x password (optional).
    pub password: String,
    /// 2.x API token (optional). Takes precedence over user/password.
    pub token: String,
}

/// One InfluxQL result series, flattened to the shared grid shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfluxQueryResult {
    /// Column names from the first returned series.
    pub columns: Vec<String>,
    /// Rows, capped at [`MAX_ROWS`].
    pub rows: Vec<Vec<String>>,
    /// True if more rows existed than we returned.
    pub truncated: bool,
    /// Wall-clock execution time.
    pub elapsed_ms: u64,
}

fn base_url(cfg: &InfluxConfig) -> String {
    format!("http://{}:{}/query", cfg.host, cfg.port)
}

/// Run an InfluxQL statement and flatten the first series into the grid.
pub fn query(cfg: &InfluxConfig, influxql: &str) -> Result<InfluxQueryResult> {
    if cfg.host.is_empty() {
        return Err(InfluxError::InvalidConfig("empty host".into()));
    }
    if cfg.port == 0 {
        return Err(InfluxError::InvalidConfig("port must be > 0".into()));
    }

    let start = Instant::now();
    let url = base_url(cfg);

    // POST /query handles both read and schema-changing statements.
    let mut form: Vec<(&str, &str)> = vec![("q", influxql)];
    let db = cfg.database.clone().unwrap_or_default();
    if !db.is_empty() {
        form.push(("db", &db));
    }
    if cfg.token.is_empty() && !cfg.user.is_empty() {
        form.push(("u", &cfg.user));
        form.push(("p", &cfg.password));
    }

    let mut req = ureq::post(&url);
    if !cfg.token.is_empty() {
        req = req.set("Authorization", &format!("Token {}", cfg.token));
    }

    let resp = req.send_form(&form).map_err(|e| match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            InfluxError::Http(format!("HTTP {code}: {}", first_line(&body)))
        }
        ureq::Error::Transport(t) => InfluxError::Http(t.to_string()),
    })?;

    let body: serde_json::Value = resp
        .into_json()
        .map_err(|e| InfluxError::Http(e.to_string()))?;

    // { "results": [ { "error": "...", "series": [ { "columns": [...],
    //   "values": [[...], ...] } ] } ] }
    let result0 = body
        .get("results")
        .and_then(|r| r.get(0))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if let Some(err) = result0.get("error").and_then(|v| v.as_str()) {
        return Err(InfluxError::Query(err.to_string()));
    }
    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        return Err(InfluxError::Query(err.to_string()));
    }

    let series0 = result0
        .get("series")
        .and_then(|s| s.get(0))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let columns: Vec<String> = series0
        .get("columns")
        .and_then(|c| c.as_array())
        .map(|a| a.iter().map(json_cell).collect())
        .unwrap_or_default();

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    if let Some(values) = series0.get("values").and_then(|v| v.as_array()) {
        for row in values {
            if rows.len() >= MAX_ROWS {
                truncated = true;
                break;
            }
            if let Some(cells) = row.as_array() {
                rows.push(cells.iter().map(json_cell).collect());
            }
        }
    }

    Ok(InfluxQueryResult {
        columns,
        rows,
        truncated,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

/// `SHOW DATABASES` → flat list of database names.
pub fn list_databases(cfg: &InfluxConfig) -> Result<Vec<String>> {
    let r = query(cfg, "SHOW DATABASES")?;
    Ok(r.rows.into_iter().filter_map(|mut row| row.pop()).collect())
}

/// `SHOW MEASUREMENTS` on the active database → measurement names.
pub fn list_measurements(cfg: &InfluxConfig) -> Result<Vec<String>> {
    let r = query(cfg, "SHOW MEASUREMENTS")?;
    Ok(r.rows.into_iter().filter_map(|mut row| row.pop()).collect())
}

/// Stringify a JSON scalar for the grid. `null` → empty string.
fn json_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}
