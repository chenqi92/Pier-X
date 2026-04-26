//! Merge tool: take packs from N source directories (e.g.
//! `--src /tmp/packs-mac --src /tmp/packs-linux`), pick the
//! best-scoring pack per command, and emit the winners to
//! `--out`. Used after running the importer on multiple hosts.
//!
//! When two sources have the same command, scoring picks the
//! winner; ties prefer the LAST `--src` (so passing
//! `--src mac --src linux` makes Linux the tiebreaker, which is
//! usually what we want — Linux is where most users will use
//! these tools).

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;

#[path = "../schema.rs"]
mod schema;
#[path = "../score.rs"]
mod score;

use schema::CommandPack;

#[derive(Parser, Debug)]
struct Args {
    /// One or more source directories of `*.json` packs. Order
    /// matters for ties (later wins).
    #[arg(long = "src", required = true)]
    sources: Vec<PathBuf>,

    /// Output directory.
    #[arg(long, default_value = "merged")]
    out: PathBuf,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out)?;

    // (command_name, score) → (path, raw_body) keyed by command.
    let mut best: BTreeMap<String, (i32, PathBuf, String)> = BTreeMap::new();

    for (idx, dir) in args.sources.iter().enumerate() {
        let entries = match std::fs::read_dir(dir) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("warn: skipping {dir:?}: {e}");
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let body = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let pack: CommandPack = match serde_json::from_str(&body) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("warn: bad JSON in {path:?}: {e}");
                    continue;
                }
            };
            // Score gets a tiny tiebreaker bonus for later sources
            // (one point per source index). With Linux passed
            // last, a tie defaults to the Linux pack.
            let s = score::score(&pack) as i32 + idx as i32;
            let entry_key = pack.command.clone();
            match best.get(&entry_key) {
                Some((existing_score, _, _)) if *existing_score >= s => {
                    // Existing wins.
                }
                _ => {
                    best.insert(entry_key, (s, path, body));
                }
            }
        }
    }

    let n = best.len();
    for (cmd, (_, src, body)) in &best {
        let out = args.out.join(format!("{cmd}.json"));
        std::fs::write(&out, body)?;
        println!("✓ {cmd:<14} ← {}", src.display());
    }
    println!("\nWrote {n} merged pack(s) to {}", args.out.display());
    Ok(())
}
