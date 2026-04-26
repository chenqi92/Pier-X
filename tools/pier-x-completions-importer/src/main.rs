//! Pier-X completion-pack importer.
//!
//! Off-line tool. Run on a build machine where the target tools
//! (docker / git / kubectl / ...) are installed; output is a
//! directory of `<command>.json` files matching the schema in
//! `pier_core::terminal::library::CommandPack`.
//!
//! The tool runs **independent of fish-shell** — every byte of
//! data comes from the user's locally installed CLI binaries via
//! one of three sources, in priority order:
//!
//!   1. `<cmd> completion zsh` — the richest format, used when the
//!      tool ships its own completion generator (kubectl, docker,
//!      gh, helm, fluxctl, npm, yarn, pnpm, cargo, rustup, ...).
//!   2. `man <cmd>` — parsed from the `SYNOPSIS` / `DESCRIPTION`
//!      / `OPTIONS` / `COMMANDS` sections.
//!   3. `<cmd> --help` — generic last-resort. Recognises clap /
//!      argparse / cobra-style layouts.
//!
//! For each command the tool tries all available sources and
//! picks the one with the most usable data (highest `score`). The
//! result is annotated with `source` (which method was used),
//! `tool_version` (best-effort parse from the tool's `--version`
//! output), and `import_date` (today's date).
//!
//! ## Usage
//!
//! ```text
//! cargo run -p pier-x-completions-importer -- \
//!     build --out pier-core/resources/completions
//! ```
//!
//! Or via a TOML seed file listing extra commands:
//!
//! ```toml
//! # tools/pier-x-completions-importer/seeds/default-list.toml
//! commands = ["docker", "git", "kubectl", "npm", "ssh"]
//! ```
//!
//! ```text
//! cargo run -p pier-x-completions-importer -- \
//!     build --seeds seeds/default-list.toml --out packs/
//! ```

use std::path::PathBuf;
use std::process::Command;

use clap::Parser;
use log::{info, warn};

mod schema;
mod score;
mod sources;

use schema::CommandPack;

#[derive(Parser, Debug)]
#[command(
    name = "pier-x-completions-importer",
    about = "Generate Pier-X completion packs from --help / man / completion scripts"
)]
struct Args {
    /// Output directory — `<cmd>.json` files land here.
    #[arg(long, default_value = "packs")]
    out: PathBuf,

    /// One or more command names to import. When omitted, falls
    /// back to the `--seeds` TOML.
    #[arg(long)]
    cmd: Vec<String>,

    /// TOML file with `commands = [...]`. Used when `--cmd` is
    /// not supplied.
    #[arg(long, default_value = "tools/pier-x-completions-importer/seeds/default-list.toml")]
    seeds: PathBuf,

    /// Force one specific source instead of picking by score.
    /// Useful for debugging a single parser.
    #[arg(long, value_enum)]
    force_source: Option<ForcedSource>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ForcedSource {
    CompletionZsh,
    Man,
    Help,
}

#[derive(serde::Deserialize)]
struct SeedFile {
    commands: Vec<String>,
}

fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    let commands: Vec<String> = if !args.cmd.is_empty() {
        args.cmd.clone()
    } else {
        let body = std::fs::read_to_string(&args.seeds)?;
        let seed: SeedFile = toml::from_str(&body)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        seed.commands
    };

    std::fs::create_dir_all(&args.out)?;

    for cmd in &commands {
        match build_pack(cmd, args.force_source) {
            Ok(pack) => {
                let path = args.out.join(format!("{cmd}.json"));
                let body = serde_json::to_string_pretty(&pack)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                std::fs::write(&path, body)?;
                info!(
                    "✓ {cmd} → {} ({} subcommands, {} options, source={})",
                    path.display(),
                    pack.subcommands.len(),
                    pack.options.len(),
                    pack.import_method,
                );
            }
            Err(e) => {
                warn!("✗ {cmd}: {e}");
            }
        }
    }

    Ok(())
}

/// Pick the best-scoring source for `cmd` and emit a pack.
fn build_pack(cmd: &str, forced: Option<ForcedSource>) -> Result<CommandPack, String> {
    let tool_version = detect_version(cmd);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();

    let candidates: Vec<(CommandPack, &'static str)> = match forced {
        Some(ForcedSource::CompletionZsh) => sources::completion_zsh::extract(cmd)
            .map(|p| vec![(p, "completion-zsh")])
            .unwrap_or_default(),
        Some(ForcedSource::Man) => sources::man::extract(cmd)
            .map(|p| vec![(p, "man")])
            .unwrap_or_default(),
        Some(ForcedSource::Help) => sources::help::extract(cmd)
            .map(|p| vec![(p, "help")])
            .unwrap_or_default(),
        None => {
            let mut out = Vec::new();
            if let Ok(p) = sources::completion_zsh::extract(cmd) {
                out.push((p, "completion-zsh"));
            }
            if let Ok(p) = sources::man::extract(cmd) {
                out.push((p, "man"));
            }
            if let Ok(p) = sources::help::extract(cmd) {
                out.push((p, "help"));
            }
            out
        }
    };

    if candidates.is_empty() {
        return Err(format!(
            "no source produced data for `{cmd}` — install it locally or skip"
        ));
    }

    let (best, method) = candidates
        .into_iter()
        .max_by_key(|(p, _)| score::score(p))
        .ok_or("scoring picked nothing")?;

    Ok(CommandPack {
        schema_version: 1,
        command: cmd.to_string(),
        tool_version,
        source: "auto-imported".to_string(),
        import_method: method.to_string(),
        import_date: date,
        subcommands: best.subcommands,
        options: best.options,
    })
}

/// Best-effort version probe. Tries `--version` and falls back to
/// `version` (no flag — git/docker top-level), parses the first
/// version-shaped token. Empty when both fail.
fn detect_version(cmd: &str) -> String {
    for args in [vec!["--version"], vec!["version"]] {
        let out = match Command::new(cmd).args(&args).output() {
            Ok(o) => o,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(v) = first_version_token(&text) {
            return v;
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        if let Some(v) = first_version_token(&stderr) {
            return v;
        }
    }
    String::new()
}

fn first_version_token(text: &str) -> Option<String> {
    // Match `1.2.3` or `1.2` somewhere in the first 256 chars —
    // we trust that the version line dominates the early output.
    let head = &text[..text.len().min(256)];
    let mut chars = head.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_ascii_digit() {
            continue;
        }
        let mut buf = String::from(c);
        while let Some(&nc) = chars.peek() {
            if nc.is_ascii_digit() || nc == '.' {
                buf.push(nc);
                chars.next();
            } else {
                break;
            }
        }
        if buf.contains('.') && buf.split('.').filter(|s| !s.is_empty()).count() >= 2 {
            return Some(buf);
        }
    }
    None
}
