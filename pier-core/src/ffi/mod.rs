//! C ABI surface — the stable boundary between `pier-core` and any UI layer.
//!
//! Functions in these submodules are `extern "C"` with C-compatible
//! types only. They are intended to be consumed from Qt (via a thin
//! C++ wrapper today, via cxx-qt once signals/slots are needed),
//! Swift, or any other language that speaks the C ABI.
//!
//! ## Memory rules
//!
//! - Any `*const c_char` returned from a pier-core function is
//!   either statically owned by pier-core (do not free) or
//!   heap-allocated by Rust (must be released by the matching
//!   `_free_*` function). Each function's documentation spells
//!   out which contract applies.
//! - NUL inputs are defined: they are handled gracefully without
//!   touching memory (documented per function).
//! - Opaque handles (`*mut PierTerminal`, etc.) are owned by the
//!   caller until the corresponding `_free` function is called.
//!   After `_free`, the handle is invalid and must not be reused.
//! - Numeric return values use `0` for "false/off" and `1` for
//!   "true/on" unless otherwise documented.
//!
//! ## Layout
//!
//! - [`core`] — version + build info + feature flags. Zero state.
//! - [`terminal`] — handle-based API around
//!   [`crate::terminal::PierTerminal`]: spawn, write, resize,
//!   snapshot, free. Includes both the in-memory password and
//!   credential-id SSH constructors.
//! - [`credentials`] — write-only access to the OS keyring
//!   (`set` / `delete`). Reads are deliberately not exposed —
//!   the SSH session layer pulls passwords directly via
//!   [`crate::credentials::get`] from inside Rust.
//! - [`connections`] — load / save the persisted SSH connection
//!   list as JSON.
//!
//! New subsystems (SFTP, database, …) each get their own
//! submodule here as they land.

pub mod connections;
pub mod core;
pub mod credentials;
pub mod docker;
pub mod log_stream;
pub mod markdown;
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod server_monitor;
pub mod services;
pub mod sftp;
pub mod ssh_session;
pub mod terminal;
pub mod tunnel;

// Re-export the individual C functions at the `ffi` root so
// symbols in libpier_core.a are not namespaced by submodule — the C
// header side sees them as flat extern symbols, which is what the
// hand-written pier_*.h headers declare.
pub use self::connections::{
    pier_connections_free_json, pier_connections_load_json, pier_connections_save_json,
};
pub use self::core::{pier_core_build_info, pier_core_has_feature, pier_core_version};
pub use self::credentials::{pier_credential_delete, pier_credential_set};
pub use self::docker::{
    pier_docker_free, pier_docker_free_string, pier_docker_inspect_container,
    pier_docker_list_containers, pier_docker_open, pier_docker_open_on_session,
    pier_docker_remove, pier_docker_restart, pier_docker_start, pier_docker_stop, PierDocker,
    PIER_DOCKER_ERR_FAILED, PIER_DOCKER_ERR_NULL, PIER_DOCKER_ERR_UNSAFE_ID,
    PIER_DOCKER_ERR_UTF8, PIER_DOCKER_OK,
};
pub use self::log_stream::{
    pier_log_drain, pier_log_exit_code, pier_log_free, pier_log_free_string, pier_log_is_alive,
    pier_log_open, pier_log_open_on_session, pier_log_stop, PierLogStream,
};
pub use self::markdown::{
    pier_markdown_free_string, pier_markdown_load_html, pier_markdown_load_source,
    pier_markdown_render_html,
};
pub use self::mysql::{
    pier_mysql_execute, pier_mysql_free, pier_mysql_free_string, pier_mysql_list_columns,
    pier_mysql_list_databases, pier_mysql_list_tables, pier_mysql_open,
    pier_mysql_open_with_credential, PierMysql,
};
pub use self::postgres::{
    pier_postgres_execute, pier_postgres_free, pier_postgres_free_string,
    pier_postgres_list_columns, pier_postgres_list_databases, pier_postgres_list_tables,
    pier_postgres_open, PierPostgres,
};
pub use self::server_monitor::{
    pier_server_monitor_free, pier_server_monitor_free_string, pier_server_monitor_open,
    pier_server_monitor_open_on_session, pier_server_monitor_probe, PierServerMonitor,
};
pub use self::redis::{
    pier_redis_free, pier_redis_free_string, pier_redis_info, pier_redis_inspect, pier_redis_open,
    pier_redis_ping, pier_redis_scan_keys, PierRedis,
};
pub use self::services::{
    pier_services_detect, pier_services_detect_on_session, pier_services_free_json,
};
pub use self::ssh_session::{
    pier_ssh_session_free, pier_ssh_session_is_alive, pier_ssh_session_last_error,
    pier_ssh_session_last_error_kind, pier_ssh_session_open, pier_ssh_session_refcount,
    PierSshSession,
};
pub use self::tunnel::{
    pier_tunnel_free, pier_tunnel_is_alive, pier_tunnel_local_port, pier_tunnel_open,
    pier_tunnel_open_on_session, PierTunnel,
};
pub use self::sftp::{
    pier_sftp_canonicalize, pier_sftp_free, pier_sftp_free_string, pier_sftp_list_dir,
    pier_sftp_mkdir, pier_sftp_new, pier_sftp_new_on_session, pier_sftp_remove_dir,
    pier_sftp_remove_file, pier_sftp_rename, PierSftp, PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL,
    PIER_AUTH_KEY, PIER_AUTH_PASSWORD,
};
pub use self::terminal::{
    pier_terminal_free, pier_terminal_is_alive, pier_terminal_last_ssh_error, pier_terminal_new,
    pier_terminal_new_ssh, pier_terminal_new_ssh_agent, pier_terminal_new_ssh_credential,
    pier_terminal_new_ssh_key, pier_terminal_new_ssh_on_session, pier_terminal_resize,
    pier_terminal_snapshot, pier_terminal_write, PierCell, PierGridInfo,
};
