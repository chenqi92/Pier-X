//! C ABI for the Git panel.
//!
//! ## Handle model
//!
//! One opaque `*mut PierGit` per panel. The handle wraps a
//! single [`crate::services::git::GitClient`] bound to a
//! specific repository path. Dropping the handle is a no-op
//! (no live connections to close).
//!
//! ## JSON-shaped results
//!
//! Every read operation returns a heap JSON string that the
//! caller must release with [`pier_git_free_string`].
//!
//! ## Threading
//!
//! Every function is synchronous and blocking (subprocess
//! `git` calls). The C++ wrapper runs them on a worker thread
//! and posts results back via Qt's `QMetaObject::invokeMethod`.

#![allow(clippy::missing_safety_doc)]

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use crate::git_graph;
use crate::services::git::GitClient;

/// Opaque Git handle.
pub struct PierGit {
    client: GitClient,
}

// ─────────────────────────────────────────────────────────
// Lifecycle
// ─────────────────────────────────────────────────────────

/// Open a Git client for the repository at or above `repo_path`.
///
/// Runs `git rev-parse --show-toplevel` internally. Returns
/// NULL if the path is not inside a Git working tree.
///
/// # Safety
///
/// `repo_path` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn pier_git_open(repo_path: *const c_char) -> *mut PierGit {
    if repo_path.is_null() {
        return ptr::null_mut();
    }
    let path_str = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) if !s.is_empty() => s,
        _ => return ptr::null_mut(),
    };
    match GitClient::open(path_str) {
        Ok(client) => Box::into_raw(Box::new(PierGit { client })),
        Err(e) => {
            log::warn!("pier_git_open failed: {e}");
            ptr::null_mut()
        }
    }
}

/// Release the Git handle. Safe to call with NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_git_free(h: *mut PierGit) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h) });
    }
}

/// Release a heap JSON string returned by any `pier_git_*`
/// function. Safe to call with NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_git_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

// ─────────────────────────────────────────────────────────
// Helper
// ──────────────────────���──────────────────────────────────

fn to_json_cstring(json: &str) -> *mut c_char {
    CString::new(json)
        .map(|cs| cs.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn err_json(msg: &str) -> *mut c_char {
    let j = serde_json::json!({ "error": msg });
    to_json_cstring(&j.to_string())
}

// ─────────────────────────────────────────────────────────
// Status
// ───────────────────────────��─────────────────────────────

/// Get working tree status as a JSON array.
///
/// Returns `[{ "path": "...", "status": "M", "staged": true }, ...]`
/// or `{ "error": "..." }` on failure.
///
/// # Safety
///
/// `h` must be a valid PierGit handle.
#[no_mangle]
pub unsafe extern "C" fn pier_git_status(h: *mut PierGit) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    let client = unsafe { &(*h).client };
    match client.status() {
        Ok(changes) => {
            let json = serde_json::to_string(&changes).unwrap_or_else(|_| "[]".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e.to_string()),
    }
}

// ────────────────────────────��────────────────────────────
// Diff
// ─────────────────────────────────────────────────────────

/// Get the unified diff for a file.
///
/// `path` may be NULL or empty to get the full diff.
/// `staged` = 1 for index diff, 0 for working tree diff.
///
/// Returns the raw diff text or `{ "error": "..." }`.
///
/// # Safety
///
/// `h` must be a valid PierGit handle. `path` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_git_diff(
    h: *mut PierGit,
    path: *const c_char,
    staged: i32,
) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    let client = unsafe { &(*h).client };
    let path_str = if path.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(path) }.to_str() {
            Ok(s) => s,
            Err(_) => return err_json("invalid utf-8 path"),
        }
    };

    let result = if staged != 0 {
        client.diff(path_str, true)
    } else {
        client.diff(path_str, false)
    };

    match result {
        Ok(diff) => to_json_cstring(&diff),
        Err(e) => err_json(&e.to_string()),
    }
}

/// Get the diff for an untracked file (full content).
///
/// # Safety
///
/// `h` must be a valid PierGit handle.
#[no_mangle]
pub unsafe extern "C" fn pier_git_diff_untracked(
    h: *mut PierGit,
    path: *const c_char,
) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    if path.is_null() {
        return err_json("path is null");
    }
    let client = unsafe { &(*h).client };
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8 path"),
    };
    match client.diff_untracked(path_str) {
        Ok(diff) => to_json_cstring(&diff),
        Err(e) => err_json(&e.to_string()),
    }
}

// ─────────────────────────────────────────────────────────
// Branch info
// ─────────────���───────────────────────────────────────────

/// Get the current branch name and tracking information.
///
/// Returns JSON: `{ "name": "main", "tracking": "origin/main",
///                  "ahead": 2, "behind": 0 }`
///
/// # Safety
///
/// `h` must be a valid PierGit handle.
#[no_mangle]
pub unsafe extern "C" fn pier_git_branch_info(h: *mut PierGit) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    let client = unsafe { &(*h).client };
    match client.branch_info() {
        Ok(info) => {
            let json = serde_json::to_string(&info).unwrap_or_else(|_| "{}".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e.to_string()),
    }
}

