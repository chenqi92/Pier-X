//! Scoring heuristic: pick the source with the most usable
//! information when multiple succeed.
//!
//! "Usable" means subcommand + option counts, weighted by how many
//! carry a non-empty description. Sparse output (e.g. `--help`
//! emitting only flag names) loses to a man page that has both
//! flags and a `COMMANDS` section.

use crate::schema::CommandPack;

/// Score a candidate pack. Higher is better. Pure function — the
/// caller picks `max_by_key`.
pub fn score(pack: &CommandPack) -> u32 {
    let mut total: u32 = 0;
    for sub in &pack.subcommands {
        total += 10;
        if has_en_description(&sub.i18n) {
            total += 5;
        }
        for opt in &sub.options {
            total += 1;
            if has_en_description(&opt.i18n) {
                total += 1;
            }
        }
    }
    for opt in &pack.options {
        total += 2;
        if has_en_description(&opt.i18n) {
            total += 2;
        }
    }
    total
}

fn has_en_description(i18n: &std::collections::BTreeMap<String, String>) -> bool {
    i18n.get("en").map(|s| !s.is_empty()).unwrap_or(false)
}
