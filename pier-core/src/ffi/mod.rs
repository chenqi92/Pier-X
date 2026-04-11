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
pub mod services;
pub mod sftp;
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
pub use self::services::{pier_services_detect, pier_services_free_json};
pub use self::tunnel::{
    pier_tunnel_free, pier_tunnel_is_alive, pier_tunnel_local_port, pier_tunnel_open, PierTunnel,
};
pub use self::sftp::{
    pier_sftp_canonicalize, pier_sftp_free, pier_sftp_free_string, pier_sftp_list_dir,
    pier_sftp_mkdir, pier_sftp_new, pier_sftp_remove_dir, pier_sftp_remove_file,
    pier_sftp_rename, PierSftp, PIER_AUTH_AGENT, PIER_AUTH_CREDENTIAL, PIER_AUTH_KEY,
    PIER_AUTH_PASSWORD,
};
pub use self::terminal::{
    pier_terminal_free, pier_terminal_is_alive, pier_terminal_last_ssh_error, pier_terminal_new,
    pier_terminal_new_ssh, pier_terminal_new_ssh_agent, pier_terminal_new_ssh_credential,
    pier_terminal_new_ssh_key, pier_terminal_resize, pier_terminal_snapshot, pier_terminal_write,
    PierCell, PierGridInfo,
};
