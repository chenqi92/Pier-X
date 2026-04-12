//! C ABI for the MySQL panel (M5d).
//!
//! ## Handle model
//!
//! One opaque `*mut PierMysql` per panel. The handle wraps a
//! single [`crate::services::mysql::MysqlClient`] whose
//! underlying `mysql_async::Pool` holds one live connection
//! to the server. Dropping the handle disconnects.
//!
//! ## JSON-shaped results
//!
//! Every read operation returns a heap JSON string that the
//! caller must release with [`pier_mysql_free_string`].
//! Execute results carry the full [`QueryResult`] shape —
//! columns, rows (with `null` for NULL cells), affected
//! count, elapsed time. List operations return a JSON array
//! of names.
//!
//! ## Connection
//!
//! [`pier_mysql_open`] runs the TCP connect + handshake +
//! auth + `SELECT 1` probe synchronously. Like every other
//! session-based FFI in pier-core, the C++ wrapper runs it
//! on a worker thread and posts results back via Qt's
//! `QMetaObject::invokeMethod`.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::services::mysql::{MysqlClient, MysqlConfig, QueryResult};

/// Opaque MySQL handle.
pub struct PierMysql {
    client: MysqlClient,
}

/// Open a MySQL connection to `host:port` authenticating as
/// `user` / `password`. `database` may be NULL for "no
/// default database". Performs the full handshake + auth +
/// `SELECT 1` probe synchronously and returns NULL on any
/// failure.
///
/// # Safety
///
/// `host` and `user` must be valid NUL-terminated UTF-8.
/// `password` may be null (empty password) or a valid
/// NUL-terminated C string. `database` follows the same
/// rule.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_open(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    password: *const c_char,
    database: *const c_char,
) -> *mut PierMysql {
    if host.is_null() || user.is_null() || port == 0 {
        return ptr::null_mut();
    }
    // SAFETY: caller contract.
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

    let config = MysqlConfig {
        host: host_str,
        port,
        user: user_str,
        password: password_str,
        database: database_opt,
    };
    match MysqlClient::connect_blocking(config) {
        Ok(client) => Box::into_raw(Box::new(PierMysql { client })),
        Err(e) => {
            log::warn!("pier_mysql_open failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Open a MySQL connection using a password stored in the OS
/// keyring under `credential_id`.
///
/// # Safety
///
/// `host`, `user`, and `credential_id` must be valid
/// NUL-terminated UTF-8. `database` may be NULL or empty.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_open_with_credential(
    host: *const c_char,
    port: u16,
    user: *const c_char,
    credential_id: *const c_char,
    database: *const c_char,
) -> *mut PierMysql {
    if host.is_null() || user.is_null() || credential_id.is_null() || port == 0 {
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
    let credential_id_str = match unsafe { CStr::from_ptr(credential_id) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
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

    let password = match crate::credentials::get(credential_id_str) {
        Ok(Some(v)) => v,
        Ok(None) => {
            log::warn!("pier_mysql_open_with_credential: missing keychain entry {credential_id_str}");
            return ptr::null_mut();
        }
        Err(e) => {
            log::warn!("pier_mysql_open_with_credential failed to read credential {credential_id_str}: {e}");
            return ptr::null_mut();
        }
    };

    let config = MysqlConfig {
        host: host_str,
        port,
        user: user_str,
        password,
        database: database_opt,
    };
    match MysqlClient::connect_blocking(config) {
        Ok(client) => Box::into_raw(Box::new(PierMysql { client })),
        Err(e) => {
            log::warn!("pier_mysql_open_with_credential failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Execute a single SQL statement. Returns a heap JSON string
/// of shape [`QueryResult`]:
///
/// ```json
/// { "columns": ["id", "name"],
///   "rows": [[1, "alice"], [2, null]],
///   "truncated": false,
///   "affected_rows": 0,
///   "last_insert_id": null,
///   "elapsed_ms": 12 }
/// ```
///
/// Returns NULL on failure (connection error, syntax error,
/// unsupported statement). Release with
/// [`pier_mysql_free_string`].
///
/// # Safety
///
/// `h` must be a live handle produced by
/// [`pier_mysql_open`]. `sql` must be a valid NUL-terminated
/// C string.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_execute(
    h: *mut PierMysql,
    sql: *const c_char,
) -> *mut c_char {
    if h.is_null() || sql.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle + NUL-terminated string.
    let handle = unsafe { &*h };
    let sql_str = match unsafe { CStr::from_ptr(sql) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let result: QueryResult = match handle.client.execute_blocking(sql_str) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("pier_mysql_execute failed: {e}");
            // Emit a partial QueryResult whose `error` field
            // carries the message. The schema matches
            // QueryResult with an extra wrapper so the UI can
            // distinguish "no rows" from "error".
            return error_as_json(&e.to_string());
        }
    };
    into_json_cstring(&result)
}

/// `SHOW DATABASES`. Returns a heap JSON array of strings
/// (internal schemas filtered — see
/// [`crate::services::mysql::MysqlClient::list_databases`]).
///
/// # Safety
///
/// `h` must be a live handle.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_list_databases(h: *mut PierMysql) -> *mut c_char {
    if h.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle.
    let handle = unsafe { &*h };
    match handle.client.list_databases_blocking() {
        Ok(dbs) => into_json_cstring(&dbs),
        Err(e) => {
            log::warn!("pier_mysql_list_databases failed: {e}");
            ptr::null_mut()
        }
    }
}

/// `SHOW TABLES FROM <database>`. Returns a heap JSON array
/// of table names. `database` is rejected if it contains any
/// character outside the strict identifier allowlist in
/// [`crate::services::mysql::is_safe_ident`].
///
/// # Safety
///
/// `h` must be a live handle. `database` must be a valid
/// NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_list_tables(
    h: *mut PierMysql,
    database: *const c_char,
) -> *mut c_char {
    if h.is_null() || database.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: live handle + NUL-terminated string.
    let handle = unsafe { &*h };
    let db = match unsafe { CStr::from_ptr(database) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match handle.client.list_tables_blocking(db) {
        Ok(tables) => into_json_cstring(&tables),
        Err(e) => {
            log::warn!("pier_mysql_list_tables failed: {e}");
            ptr::null_mut()
        }
    }
}

/// `SHOW COLUMNS FROM <database>.<table>`. Returns a heap
/// JSON array of column metadata objects.
///
/// # Safety
///
/// `h` must be a live handle. `database` and `table` must be
/// valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_list_columns(
    h: *mut PierMysql,
    database: *const c_char,
    table: *const c_char,
) -> *mut c_char {
    if h.is_null() || database.is_null() || table.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*h };
    let db = match unsafe { CStr::from_ptr(database) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let table_name = match unsafe { CStr::from_ptr(table) }.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match handle.client.list_columns_blocking(db, table_name) {
        Ok(columns) => into_json_cstring(&columns),
        Err(e) => {
            log::warn!("pier_mysql_list_columns failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Release a JSON string returned by any `pier_mysql_*`
/// function. Safe to call with NULL.
///
/// # Safety
///
/// `s`, if non-null, must have been returned by a
/// `pier_mysql_*` call and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller contract.
    drop(unsafe { CString::from_raw(s) });
}

/// Drop a MySQL handle. Safe to call with NULL.
///
/// # Safety
///
/// `h`, if non-null, must have been returned by
/// [`pier_mysql_open`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pier_mysql_free(h: *mut PierMysql) {
    if h.is_null() {
        return;
    }
    // SAFETY: caller contract — box originally from into_raw.
    let boxed = unsafe { Box::from_raw(h) };
    // Clone the client out and ask the pool to disconnect on
    // the shared runtime — the destructor otherwise leaves
    // the pool to drop asynchronously with no guarantees.
    let client = boxed.client.clone();
    let _ = crate::ssh::runtime::shared().block_on(client.disconnect());
}

// ── Internal helpers ────────────────────────────────────

/// Serialize `value` as JSON and wrap in a heap CString.
fn into_json_cstring<T: serde::Serialize>(value: &T) -> *mut c_char {
    let json = match serde_json::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("pier_mysql: json serialize failed: {e}");
            return ptr::null_mut();
        }
    };
    match CString::new(json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Emit an "empty result with error string" JSON payload so
/// the UI can show the server's error in place of a grid.
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
        // SAFETY: all null paths documented as safe.
        unsafe {
            assert!(pier_mysql_open(
                ptr::null(), 3306, ptr::null(), ptr::null(), ptr::null()
            )
            .is_null());
            assert!(pier_mysql_execute(ptr::null_mut(), ptr::null()).is_null());
            assert!(pier_mysql_list_databases(ptr::null_mut()).is_null());
            assert!(pier_mysql_list_tables(ptr::null_mut(), ptr::null()).is_null());
            pier_mysql_free_string(ptr::null_mut());
            pier_mysql_free(ptr::null_mut());
        }
    }

    #[test]
    fn unreachable_host_fails_fast() {
        let host = CString::new("192.0.2.1").unwrap();
        let user = CString::new("root").unwrap();
        let pass = CString::new("x").unwrap();
        let start = std::time::Instant::now();
        // SAFETY: all strings valid.
        let h = unsafe {
            pier_mysql_open(
                host.as_ptr(), 3306, user.as_ptr(), pass.as_ptr(), ptr::null()
            )
        };
        let elapsed = start.elapsed();
        assert!(h.is_null());
        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "mysql open should fail fast on unroutable host, took {elapsed:?}",
        );
    }

    #[test]
    fn zero_port_rejected() {
        let host = CString::new("127.0.0.1").unwrap();
        let user = CString::new("root").unwrap();
        // SAFETY: all strings valid.
        let h = unsafe {
            pier_mysql_open(host.as_ptr(), 0, user.as_ptr(), ptr::null(), ptr::null())
        };
        assert!(h.is_null());
    }

    #[test]
    fn error_as_json_carries_message() {
        let raw = error_as_json("syntax error near 'foo'");
        assert!(!raw.is_null());
        // SAFETY: raw was just produced by error_as_json.
        let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap();
        assert!(text.contains("\"error\":\"syntax error near 'foo'\""));
        assert!(text.contains("\"columns\":[]"));
        // SAFETY: release the allocation.
        unsafe { pier_mysql_free_string(raw) };
    }
}
