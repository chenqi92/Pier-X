//! Secret scrubbing for outbound AI content (PRODUCT-SPEC §5.14.2).
//!
//! Runs over (a) user-attached context (terminal output, file
//! contents) before it is sent to the model, and (b) tool results
//! before they are fed back into the conversation — a `read_file`
//! of an `.env` or a `cat ~/.aws/credentials` must not exfiltrate
//! live keys to the provider.
//!
//! Hand-rolled scanning, no regex crate (same stance as the risk
//! classifier). Patterns are deliberately high-precision: masking a
//! real secret matters more than catching every exotic format, and
//! false positives erode trust in the highlight UI.

use serde::Serialize;

const MASK: &str = "[REDACTED]";

/// Result of one [`scrub`] pass.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrubResult {
    /// The input with every detected secret replaced by `[REDACTED]`.
    pub text: String,
    /// Human-readable labels of what was masked, deduplicated
    /// (e.g. `"private key block"`, `"password assignment"`).
    pub hits: Vec<String>,
}

/// Scrub `input`, masking anything that looks like a live secret.
pub fn scrub(input: &str) -> ScrubResult {
    let mut hits: Vec<String> = Vec::new();
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_key_block = false;

    for line in input.lines() {
        if in_key_block {
            if line.contains("-----END") {
                in_key_block = false;
                out_lines.push(format!("{MASK} (private key block end)"));
            } else {
                out_lines.push(MASK.to_string());
            }
            continue;
        }
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            in_key_block = true;
            push_hit(&mut hits, "private key block");
            out_lines.push(format!("{MASK} (private key block)"));
            continue;
        }
        out_lines.push(scrub_line(line, &mut hits));
    }

    // `lines()` drops a trailing newline; preserve it so diffs of
    // scrubbed-vs-raw content stay aligned.
    let mut text = out_lines.join("\n");
    if input.ends_with('\n') {
        text.push('\n');
    }
    ScrubResult { text, hits }
}