// ───────────────────────────────���─────────────────────────
// Staging
// ─────────────────────────────────────────────────────────

/// Stage specific files. `paths_json` is a JSON array of paths.
///
/// Returns 0 on success, -1 on failure.
///
/// # Safety
///
/// `h` must be a valid PierGit handle.
#[no_mangle]
pub unsafe extern "C" fn pier_git_stage(
    h: *mut PierGit,
    paths_json: *const c_char,
) -> i32 {
    if h.is_null() || paths_json.is_null() {
        return -1;
    }
    let client = unsafe { &(*h).client };
    let json_str = match unsafe { CStr::from_ptr(paths_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let paths: Vec<String> = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match client.stage(&paths) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_git_stage failed: {e}");
            -1
        }
    }
}

/// Stage all changes.
///
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn pier_git_stage_all(h: *mut PierGit) -> i32 {
    if h.is_null() {
        return -1;
    }
    let client = unsafe { &(*h).client };
    match client.stage_all() {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_git_stage_all failed: {e}");
            -1
        }
    }
}

/// Unstage specific files. `paths_json` is a JSON array of paths.
///
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn pier_git_unstage(
    h: *mut PierGit,
    paths_json: *const c_char,
) -> i32 {
    if h.is_null() || paths_json.is_null() {
        return -1;
    }
    let client = unsafe { &(*h).client };
    let json_str = match unsafe { CStr::from_ptr(paths_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let paths: Vec<String> = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match client.unstage(&paths) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_git_unstage failed: {e}");
            -1
        }
    }
}

/// Unstage all files.
///
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn pier_git_unstage_all(h: *mut PierGit) -> i32 {
    if h.is_null() {
        return -1;
    }
    let client = unsafe { &(*h).client };
    match client.unstage_all() {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_git_unstage_all failed: {e}");
            -1
        }
    }
}

/// Discard working tree changes for specific files.
///
/// Returns 0 on success, -1 on failure.
#[no_mangle]
pub unsafe extern "C" fn pier_git_discard(
    h: *mut PierGit,
    paths_json: *const c_char,
) -> i32 {
    if h.is_null() || paths_json.is_null() {
        return -1;
    }
    let client = unsafe { &(*h).client };
    let json_str = match unsafe { CStr::from_ptr(paths_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let paths: Vec<String> = match serde_json::from_str(json_str) {
        Ok(p) => p,
        Err(_) => return -1,
    };
    match client.discard(&paths) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("pier_git_discard failed: {e}");
            -1
        }
    }
}

// ──────────────────────────────���──────────────────────────
// Commit
// ────────��────────────────────────────────────────────────

/// Create a commit with the given message.
///
/// Returns the commit output as a heap string, or
/// `{ "error": "..." }` on failure.
///
/// # Safety
///
/// `h` must be a valid PierGit handle.
#[no_mangle]
pub unsafe extern "C" fn pier_git_commit(
    h: *mut PierGit,
    message: *const c_char,
) -> *mut c_char {
    if h.is_null() || message.is_null() {
        return err_json("null handle or message");
    }
    let client = unsafe { &(*h).client };
    let msg = match unsafe { CStr::from_ptr(message) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8 message"),
    };
    match client.commit(msg) {
        Ok(out) => to_json_cstring(&out),
        Err(e) => err_json(&e.to_string()),
    }
}

// ───��─────────────────────────────────────────────────────
// Remote
// ──────────��──────────────────────────────────────────────

/// Push current branch to remote.
///
/// Returns the push output or `{ "error": "..." }`.
#[no_mangle]
pub unsafe extern "C" fn pier_git_push(h: *mut PierGit) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    let client = unsafe { &(*h).client };
    match client.push() {
        Ok(out) => to_json_cstring(&out),
        Err(e) => err_json(&e.to_string()),
    }
}

/// Pull from remote.
///
/// Returns the pull output or `{ "error": "..." }`.
#[no_mangle]
pub unsafe extern "C" fn pier_git_pull(h: *mut PierGit) -> *mut c_char {
    if h.is_null() {
        return err_json("null handle");
    }
    let client = unsafe { &(*h).client };
    match client.pull() {
        Ok(out) => to_json_cstring(&out),
        Err(e) => err_json(&e.to_string()),
    }
}

// ─────────────────────────────────────────────────────────
// Graph functions (from git_graph module)
// ───────────────────��─────────────────────────────────────

