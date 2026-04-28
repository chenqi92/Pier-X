//! Apache (httpd) config parser + renderer.
//!
//! Apache's grammar is line-oriented:
//!
//! * `Directive arg1 arg2`   — newline terminated; `\<newline>` joins
//! * `<Section args>` ... `</Section>` — recursive container
//! * `# comment`             — to end of line
//! * `"quoted args"`         — only double quotes; backslash escapes
//!                             `\"` and `\\`
//!
//! The parser is line-based: we split the source into logical lines
//! (after collapsing `\<newline>` continuation), tokenize each line
//! into a `(name, args)` pair, then walk the line stream maintaining
//! a section stack. Lines that look like `<Name ...>` push a new
//! section; `</Name>` pops. Mismatched tags are reported as warnings
//! and we recover by treating the unmatched line as a leaf directive.
//!
//! The AST mirrors caddy / nginx: each statement is a `Directive`
//! with a name + args + optional `section` body. Round-tripping
//! preserves comments, blank lines, and quoted-arg styles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApacheParseResult {
    pub nodes: Vec<ApacheNode>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ApacheNode {
    Directive(ApacheDirective),
    Comment {
        text: String,
        leading_blanks: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApacheDirective {
    /// e.g. `ServerName`, `VirtualHost`, `Directory`. Section names
    /// are stored without the angle brackets — the renderer adds them.
    pub name: String,
    pub args: Vec<String>,
    pub leading_comments: Vec<String>,
    pub leading_blanks: u32,
    pub inline_comment: Option<String>,
    /// `Some` when this directive is a section container — the body
    /// is the children between `<Name ...>` and `</Name>`.
    pub section: Option<Vec<ApacheNode>>,
}

pub fn parse(src: &str) -> ApacheParseResult {
    let lines = split_logical_lines(src);
    let mut idx = 0;
    let mut errors = Vec::new();
    let nodes = parse_lines(&lines, &mut idx, None, &mut errors);
    ApacheParseResult { nodes, errors }
}

pub fn render(nodes: &[ApacheNode]) -> String {
    let mut out = String::new();
    render_nodes(nodes, 0, &mut out);
    out
}

// ── Line splitter ───────────────────────────────────────────────────
//
// Yields one entry per logical line. Backslash-newline continuation
// is collapsed; the leading-blanks count is the number of blank
// lines that preceded this line. Comments come back as `Logical::Comment`
// so the parser can attach them as halo to the next directive.

#[derive(Debug, Clone, PartialEq, Eq)]
enum Logical {
    Blank,
    Comment(String),
    Line(String),
}

fn split_logical_lines(src: &str) -> Vec<Logical> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut continuation = false;
    for raw in src.split('\n') {
        // Detect backslash-newline continuation. `\\` at end of line
        // (an unescaped backslash followed by the newline we just
        // split on) means "join with next line".
        if continuation {
            buf.push_str(raw);
        } else {
            buf = raw.to_string();
        }
        let stripped_continuation = buf.trim_end_matches('\r').to_string();
        if stripped_continuation.ends_with('\\') {
            buf.truncate(stripped_continuation.len() - 1);
            buf.push(' '); // join with whitespace
            continuation = true;
            continue;
        }
        continuation = false;
        let line = buf.trim_end_matches('\r').to_string();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            out.push(Logical::Blank);
        } else if let Some(rest) = trimmed.strip_prefix('#') {
            out.push(Logical::Comment(rest.trim().to_string()));
        } else {
            out.push(Logical::Line(trimmed.to_string()));
        }
    }
    // If the file ended with a continuation marker, fall through with
    // whatever buffer we have.
    out
}

// ── Line tokenizer ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum LineKind {
    /// `<Name args>` — opens a section.
    SectionOpen {
        name: String,
        args: Vec<String>,
    },
    /// `</Name>` — closes a section.
    SectionClose {
        name: String,
    },
    /// `Name args [# inline comment]` — a leaf directive.
    Directive {
        name: String,
        args: Vec<String>,
        inline_comment: Option<String>,
    },
}