fn push_hit(hits: &mut Vec<String>, label: &str) {
    if !hits.iter().any(|h| h == label) {
        hits.push(label.to_string());
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '_'
        || c == '-'
        || c == '.'
        || c == '+'
        || c == '/'
        || c == '='
}

fn scrub_line(line: &str, hits: &mut Vec<String>) -> String {
    let mut s = line.to_string();
    s = scrub_authorization(&s, hits);
    s = scrub_key_value(&s, hits);
    s = scrub_known_prefixes(&s, hits);
    s = scrub_aws_key_id(&s, hits);
    s
}

/// `Authorization: Bearer xxx` / `Authorization: Basic xxx`.
fn scrub_authorization(line: &str, hits: &mut Vec<String>) -> String {
    let lower = line.to_ascii_lowercase();
    let Some(auth_pos) = lower.find("authorization") else {
        return line.to_string();
    };
    let after = &lower[auth_pos + "authorization".len()..];
    let Some(colon_rel) = after.find(':') else {
        return line.to_string();
    };
    let value_start = auth_pos + "authorization".len() + colon_rel + 1;
    let value = lower[value_start..].trim_start();
    if value.starts_with("bearer") || value.starts_with("basic") {
        push_hit(hits, "authorization header");
        let scheme_len = if value.starts_with("bearer") { 6 } else { 5 };
        let pad = lower[value_start..].len() - value.len();
        let token_start = value_start + pad + scheme_len;
        let kept = &line[..token_start];
        return format!("{kept} {MASK}");
    }
    line.to_string()
}

/// `password=…`, `api_key: …`, `export TOKEN=…`, etc.
fn scrub_key_value(line: &str, hits: &mut Vec<String>) -> String {
    const KEYS: &[&str] = &[
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "access_key",
        "accesskey",
        "secret_key",
        "secretkey",
        "private_key",
        "auth_token",
        "client_secret",
        "aws_secret_access_key",
        "aws_access_key_id",
    ];
    let mut result = line.to_string();
    let mut search_from = 0usize;
    loop {
        let lower = result.to_ascii_lowercase();
        let mut best: Option<(usize, usize)> = None; // (key_start, key_len)
        for key in KEYS {
            if let Some(rel) = lower[search_from..].find(key) {
                let pos = search_from + rel;
                let candidate = (pos, key.len());
                if best
                    .map(|b| candidate.0 < b.0 || (candidate.0 == b.0 && candidate.1 > b.1))
                    .unwrap_or(true)
                {
                    // Prefer the longest key at the same position so
                    // `aws_secret_access_key` wins over `secret`.
                    best = Some(candidate);
                }
            }
        }
        let Some((pos, key_len)) = best else { break };

        // Word-boundary check on both sides of the key.
        let before_ok = pos == 0
            || !result[..pos]
                .chars()
                .last()
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
        let after_key = &result[pos + key_len..];
        let trimmed = after_key.trim_start();
        let sep_ok = trimmed.starts_with('=') || trimmed.starts_with(':');
        if !before_ok || !sep_ok {
            search_from = pos + key_len;
            if search_from >= result.len() {
                break;
            }
            continue;
        }

        // Locate the value: after the separator, skip spaces/quotes.
        let sep_offset = after_key.len() - trimmed.len() + 1;
        let mut value_start = pos + key_len + sep_offset;
        let bytes: Vec<char> = result.chars().collect();
        while value_start < bytes.len()
            && (bytes[value_start] == ' '
                || bytes[value_start] == '"'
                || bytes[value_start] == '\'')
        {
            value_start += 1;
        }
        let mut value_end = value_start;
        while value_end < bytes.len()
            && !bytes[value_end].is_whitespace()
            && bytes[value_end] != '"'
            && bytes[value_end] != '\''
            && bytes[value_end] != '&'
        {
            value_end += 1;
        }
        if value_end > value_start {
            push_hit(hits, "credential assignment");
            let prefix: String = bytes[..value_start].iter().collect();
            let suffix: String = bytes[value_end..].iter().collect();
            result = format!("{prefix}{MASK}{suffix}");
            search_from = prefix.len() + MASK.len();
        } else {
            search_from = pos + key_len;
        }
        if search_from >= result.len() {
            break;
        }
    }
    result
}

/// Well-known vendor token prefixes.
fn scrub_known_prefixes(line: &str, hits: &mut Vec<String>) -> String {
    // (prefix, minimum trailing token chars, label)
    const PREFIXES: &[(&str, usize, &str)] = &[
        ("github_pat_", 20, "GitHub token"),
        ("ghp_", 20, "GitHub token"),
        ("gho_", 20, "GitHub token"),
        ("ghs_", 20, "GitHub token"),
        ("glpat-", 16, "GitLab token"),
        ("sk-ant-", 16, "Anthropic API key"),
        ("sk-", 20, "API key"),
        ("xoxb-", 16, "Slack token"),
        ("xoxp-", 16, "Slack token"),
        ("AIza", 30, "Google API key"),
    ];
    let mut result = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    'outer: while i < chars.len() {
        for (prefix, min_len, label) in PREFIXES {
            let pchars: Vec<char> = prefix.chars().collect();
            if i + pchars.len() <= chars.len()
                && chars[i..i + pchars.len()].iter().collect::<String>() == *prefix
            {
                // Boundary before the prefix.
                let boundary_ok = i == 0 || !is_token_char(chars[i - 1]);
                if boundary_ok {
                    let mut j = i + pchars.len();
                    while j < chars.len() && is_token_char(chars[j]) {
                        j += 1;
                    }
                    if j - (i + pchars.len()) >= *min_len {
                        push_hit(hits, label);
                        result.push_str(MASK);
                        i = j;
                        continue 'outer;
                    }
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// AWS access key id: `AKIA` + 16 uppercase alphanumerics.
fn scrub_aws_key_id(line: &str, hits: &mut Vec<String>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut result = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 20 <= chars.len()
            && chars[i..i + 4].iter().collect::<String>() == "AKIA"
            && chars[i + 4..i + 20]
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && (i == 0 || !is_token_char(chars[i - 1]))
            && (i + 20 == chars.len() || !is_token_char(chars[i + 20]))
        {
            push_hit(hits, "AWS access key id");
            result.push_str(MASK);
            i += 20;
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_private_key_block() {
        let input = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nmore\n-----END OPENSSH PRIVATE KEY-----\nafter";
        let r = scrub(input);
        assert!(!r.text.contains("b3BlbnNzaC1rZXk"));
        assert!(r.text.contains("after"));
        assert!(r.hits.iter().any(|h| h.contains("private key")));
    }

    #[test]
    fn masks_password_assignments() {
        let r = scrub("DB_PASSWORD=hunter2 host=db.local");
        assert!(!r.text.contains("hunter2"));
        assert!(r.text.contains("host=db.local"));
        let r2 = scrub("password: s3cret!");
        assert!(!r2.text.contains("s3cret"));
    }

    #[test]
    fn masks_known_token_prefixes() {
        let r = scrub("token ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA in env");
        assert!(!r.text.contains("ghp_AAAA"));
        let r2 = scrub("ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnop1234");
        assert!(!r2.text.contains("sk-ant-"));
    }

    #[test]
    fn masks_aws_key_id() {
        let r = scrub("aws key AKIAIOSFODNN7EXAMPLE found");
        assert!(!r.text.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(r.text.contains("found"));
    }

    #[test]
    fn masks_authorization_header() {
        let r = scrub("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig");
        assert!(!r.text.contains("eyJhbGci"));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let input = "tokenizer settings: passwordless sudo is disabled; see docs/secrets.md";
        let r = scrub(input);
        assert_eq!(r.text, input);
        assert!(r.hits.is_empty());
    }

    #[test]
    fn preserves_trailing_newline() {
        let r = scrub("line\n");
        assert!(r.text.ends_with('\n'));
    }
}
