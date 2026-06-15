//! AI assistant backend (PRODUCT-SPEC §5.14 / §8.7).
//!
//! Three UI-agnostic capabilities, consumed by the desktop shell's
//! command layer:
//!
//!   * [`provider`] — streaming chat clients for Anthropic /
//!     OpenAI-compatible / Ollama endpoints. Blocking + callback
//!     style (house convention §8.3): the caller hands in an
//!     `FnMut` for deltas and a `CancellationToken`, and gets the
//!     finished turn (text + tool calls + usage) back.
//!   * [`risk`] — the single risk classifier for AI-proposed
//!     actions. L0 read-only auto-runs, L1 needs an approval card,
//!     L2 needs a strong confirm and can never be allow-listed,
//!     L3 is the red line: the execution channel does not exist.
//!     Fail-closed: anything the table doesn't recognise is L2.
//!   * [`redact`] — secret scrubbing for outbound prompt content
//!     and tool results (private-key blocks, cloud keys, tokens,
//!     `password=` pairs).
//!
//! No `tauri` / UI dependencies. HTTP goes through `ureq` (same
//! crate the webhook fan-out already uses) so this module adds
//! zero new heavyweight dependencies.

pub mod provider;
pub mod redact;
pub mod risk;
pub mod types;

pub use redact::scrub;
pub use risk::{classify_command, classify_list_path, classify_read_path, classify_write_path};
pub use types::*;