fn tokenize_line(line: &str) -> Result<LineKind, String> {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return Err("empty directive line".to_string());
    }

    // Section close: `</Name>` (no args, may have trailing whitespace).
    if line.starts_with("</") {
        let body = &line[2..];
        let close = body
            .find('>')
            .ok_or_else(|| format!("unterminated section close: {line}"))?;
        let name = body[..close].trim();
        let trailing = body[close + 1..].trim();
        if !trailing.is_empty() && !trailing.starts_with('#') {
            return Err(format!(
                "unexpected content after `</{name}>`: {trailing}"
            ));
        }
        return Ok(LineKind::SectionClose {
            name: name.to_string(),
        });
    }

    // Section open: `<Name args>` ending with `>` on the same line.
    // Apache permits `<If "regex with > in it">` — we use the LAST `>`
    // on the line as the close marker.
    if line.starts_with('<') {
        // Strip a trailing `# comment` first (so we don't eat a `>`
        // inside a comment).
        let (head, _trailing_comment) = strip_inline_comment(line);
        if head.starts_with('<') {
            let body = &head[1..];
            let close_rel = body
                .rfind('>')
                .ok_or_else(|| format!("unterminated section open: {line}"))?;
            let inside = &body[..close_rel];
            let mut tokens = tokenize_args(inside)?;
            if tokens.is_empty() {
                return Err(format!("section open with no name: {line}"));
            }
            let name = tokens.remove(0);
            return Ok(LineKind::SectionOpen { name, args: tokens });
        }
    }

    // Regular directive. Split off any trailing inline comment first.
    let (head, inline_comment) = strip_inline_comment(line);
    let mut tokens = tokenize_args(head.trim())?;
    if tokens.is_empty() {
        return Err(format!("directive line had no name: {line}"));
    }
    let name = tokens.remove(0);
    Ok(LineKind::Directive {
        name,
        args: tokens,
        inline_comment,
    })
}

/// Split a line into its non-comment head + optional inline comment
/// text. A `#` inside a quoted arg does NOT start a comment.
fn strip_inline_comment(line: &str) -> (&str, Option<String>) {
    let mut in_quote: Option<char> = None;
    let mut prev_was_space = true;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '"' if in_quote.is_none() => in_quote = Some('"'),
            '"' if in_quote == Some('"') => {
                // Check for escaped `\"`.
                if i > 0 && bytes[i - 1] == b'\\' {
                    // Escaped — stay in quote.
                } else {
                    in_quote = None;
                }
            }
            '#' if in_quote.is_none() && prev_was_space => {
                let head = &line[..i];
                let comment = line[i + 1..].trim().to_string();
                return (head.trim_end(), Some(comment));
            }
            _ => {}
        }
        prev_was_space = c.is_whitespace();
        i += 1;
    }
    (line, None)
}

/// Tokenize a string into args. Splits on whitespace, respects
/// double-quoted strings (with `\"` / `\\` escapes). Single quotes
/// are literal characters per Apache convention.
fn tokenize_args(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut in_quote = false;
    let mut had_quote = false;
    while let Some(c) = chars.next() {
        if in_quote {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        cur.push('\\');
                        cur.push(next);
                        chars.next();
                        continue;
                    }
                }
                cur.push(c);
                continue;
            }
            if c == '"' {
                in_quote = false;
                continue;
            }
            cur.push(c);
        } else if c == '"' {
            in_quote = true;
            had_quote = true;
            continue;
        } else if c.is_whitespace() {
            if !cur.is_empty() || had_quote {
                let arg = if had_quote {
                    format!("\"{cur}\"")
                } else {
                    std::mem::take(&mut cur)
                };
                if had_quote {
                    out.push(arg);
                    cur.clear();
                } else {
                    out.push(arg);
                }
                had_quote = false;
            }
        } else {
            cur.push(c);
        }
    }
    if in_quote {
        return Err(format!("unterminated quoted string: {s}"));
    }
    if !cur.is_empty() || had_quote {
        if had_quote {
            out.push(format!("\"{cur}\""));
        } else {
            out.push(cur);
        }
    }
    Ok(out)
}

// ── Parser core ─────────────────────────────────────────────────────

