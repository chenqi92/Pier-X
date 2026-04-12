//! Per-service tool panels ("service clients").
//!
//! M5 introduces a family of dedicated clients that live above
//! a plain SSH local-forward tunnel. The user clicks a service
//! pill in the terminal view, Pier-X opens a tunnel via
//! [`crate::ssh::tunnel`], and the client here connects to
//! `localhost:<tunnel_port>` using the native wire protocol.
//!
//! ## Why not reuse ssh::*?
//!
//! The `ssh` module knows how to move bytes, stat files, and
//! forward TCP — nothing else. Each service has its own client
//! library with its own connection pooling, command shape, and
//! error model. Putting Redis, MySQL, Docker, etc. under one
//! generic `ssh::protocol` trait would force a lowest-common-
//! denominator API that helps nobody.
//!
//! Instead this module is a thin home for one submodule per
//! service:
//!
//!   * `redis`   — Redis / Valkey browser (M5a, this commit)
//!   * `mysql`   — MySQL / MariaDB client (M5b)
//!   * `docker`  — Docker containers + images (M5c)
//!
//! All submodules share two conventions:
//!
//!   1. A `connect(host, port)` entry point returning a handle
//!      that clones cheaply.
//!   2. Both `async` and `_blocking` method pairs. The blocking
//!      variants `runtime::shared().block_on(...)` the async
//!      one so the UI can call directly from the Qt main thread
//!      without a threadpool of its own.
//!
//! The handle types are always Send + Sync so the UI layer can
//! stash them inside QObjects that cross thread boundaries.

pub mod docker;
pub mod git;
pub mod local_exec;
pub mod mysql;
pub mod postgres;
pub mod redis;
pub mod search;
pub mod server_monitor;
pub mod sqlite;
