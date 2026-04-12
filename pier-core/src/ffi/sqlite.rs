//! C ABI for the SQLite inspector panel.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::services::sqlite::SqliteClient;

pub struct PierSqlite {
    client: SqliteClient,
}

fn to_json(s: &str) -> *mut c_char {
    CString::new(s).map(|c| c.into_raw()).unwrap_or(ptr::null_mut())
}
fn err_json(msg: &str) -> *mut c_char {
    to_json(&serde_json::json!({"error": msg}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_open(path: *const c_char) -> *mut PierSqlite {
    if path.is_null() { return ptr::null_mut(); }
    let p = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    match SqliteClient::open(p) {
        Ok(client) => Box::into_raw(Box::new(PierSqlite { client })),
        Err(e) => { log::warn!("pier_sqlite_open: {e}"); ptr::null_mut() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_free(h: *mut PierSqlite) {
    if !h.is_null() { drop(unsafe { Box::from_raw(h) }); }
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_free_string(s: *mut c_char) {
    if !s.is_null() { drop(unsafe { CString::from_raw(s) }); }
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_list_tables(h: *mut PierSqlite) -> *mut c_char {
    if h.is_null() { return err_json("null"); }
    match unsafe { &(*h).client }.list_tables() {
        Ok(t) => to_json(&serde_json::to_string(&t).unwrap_or_else(|_| "[]".into())),
        Err(e) => err_json(&e.to_string()),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_table_columns(
    h: *mut PierSqlite, table: *const c_char,
) -> *mut c_char {
    if h.is_null() || table.is_null() { return err_json("null"); }
    let t = match unsafe { CStr::from_ptr(table) }.to_str() { Ok(s) => s, Err(_) => return err_json("utf8") };
    match unsafe { &(*h).client }.table_columns(t) {
        Ok(c) => to_json(&serde_json::to_string(&c).unwrap_or_else(|_| "[]".into())),
        Err(e) => err_json(&e.to_string()),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pier_sqlite_execute(
    h: *mut PierSqlite, sql: *const c_char,
) -> *mut c_char {
    if h.is_null() || sql.is_null() { return err_json("null"); }
    let s = match unsafe { CStr::from_ptr(sql) }.to_str() { Ok(s) => s, Err(_) => return err_json("utf8") };
    let result = unsafe { &(*h).client }.execute(s);
    to_json(&serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()))
}