fn parse_lines(
    lines: &[Logical],
    idx: &mut usize,
    expect_close: Option<&str>,
    errors: &mut Vec<String>,
) -> Vec<ApacheNode> {
    let mut out: Vec<ApacheNode> = Vec::new();
    let mut pending_blanks: u32 = 0;
    let mut pending_comments: Vec<String> = Vec::new();

    while *idx < lines.len() {
        match &lines[*idx] {
            Logical::Blank => {
                pending_blanks = pending_blanks.saturating_add(1);
                *idx += 1;
            }
            Logical::Comment(text) => {
                if pending_blanks >= 2 && !pending_comments.is_empty() {
                    drain_comments(&mut out, &mut pending_comments, pending_blanks - 1);
                }
                pending_comments.push(text.clone());
                pending_blanks = 0;
                *idx += 1;
            }
            Logical::Line(text) => match tokenize_line(text) {
                Err(e) => {
                    errors.push(e);
                    *idx += 1;
                }
                Ok(LineKind::SectionClose { name }) => {
                    if let Some(expected) = expect_close {
                        if expected.eq_ignore_ascii_case(&name) {
                            // Consume the close and return; trailing
                            // pending comments stay as-is at the end
                            // of the section.
                            drain_comments(&mut out, &mut pending_comments, 0);
                            *idx += 1;
                            return out;
                        }
                        errors.push(format!(
                            "section mismatch: expected </{expected}>, got </{name}>",
                        ));
                        // Treat as best-effort close.
                        *idx += 1;
                        return out;
                    }
                    errors.push(format!("stray </{name}> at top level"));
                    *idx += 1;
                }
                Ok(LineKind::SectionOpen { name, args }) => {
                    *idx += 1;
                    let body = parse_lines(lines, idx, Some(&name), errors);
                    out.push(ApacheNode::Directive(ApacheDirective {
                        name,
                        args,
                        leading_comments: std::mem::take(&mut pending_comments),
                        leading_blanks: pending_blanks.saturating_sub(1),
                        inline_comment: None,
                        section: Some(body),
                    }));
                    pending_blanks = 0;
                }
                Ok(LineKind::Directive {
                    name,
                    args,
                    inline_comment,
                }) => {
                    out.push(ApacheNode::Directive(ApacheDirective {
                        name,
                        args,
                        leading_comments: std::mem::take(&mut pending_comments),
                        leading_blanks: pending_blanks.saturating_sub(1),
                        inline_comment,
                        section: None,
                    }));
                    pending_blanks = 0;
                    *idx += 1;
                }
            },
        }
    }

    if let Some(expected) = expect_close {
        errors.push(format!(
            "unterminated section: expected </{expected}> before EOF",
        ));
    }
    drain_comments(&mut out, &mut pending_comments, 0);
    out
}

fn drain_comments(
    out: &mut Vec<ApacheNode>,
    pending: &mut Vec<String>,
    blanks_for_first: u32,
) {
    let mut first = true;
    for c in pending.drain(..) {
        out.push(ApacheNode::Comment {
            text: c,
            leading_blanks: if first { blanks_for_first } else { 0 },
        });
        first = false;
    }
}

// ── Renderer ────────────────────────────────────────────────────────

fn render_nodes(nodes: &[ApacheNode], depth: usize, out: &mut String) {
    for (i, node) in nodes.iter().enumerate() {
        let leading_blanks = match node {
            ApacheNode::Comment { leading_blanks, .. } => *leading_blanks,
            ApacheNode::Directive(d) => d.leading_blanks,
        };
        if i > 0 {
            for _ in 0..leading_blanks.min(2) {
                out.push('\n');
            }
        }
        match node {
            ApacheNode::Comment { text, .. } => {
                indent(out, depth);
                out.push('#');
                if !text.is_empty() {
                    out.push(' ');
                    out.push_str(text);
                }
                out.push('\n');
            }
            ApacheNode::Directive(d) => render_directive(d, depth, out),
        }
    }
}

