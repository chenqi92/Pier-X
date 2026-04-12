//! C ABI for the PostgreSQL panel (M7a).
//!
//! Mirrors [`super::mysql`] byte-for-byte in API shape:
//! one opaque handle + JSON execute / list_databases /
//! list_tables / list_columns + free. The C++ and QML
//! layers can reuse the same result-model code for both
//! MySQL and PostgreSQL because the JSON schemas are
//! identical.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::services::postgres::{PostgresClient, PostgresConfig, QueryResult};

/// Opaque PostgreSQL handle.
pub struct PierPostgres {
    client: PostgresClient,
}

/// Open a PostgreSQL connection. Performs handshake + auth +
/// `SELECT 1` probe synchronously. Returns NULL on any
/// failure.
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8.
/// `password` and `database` may be NULL or empty.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    password: *const c_char,
    database: *const c_char,
) -> *mut PierPostgres {
    if host.is_null() || user.is_null() || port == 0 {
        return ptr::null_mut();
    }
    let host_str = match unsafe { CStr::from_ptr(host) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let user_str = match unsafe { CStr::from_ptr(user) }.to_str() {
        Ok(s) if !s.is_empty() => s.to_string(),
        _ => return ptr::null_mut(),
    };
    let password_str = if password.is_null() {
        String::new()
    } else {
        match unsafe { CStr::from_ptr(password) }.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        }
    };
    let database_opt = if database.is_null() {
        None
    } else {
        match unsafe { CStr::from_ptr(database) }.to_str() {
            Ok("") => None,
            Ok(s) => Some(s.to_string()),
            Err(_) => return ptr::null_mut(),
        }
    };

    let config = PostgresConfig {
        host: host_str,
        port,
        user: user_str,
        password: password_str,
        database: database_opt,
    };
    match PostgresClient::connect_blocking(config) {
        Ok(client) => Box::into_raw(Box::new(PierPostgres { client })),
        Err(e) => {
            log::warn!("pier_postgres_open failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Execute a single SQL statement. Returns a heap JSON
/// string of the same [`QueryResult`] shape as MySQL's
/// `pier_mysql_execute`. On server-side errors the JSON
/// carries an `"error"` field.
///
/// # Safety
///
/// `h` must be a live handle. `sql` must be a valid
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_execute(
    h: *mut PierPostgres,
    sql: *const c_char,
) -> *mut c_char {
    if h.is_null() || sql.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    let sql_str = match unsafe { CStr::from_ptr(sql) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let result: QueryResult = match handle.client.execute_blocking(sql_str) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("pier_postgres_execute failed: {e}");
            return error_as_json(&e.to_string());
        }
    };
    into_json_cstring(&result)
}

/// List databases. Returns a JSON array of strings.
///
/// # Safety
///
/// `h` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_list_databases(h: *mut PierPostgres) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    match handle.client.list_databases_blocking() {
        Ok(dbs) => into_json_cstring(&dbs),
        Err(e) => {
            log::warn!("pier_postgres_list_databases failed: {e}");
            ptr::null_mut()
        }
    }
}

/// List tables in `schema` (default `"public"`). Returns a
/// JSON array of strings.
///
/// # Safety
///
/// `h` must be a live handle. `schema` must be a valid
/// NUL-terminated C string (or NULL for `"public"`).
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_list_tables(
    h: *mut PierPostgres,
    schema: *const c_char,
) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    let schema_str = if schema.is_null() {
        "public".to_string()
    } else {
        match unsafe { CStr::from_ptr(schema) }.to_str() {
            Ok("") => "public".to_string(),
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        }
    };
    match handle.client.list_tables_blocking(&schema_str) {
        Ok(tables) => into_json_cstring(&tables),
        Err(e) => {
            log::warn!("pier_postgres_list_tables failed: {e}");
            ptr::null_mut()
        }
    }
}

/// List columns for `schema.table`. Returns a JSON array of
/// `ColumnInfo` objects (same shape as MySQL's
/// `pier_mysql_list_columns`).
///
/// # Safety
///
/// `h` must be a live handle. `schema` and `table` must be
/// valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_list_columns(
    h: *mut PierPostgres,
    schema: *const c_char,
    table: *const c_char,
) -> *mut c_char {
    if h.is_null() || table.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    let schema_str = if schema.is_null() {
        "public".to_string()
    } else {
        match unsafe { CStr::from_ptr(schema) }.to_str() {
            Ok("") => "public".to_string(),
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        }
    };
    let table_str = match unsafe { CStr::from_ptr(table) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match handle.client.list_columns_blocking(&schema_str, table_str) {
        Ok(cols) => into_json_cstring(&cols),
        Err(e) => {
            log::warn!("pier_postgres_list_columns failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Free a JSON string returned by any `pier_postgres_*`.
/// Safe on NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(s) });
}

/// Free a PostgreSQL handle. Safe on NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_postgres_free(h: *mut PierPostgres) {
    if h.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(h) });
}

// ── Internal helpers ────────────────────────────────────

fn into_json_cstring<T: serde::Serialize>(value: &T) -> *mut c_char {
    let json = match serde_json::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_postgres: json serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

fn error_as_json(message: &str) -> *mut c_char {
    #[derive(serde::Serialize)]
    struct ErrorResult<'a> {
        columns: &'a [String],
        rows: &'a [Vec<Option<String>>],
        truncated: bool,
        affected_rows: u64,
        last_insert_id: Option<u64>,
        elapsed_ms: u64,
        error: &'a str,
    }
    let payload = ErrorResult {
        columns: &[],
        rows: &[],
        truncated: false,
        affected_rows: 0,
        last_insert_id: None,
        elapsed_ms: 0,
        error: message,
    };
    into_json_cstring(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inputs_are_safe() {
        unsafe {
            assert!(
                pier_postgres_open(ptr::null(), 5432, ptr::null(), ptr::null(), ptr::null())
                    .is_null()
            );
            assert!(pier_postgres_execute(ptr::null_mut(), ptr::null()).is_null());
            assert!(pier_postgres_list_databases(ptr::null_mut()).is_null());
            assert!(pier_postgres_list_tables(ptr::null_mut(), ptr::null()).is_null());
            assert!(
                pier_postgres_list_columns(ptr::null_mut(), ptr::null(), ptr::null()).is_null()
            );
            pier_postgres_free_string(ptr::null_mut());
            pier_postgres_free(ptr::null_mut());
        }
    }

    #[test]
    fn zero_port_rejected() {
        let host = CString::new("127.0.0.1").unwrap();
        let user = CString::new("root").unwrap();
        let h = unsafe {
            pier_postgres_open(host.as_ptr(), 0, user.as_ptr(), ptr::null(), ptr::null())
        };
        assert!(h.is_null());
    }

    #[test]
    fn error_as_json_carries_message() {
        let raw = error_as_json("syntax error at or near \"FOO\"");
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap();
        assert!(text.contains("\"error\":"));
        assert!(text.contains("syntax error"));
        unsafe { pier_postgres_free_string(raw) };
    }
}