/// Load commit graph data with filters.
///
/// Returns a JSON array of CommitEntry objects, or
/// `{ "error": "..." }` on failure.
///
/// # Safety
///
/// `repo_path` must be a valid NUL-terminated UTF-8 string.
/// All optional filter params may be NULL.
#[no_mangle]
pub unsafe extern "C" fn pier_git_graph_log(
    repo_path: *const c_char,
    limit: u32,
    skip: u32,
    branch: *const c_char,
    author: *const c_char,
    search_text: *const c_char,
    after_timestamp: i64,
    topo_order: bool,
    first_parent: bool,
    no_merges: bool,
    paths: *const c_char,
) -> *mut c_char {
    if repo_path.is_null() {
        return err_json("null repo_path");
    }
    let rp = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8 repo_path"),
    };

    let opt_str = |p: *const c_char| -> Option<String> {
        if p.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(p) }
                .to_str()
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        }
    };

    let path_list: Vec<String> = if paths.is_null() {
        Vec::new()
    } else {
        match unsafe { CStr::from_ptr(paths) }.to_str() {
            Ok(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or_default(),
            _ => Vec::new(),
        }
    };

    let filter = git_graph::GraphFilter {
        branch: opt_str(branch),
        author: opt_str(author),
        search_text: opt_str(search_text),
        after_timestamp,
        topo_order,
        first_parent_only: first_parent,
        no_merges,
        paths: path_list,
    };

    match git_graph::graph_log(rp, limit as usize, skip as usize, &filter) {
        Ok(entries) => {
            let json = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e),
    }
}

/// Compute IDEA-style graph layout from commits JSON.
///
/// Returns a JSON array of GraphRow objects with segments and
/// arrows for rendering.
///
/// # Safety
///
/// `commits_json` and `main_chain_json` must be valid
/// NUL-terminated UTF-8 JSON strings.
#[no_mangle]
pub unsafe extern "C" fn pier_git_compute_graph_layout(
    commits_json: *const c_char,
    main_chain_json: *const c_char,
    lane_width: f32,
    row_height: f32,
    show_long_edges: bool,
) -> *mut c_char {
    if commits_json.is_null() || main_chain_json.is_null() {
        return err_json("null input");
    }
    let commits_str = match unsafe { CStr::from_ptr(commits_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8 commits_json"),
    };
    let chain_str = match unsafe { CStr::from_ptr(main_chain_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8 main_chain_json"),
    };

    let commits: Vec<git_graph::LayoutInput> = match serde_json::from_str(commits_str) {
        Ok(c) => c,
        Err(e) => return err_json(&format!("parse commits: {}", e)),
    };
    let chain_vec: Vec<String> = match serde_json::from_str(chain_str) {
        Ok(c) => c,
        Err(e) => return err_json(&format!("parse main_chain: {}", e)),
    };
    let main_chain: HashSet<String> = chain_vec.into_iter().collect();

    let params = git_graph::LayoutParams {
        lane_width,
        row_height,
        show_long_edges,
    };

    let rows = git_graph::compute_graph_layout(&commits, &main_chain, &params);
    let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
    to_json_cstring(&json)
}

/// List all branch names (local + remote).
///
/// Returns a JSON array of strings.
#[no_mangle]
pub unsafe extern "C" fn pier_git_list_branches(repo_path: *const c_char) -> *mut c_char {
    if repo_path.is_null() {
        return err_json("null repo_path");
    }
    let rp = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8"),
    };
    match git_graph::list_branches(rp) {
        Ok(branches) => {
            let json = serde_json::to_string(&branches).unwrap_or_else(|_| "[]".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e),
    }
}

/// List unique commit authors.
///
/// Returns a JSON array of strings.
#[no_mangle]
pub unsafe extern "C" fn pier_git_list_authors(
    repo_path: *const c_char,
    limit: u32,
) -> *mut c_char {
    if repo_path.is_null() {
        return err_json("null repo_path");
    }
    let rp = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8"),
    };
    match git_graph::list_authors(rp, limit as usize) {
        Ok(authors) => {
            let json = serde_json::to_string(&authors).unwrap_or_else(|_| "[]".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e),
    }
}

/// Get the first-parent chain hashes for a given ref.
///
/// Returns a JSON array of hash strings.
#[no_mangle]
pub unsafe extern "C" fn pier_git_first_parent_chain(
    repo_path: *const c_char,
    ref_name: *const c_char,
    limit: u32,
) -> *mut c_char {
    if repo_path.is_null() || ref_name.is_null() {
        return err_json("null input");
    }
    let rp = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8"),
    };
    let rn = match unsafe { CStr::from_ptr(ref_name) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8"),
    };
    match git_graph::first_parent_chain(rp, rn, limit as usize) {
        Ok(hashes) => {
            let json = serde_json::to_string(&hashes).unwrap_or_else(|_| "[]".into());
            to_json_cstring(&json)
        }
        Err(e) => err_json(&e),
    }
}

/// Detect the default branch (main/master).
///
/// Returns a heap string with the branch name.
#[no_mangle]
pub unsafe extern "C" fn pier_git_detect_default_branch(
    repo_path: *const c_char,
) -> *mut c_char {
    if repo_path.is_null() {
        return err_json("null repo_path");
    }
    let rp = match unsafe { CStr::from_ptr(repo_path) }.to_str() {
        Ok(s) => s,
        Err(_) => return err_json("invalid utf-8"),
    };
    match git_graph::detect_default_branch(rp) {
        Ok(name) => to_json_cstring(&name),
        Err(e) => err_json(&e),
    }
}