fn render_directive(d: &ApacheDirective, depth: usize, out: &mut String) {
    for c in &d.leading_comments {
        indent(out, depth);
        out.push('#');
        if !c.is_empty() {
            out.push(' ');
            out.push_str(c);
        }
        out.push('\n');
    }
    indent(out, depth);
    if let Some(body) = &d.section {
        // Section open tag.
        out.push('<');
        out.push_str(&d.name);
        for a in &d.args {
            out.push(' ');
            out.push_str(a);
        }
        out.push('>');
        if let Some(c) = &d.inline_comment {
            out.push_str(" #");
            if !c.is_empty() {
                out.push(' ');
                out.push_str(c);
            }
        }
        out.push('\n');
        render_nodes(body, depth + 1, out);
        indent(out, depth);
        out.push_str("</");
        out.push_str(&d.name);
        out.push_str(">\n");
    } else {
        out.push_str(&d.name);
        for a in &d.args {
            out.push(' ');
            out.push_str(a);
        }
        if let Some(c) = &d.inline_comment {
            out.push_str(" #");
            if !c.is_empty() {
                out.push(' ');
                out.push_str(c);
            }
        }
        out.push('\n');
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> ApacheParseResult {
        let r = parse(src);
        assert!(r.errors.is_empty(), "parse errors: {:?}", r.errors);
        r
    }

    #[test]
    fn simple_directive() {
        let r = parse_ok("ServerName example.com\n");
        assert_eq!(r.nodes.len(), 1);
    }

    #[test]
    fn vhost_section() {
        let src = "<VirtualHost *:80>\n    ServerName example.com\n    DocumentRoot /var/www\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("<VirtualHost *:80>"));
        assert!(out.contains("</VirtualHost>"));
        assert!(out.contains("ServerName example.com"));
    }

    #[test]
    fn nested_directory_inside_vhost() {
        let src = "<VirtualHost *:80>\n    DocumentRoot /var/www\n    <Directory /var/www>\n        Require all granted\n    </Directory>\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("<Directory /var/www>"));
        assert!(out.contains("Require all granted"));
    }

    #[test]
    fn quoted_arg() {
        let src = "ServerAdmin \"webmaster@example.com\"\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("\"webmaster@example.com\""));
    }

    #[test]
    fn comment_round_trip() {
        let src = "# top-level comment\nServerName example.com\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("# top-level comment"));
        assert!(out.contains("ServerName example.com"));
    }

    #[test]
    fn line_continuation() {
        let src = "RewriteRule ^/old$ \\\n    /new [R=301,L]\n";
        let r = parse_ok(src);
        // Continuation collapses into one logical directive.
        assert_eq!(r.nodes.len(), 1);
    }

    #[test]
    fn ifmodule_section() {
        let src = "<IfModule mod_ssl.c>\n    Listen 443\n</IfModule>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("<IfModule mod_ssl.c>"));
        assert!(out.contains("Listen 443"));
    }

    #[test]
    fn nested_ifmodule_inside_vhost() {
        let src =
            "<VirtualHost *:80>\n    ServerName example.com\n    <IfModule mod_rewrite.c>\n        RewriteEngine on\n    </IfModule>\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("<VirtualHost *:80>"));
        assert!(out.contains("    <IfModule mod_rewrite.c>"));
        assert!(out.contains("        RewriteEngine on"));
        assert!(out.contains("    </IfModule>"));
        assert!(out.contains("</VirtualHost>"));
    }

    #[test]
    fn rewrite_rule_with_regex() {
        let src = "RewriteRule ^/api/(.*)$ http://backend/$1 [P,L]\n";
        let r = parse_ok(src);
        let d = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(d.name, "RewriteRule");
        assert_eq!(d.args.len(), 3);
        assert_eq!(d.args[0], "^/api/(.*)$");
        assert_eq!(d.args[1], "http://backend/$1");
        assert_eq!(d.args[2], "[P,L]");
    }

    #[test]
    fn if_section_with_gt_inside_quotes() {
        // Apache's `<If>` sections can have regex args that contain
        // `>`. The lexer uses the LAST `>` on the line.
        let src =
            "<If \"%{REQUEST_URI} =~ /\\>/\">\n    Require all denied\n</If>\n";
        let r = parse_ok(src);
        let outer = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(outer.name, "If");
        assert!(outer.section.is_some());
    }

    #[test]
    fn empty_section_round_trips() {
        let src = "<VirtualHost *:80>\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("<VirtualHost *:80>"));
        assert!(out.contains("</VirtualHost>"));
    }

    #[test]
    fn case_insensitive_section_close() {
        // Apache treats section names case-insensitively.
        let src = "<VirtualHost *:80>\n    ServerName ex.com\n</virtualhost>\n";
        let r = parse(src);
        // No errors expected — case-insensitive match.
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
    }

    #[test]
    fn server_alias_with_multiple_hostnames() {
        let src = "ServerAlias www.example.com api.example.com cdn.example.com\n";
        let r = parse_ok(src);
        let d = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(d.args.len(), 3);
    }

    #[test]
    fn comment_inside_section() {
        let src =
            "<VirtualHost *:80>\n    # the main site\n    ServerName ex.com\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        assert!(out.contains("# the main site"));
    }

    #[test]
    fn mismatched_section_close_recovers() {
        // Should report an error but not panic.
        let src = "<VirtualHost *:80>\n    ServerName ex.com\n</Directory>\n";
        let r = parse(src);
        assert!(!r.errors.is_empty());
        // Some content still parses.
        assert!(!r.nodes.is_empty());
    }

    #[test]
    fn quoted_arg_with_hash_no_comment() {
        // `#` inside a quoted string is not a comment.
        let src = "ServerAdmin \"webmaster#example.com\"\n";
        let r = parse_ok(src);
        let d = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(d.args.len(), 1);
        assert!(d.args[0].contains('#'));
        assert!(d.inline_comment.is_none());
    }

    #[test]
    fn inline_comment_on_directive() {
        let src = "ServerName example.com   # the public hostname\n";
        let r = parse_ok(src);
        let d = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(d.name, "ServerName");
        assert_eq!(d.inline_comment.as_deref(), Some("the public hostname"));
    }

    #[test]
    fn directory_inside_vhost_round_trip() {
        let src = "<VirtualHost *:80>\n    DocumentRoot /var/www\n    <Directory /var/www>\n        Require all granted\n        AllowOverride All\n    </Directory>\n</VirtualHost>\n";
        let r = parse_ok(src);
        let out = render(&r.nodes);
        // Re-parse the output to confirm we didn't drift.
        let r2 = parse_ok(&out);
        assert_eq!(r.nodes, r2.nodes);
    }

    /// parse → render → parse again and assert AST equality.
    fn assert_idempotent(src: &str, label: &str) {
        let r1 = parse(src);
        assert!(
            r1.errors.is_empty(),
            "{label} first parse errors: {:?}",
            r1.errors
        );
        let rendered = render(&r1.nodes);
        let r2 = parse(&rendered);
        assert!(
            r2.errors.is_empty(),
            "{label} second parse errors: {:?}\nrendered:\n{}",
            r2.errors,
            rendered
        );
        assert_eq!(
            r1.nodes, r2.nodes,
            "{label} drifted on second parse. rendered:\n{}",
            rendered
        );
    }

    #[test]
    fn round_trip_default_vhost() {
        let src = "<VirtualHost *:80>\n    ServerName example.com\n    ServerAlias www.example.com\n    DocumentRoot /var/www/html\n\n    <Directory /var/www/html>\n        Options Indexes FollowSymLinks\n        AllowOverride All\n        Require all granted\n    </Directory>\n\n    ErrorLog ${APACHE_LOG_DIR}/error.log\n    CustomLog ${APACHE_LOG_DIR}/access.log combined\n</VirtualHost>\n";
        assert_idempotent(src, "default_vhost");
    }

    #[test]
    fn round_trip_ssl_vhost() {
        let src = "<VirtualHost *:443>\n    ServerName example.com\n    DocumentRoot /var/www/html\n\n    SSLEngine on\n    SSLCertificateFile /etc/letsencrypt/live/example.com/fullchain.pem\n    SSLCertificateKeyFile /etc/letsencrypt/live/example.com/privkey.pem\n    SSLProtocol all -SSLv3 -TLSv1 -TLSv1.1\n    SSLHonorCipherOrder off\n</VirtualHost>\n";
        assert_idempotent(src, "ssl_vhost");
    }

    #[test]
    fn round_trip_proxy_vhost() {
        let src = "<VirtualHost *:80>\n    ServerName api.example.com\n    ProxyPreserveHost on\n    ProxyPass / http://127.0.0.1:8080/\n    ProxyPassReverse / http://127.0.0.1:8080/\n</VirtualHost>\n";
        assert_idempotent(src, "proxy_vhost");
    }

    #[test]
    fn round_trip_ifmodule_listen() {
        let src = "<IfModule mod_ssl.c>\n    Listen 443\n</IfModule>\n\n<IfModule mod_gnutls.c>\n    Listen 443\n</IfModule>\n";
        assert_idempotent(src, "ifmodule_listen");
    }

    #[test]
    fn round_trip_basic_auth_location() {
        let src = "<VirtualHost *:80>\n    ServerName secure.example.com\n    DocumentRoot /var/www/secure\n\n    <Location />\n        AuthType Basic\n        AuthName \"Restricted\"\n        AuthUserFile /etc/apache2/.htpasswd\n        Require valid-user\n    </Location>\n</VirtualHost>\n";
        assert_idempotent(src, "basic_auth_location");
    }

    #[test]
    fn round_trip_multiple_vhosts() {
        let src = "<VirtualHost *:80>\n    ServerName a.example.com\n    DocumentRoot /var/www/a\n</VirtualHost>\n\n<VirtualHost *:80>\n    ServerName b.example.com\n    DocumentRoot /var/www/b\n</VirtualHost>\n";
        assert_idempotent(src, "multiple_vhosts");
    }

    /// Canonical text the New-site wizard emits for Apache without
    /// SSL. Parser must accept it and key directives must detect.
    #[test]
    fn wizard_output_apache_plain() {
        let src = "<VirtualHost *:80>\n    ServerName example.com\n    ServerAlias www.example.com\n    DocumentRoot /var/www/html\n\n    <Directory /var/www/html>\n        Options Indexes FollowSymLinks\n        AllowOverride All\n        Require all granted\n    </Directory>\n\n    ErrorLog ${APACHE_LOG_DIR}/example.com-error.log\n    CustomLog ${APACHE_LOG_DIR}/example.com-access.log combined\n</VirtualHost>\n";
        let r = parse(src);
        assert!(
            r.errors.is_empty(),
            "wizard apache plain had parse errors: {:?}",
            r.errors
        );
        let vhost = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(vhost.name, "VirtualHost");
        assert_eq!(vhost.args, vec!["*:80".to_string()]);
        let body = vhost.section.as_ref().unwrap();
        let names: Vec<&str> = body
            .iter()
            .filter_map(|n| match n {
                ApacheNode::Directive(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"ServerName"));
        assert!(names.contains(&"ServerAlias"));
        assert!(names.contains(&"DocumentRoot"));
        assert!(names.contains(&"Directory"));
        assert!(names.contains(&"ErrorLog"));
        assert!(names.contains(&"CustomLog"));
        assert_idempotent(src, "wizard_apache_plain");
    }

    /// Canonical text the New-site wizard emits for Apache with
    /// SSL turned on. Validates the SSL block placement.
    #[test]
    fn wizard_output_apache_ssl() {
        let src = "<VirtualHost *:443>\n    ServerName example.com\n    ServerAlias www.example.com\n    DocumentRoot /var/www/html\n\n    <Directory /var/www/html>\n        Options Indexes FollowSymLinks\n        AllowOverride All\n        Require all granted\n    </Directory>\n\n    SSLEngine on\n    SSLCertificateFile /etc/letsencrypt/live/example.com/fullchain.pem\n    SSLCertificateKeyFile /etc/letsencrypt/live/example.com/privkey.pem\n    SSLProtocol all -SSLv3 -TLSv1 -TLSv1.1\n    SSLHonorCipherOrder off\n\n    ErrorLog ${APACHE_LOG_DIR}/example.com-error.log\n    CustomLog ${APACHE_LOG_DIR}/example.com-access.log combined\n</VirtualHost>\n";
        let r = parse(src);
        assert!(
            r.errors.is_empty(),
            "wizard apache SSL had parse errors: {:?}",
            r.errors
        );
        let vhost = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(vhost.args, vec!["*:443".to_string()]);
        let body = vhost.section.as_ref().unwrap();
        let names: Vec<&str> = body
            .iter()
            .filter_map(|n| match n {
                ApacheNode::Directive(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"SSLEngine"));
        assert!(names.contains(&"SSLCertificateFile"));
        assert!(names.contains(&"SSLCertificateKeyFile"));
        assert!(names.contains(&"SSLProtocol"));
        assert_idempotent(src, "wizard_apache_ssl");
    }

    /// Wizard variant: no ServerAlias provided, port 80.
    #[test]
    fn wizard_output_apache_no_alias() {
        let src = "<VirtualHost *:80>\n    ServerName example.com\n    DocumentRoot /var/www/html\n\n    <Directory /var/www/html>\n        Options Indexes FollowSymLinks\n        AllowOverride All\n        Require all granted\n    </Directory>\n\n    ErrorLog ${APACHE_LOG_DIR}/example.com-error.log\n    CustomLog ${APACHE_LOG_DIR}/example.com-access.log combined\n</VirtualHost>\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        let vhost = match &r.nodes[0] {
            ApacheNode::Directive(d) => d,
            _ => panic!(),
        };
        let body = vhost.section.as_ref().unwrap();
        let names: Vec<&str> = body
            .iter()
            .filter_map(|n| match n {
                ApacheNode::Directive(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();
        // Without alias the wizard omits the ServerAlias line.
        assert!(!names.contains(&"ServerAlias"));
        assert_idempotent(src, "wizard_apache_no_alias");
    }
}
