//! Caddyfile parser + renderer.
//!
//! Caddyfile syntax is small and line-oriented:
//!   * tokens: word, double-quoted string, backtick-quoted string,
//!     `{`, `}`, `# comment`, newline
//!   * statement terminator: newline (no semicolons)
//!   * `\<newline>` continues a logical line
//!   * top-level: site blocks (`addr1, addr2 { ... }`), snippets
//!     (`(name) { ... }`), or a single global block (`{ ... }` with
//!     no preceding addresses)
//!   * matchers (`@name`) and snippet refs are just words
//!
//! The AST mirrors `services::nginx`'s shape so the frontend can render
//! it with the same node-tree primitive: each statement is a Directive
//! with a name, args, and an optional block. A site block keeps its
//! first address as the directive name and any extra addresses as args.
//! A global block has an empty name. A snippet has the parenthesised
//! name as its directive name (e.g. `"(common)"`).
//!
//! Heredocs (`<<DELIM ... DELIM`, Caddy 2.5+) and unusual constructs
//! are reported as warnings; the parser falls back to capturing the
//! rest of the line / file as raw text so round-trip stays loss-free
//! for the common case.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaddyParseResult {
    pub nodes: Vec<CaddyNode>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CaddyNode {
    Directive(CaddyDirective),
    Comment {
        /// `#` and indentation excluded.
        text: String,
        /// Blank lines that preceded this comment.
        leading_blanks: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaddyDirective {
    /// Directive name. For a site block, the first address (e.g.
    /// `example.com`). For a snippet, the parenthesised form
    /// (e.g. `(common)`). For a global block, an empty string.
    pub name: String,
    /// Positional arguments after the name. Quoted args keep their
    /// quoting in the string (e.g. `"\"hello world\""`) so the
    /// renderer can emit the same style.
    pub args: Vec<String>,
    /// Comments that immediately precede this directive.
    pub leading_comments: Vec<String>,
    /// Blank lines between the previous sibling and this directive.
    pub leading_blanks: u32,
    /// Trailing same-line comment, after the directive content but
    /// before the line terminator.
    pub inline_comment: Option<String>,
    /// `Some` when this directive opens a block.
    pub block: Option<Vec<CaddyNode>>,
}

pub fn parse(src: &str) -> CaddyParseResult {
    let mut lex = Lexer::new(src);
    let mut errors = Vec::new();
    let nodes = parse_block(&mut lex, false, &mut errors);
    CaddyParseResult { nodes, errors }
}

pub fn render(nodes: &[CaddyNode]) -> String {
    let mut out = String::new();
    render_nodes(nodes, 0, &mut out);
    out
}

// ── Lexer ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    /// Quoted argument; the second field tracks the quote style so the
    /// renderer can preserve it.
    Quoted {
        text: String,
        style: QuoteStyle,
    },
    BraceOpen,
    BraceClose,
    Comment(String),
    Newline,
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteStyle {
    Double,
    Backtick,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    /// Consume `\r`, space, tab. Newlines are tokens. Also handles the
    /// `\<newline>` line-continuation by silently consuming both bytes.
    fn skip_inline_ws(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => {
                    self.pos += 1;
                }
                Some(b'\\') if self.src.get(self.pos + 1) == Some(&b'\n') => {
                    self.pos += 2;
                }
                _ => break,
            }
        }
    }

    fn next_tok(&mut self) -> Tok {
        self.skip_inline_ws();
        let Some(c) = self.peek() else {
            return Tok::Eof;
        };
        match c {
            b'\n' => {
                self.pos += 1;
                Tok::Newline
            }
            b'{' => {
                // Disambiguate block opener vs `{placeholder}` arg.
                // A standalone `{` (followed by whitespace / EOL / EOF)
                // opens a block; a `{name}` / `{name.sub}` shape with
                // no whitespace before its closing `}` is a Caddy
                // placeholder and should be emitted as a Word so the
                // parser keeps it on the directive's arg list.
                let mut k = self.pos + 1;
                let mut found_close = false;
                while k < self.src.len() {
                    let cc = self.src[k];
                    if cc == b'}' {
                        found_close = true;
                        break;
                    }
                    if matches!(cc, b' ' | b'\t' | b'\n' | b'\r' | b'{') {
                        break;
                    }
                    k += 1;
                }
                if found_close && k > self.pos + 1 {
                    // Capture `{...}` as a Word. Continue past `}`
                    // and let the bare-word path collect any trailing
                    // characters (e.g. `{host}.suffix`).
                    let start = self.pos;
                    self.pos = k + 1;
                    while let Some(c) = self.peek() {
                        if matches!(
                            c,
                            b' ' | b'\t' | b'\r' | b'\n' | b'}' | b'#' | b'"' | b'`'
                        ) {
                            break;
                        }
                        if c == b'{' {
                            // Another inline placeholder — recurse
                            // by re-entering the bare-word inline
                            // placeholder logic below. For now just
                            // include it.
                            let mut k2 = self.pos + 1;
                            let mut found_close2 = false;
                            while k2 < self.src.len() {
                                let cc = self.src[k2];
                                if cc == b'}' {
                                    found_close2 = true;
                                    break;
                                }
                                if matches!(cc, b' ' | b'\t' | b'\n' | b'\r' | b'{') {
                                    break;
                                }
                                k2 += 1;
                            }
                            if found_close2 {
                                self.pos = k2 + 1;
                                continue;
                            }
                            break;
                        }
                        self.pos += 1;
                    }
                    let raw = &self.src[start..self.pos];
                    return Tok::Word(String::from_utf8_lossy(raw).into_owned());
                }
                self.pos += 1;
                Tok::BraceOpen
            }
            b'}' => {
                self.pos += 1;
                Tok::BraceClose
            }
            b'#' => {
                self.pos += 1;
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
                let raw = &self.src[start..self.pos];
                Tok::Comment(String::from_utf8_lossy(raw).trim().to_string())
            }
            b'"' => {
                self.pos += 1;
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == b'\\' {
                        // Caddy honors `\"` and `\\` escapes inside
                        // double-quoted strings. Preserve the escape
                        // verbatim so render emits the same source.
                        self.pos += 1;
                        if let Some(next) = self.bump() {
                            text.push('\\');
                            text.push(next as char);
                        }
                        continue;
                    }
                    if c == b'"' {
                        self.pos += 1;
                        return Tok::Quoted {
                            text,
                            style: QuoteStyle::Double,
                        };
                    }
                    if c == b'\n' {
                        // Unterminated double-quoted string. Bail.
                        break;
                    }
                    self.pos += 1;
                    text.push(c as char);
                }
                Tok::Quoted {
                    text,
                    style: QuoteStyle::Double,
                }
            }
            b'`' => {
                // Backtick strings are literal — no escapes, may span
                // multiple lines.
                self.pos += 1;
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    if c == b'`' {
                        self.pos += 1;
                        return Tok::Quoted {
                            text,
                            style: QuoteStyle::Backtick,
                        };
                    }
                    self.pos += 1;
                    text.push(c as char);
                }
                Tok::Quoted {
                    text,
                    style: QuoteStyle::Backtick,
                }
            }
            _ => {
                // Bare word — anything until whitespace, brace, comment,
                // or quote. `(` / `)` are word-internal so snippet refs
                // like `(common)` lex as a single token.
                //
                // `{name}` / `{name.sub}` placeholders are also
                // word-internal: when we see `{` inside a word and a
                // matching `}` appears before any whitespace or
                // nested `{`, we consume the whole placeholder span
                // as part of the word. Otherwise (`{` at the end of
                // a logical line) the brace is a block opener.
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if matches!(
                        c,
                        b' ' | b'\t' | b'\r' | b'\n' | b'}' | b'#' | b'"' | b'`'
                    ) {
                        break;
                    }
                    if c == b'{' {
                        // Look ahead for a placeholder shape.
                        let mut k = self.pos + 1;
                        let mut found_close = false;
                        while k < self.src.len() {
                            let cc = self.src[k];
                            if cc == b'}' {
                                found_close = true;
                                break;
                            }
                            if matches!(cc, b' ' | b'\t' | b'\n' | b'\r' | b'{') {
                                break;
                            }
                            k += 1;
                        }
                        if found_close {
                            // Consume `{...}` as part of the word.
                            self.pos = k + 1;
                            continue;
                        }
                        // Standalone `{` — break so the parser sees
                        // a block opener.
                        break;
                    }
                    if c == b'\\' && self.src.get(self.pos + 1) == Some(&b'\n') {
                        // Line continuation — terminates the current
                        // bare word.
                        break;
                    }
                    self.pos += 1;
                }
                let raw = &self.src[start..self.pos];
                Tok::Word(String::from_utf8_lossy(raw).into_owned())
            }
        }
    }
}

// ── Parser core ─────────────────────────────────────────────────────

fn parse_block(
    lex: &mut Lexer<'_>,
    stop_on_close: bool,
    errors: &mut Vec<String>,
) -> Vec<CaddyNode> {
    let mut out: Vec<CaddyNode> = Vec::new();
    let mut pending_blanks: u32 = 0;
    let mut pending_comments: Vec<String> = Vec::new();

    loop {
        let tok = lex.next_tok();
        match tok {
            Tok::Eof => {
                if stop_on_close {
                    errors.push("unexpected end of file (missing `}`)".into());
                }
                drain_pending(&mut out, &mut pending_comments);
                return out;
            }
            Tok::BraceClose => {
                if !stop_on_close {
                    errors.push("stray `}` at top level".into());
                    continue;
                }
                drain_pending(&mut out, &mut pending_comments);
                return out;
            }
            Tok::Newline => {
                pending_blanks = pending_blanks.saturating_add(1);
            }
            Tok::BraceOpen => {
                // A bare `{` starting a block — this is the
                // top-of-file global options block. Treat as a
                // directive with empty name.
                let block = parse_block(lex, true, errors);
                out.push(CaddyNode::Directive(CaddyDirective {
                    name: String::new(),
                    args: Vec::new(),
                    leading_comments: std::mem::take(&mut pending_comments),
                    leading_blanks: pending_blanks.saturating_sub(1),
                    inline_comment: None,
                    block: Some(block),
                }));
                pending_blanks = 0;
            }
            Tok::Comment(text) => {
                if pending_blanks >= 2 && !pending_comments.is_empty() {
                    let blanks_for_first = pending_blanks.saturating_sub(1);
                    let mut first = true;
                    for c in pending_comments.drain(..) {
                        out.push(CaddyNode::Comment {
                            text: c,
                            leading_blanks: if first { blanks_for_first } else { 0 },
                        });
                        first = false;
                    }
                }
                pending_comments.push(text);
                pending_blanks = 0;
            }
            Tok::Word(directive_name) => {
                if pending_blanks >= 2 && !pending_comments.is_empty() {
                    for c in pending_comments.drain(..) {
                        out.push(CaddyNode::Comment {
                            text: c,
                            leading_blanks: 0,
                        });
                    }
                }
                let mut args: Vec<String> = Vec::new();
                let mut block: Option<Vec<CaddyNode>> = None;
                let mut inline_comment: Option<String> = None;

                loop {
                    let t = lex.next_tok();
                    match t {
                        Tok::Word(w) => {
                            if let Some(stripped) = w.strip_prefix("<<") {
                                if !stripped.is_empty() {
                                    // Heredoc syntax. Capture the body
                                    // verbatim; round-trip via render
                                    // emits it back as a single arg.
                                    let body = read_heredoc_body(lex, stripped, errors);
                                    args.push(format!("<<{stripped}\n{body}{stripped}"));
                                    continue;
                                }
                            }
                            args.push(w);
                        }
                        Tok::Quoted { text, style } => {
                            let mut s = String::with_capacity(text.len() + 2);
                            let q = match style {
                                QuoteStyle::Double => '"',
                                QuoteStyle::Backtick => '`',
                            };
                            s.push(q);
                            s.push_str(&text);
                            s.push(q);
                            args.push(s);
                        }
                        Tok::Comment(c) => {
                            // Comment on the same line as the directive
                            // — promote to inline.
                            inline_comment = Some(c);
                            // Caddy line ends at newline AFTER the comment,
                            // so consume up to and including the newline.
                            // The lexer's next call will see the newline.
                            break;
                        }
                        Tok::Newline => {
                            break;
                        }
                        Tok::BraceOpen => {
                            block = Some(parse_block(lex, true, errors));
                            // After `}` look for an inline comment
                            // before the next newline.
                            lex.skip_inline_ws();
                            if lex.peek() == Some(b'#') {
                                lex.pos += 1;
                                let start = lex.pos;
                                while let Some(c) = lex.peek() {
                                    if c == b'\n' {
                                        break;
                                    }
                                    lex.pos += 1;
                                }
                                let raw = &lex.src[start..lex.pos];
                                inline_comment = Some(
                                    String::from_utf8_lossy(raw).trim().to_string(),
                                );
                            }
                            break;
                        }
                        Tok::BraceClose => {
                            // Caddy doesn't normally close a block on
                            // the same line as a directive. Treat as a
                            // recoverable error and back up so the
                            // outer parser sees the close.
                            errors.push(format!(
                                "directive `{directive_name}` ended with `}}` instead of newline",
                            ));
                            // Push back is awkward; instead the outer
                            // call will not see this `}`. So flag and
                            // emit anyway.
                            break;
                        }
                        Tok::Eof => {
                            // Last directive in file with no trailing
                            // newline — that's fine.
                            break;
                        }
                    }
                }

                out.push(CaddyNode::Directive(CaddyDirective {
                    name: directive_name,
                    args,
                    leading_comments: std::mem::take(&mut pending_comments),
                    leading_blanks: pending_blanks.saturating_sub(1),
                    inline_comment,
                    block,
                }));
                pending_blanks = 0;
            }
            Tok::Quoted { text, .. } => {
                // A quoted string at directive-head position is unusual
                // but legal — site addresses could be quoted. Treat as
                // a directive name without quotes (we lose the quoting
                // on round-trip; document rather than fight).
                let mut args: Vec<String> = Vec::new();
                let mut block: Option<Vec<CaddyNode>> = None;
                let mut inline_comment: Option<String> = None;
                loop {
                    let t = lex.next_tok();
                    match t {
                        Tok::Word(w) => args.push(w),
                        Tok::Quoted { text, style } => {
                            let q = match style {
                                QuoteStyle::Double => '"',
                                QuoteStyle::Backtick => '`',
                            };
                            args.push(format!("{q}{text}{q}"));
                        }
                        Tok::Comment(c) => {
                            inline_comment = Some(c);
                            break;
                        }
                        Tok::Newline | Tok::Eof => break,
                        Tok::BraceOpen => {
                            block = Some(parse_block(lex, true, errors));
                            break;
                        }
                        Tok::BraceClose => break,
                    }
                }
                out.push(CaddyNode::Directive(CaddyDirective {
                    name: text,
                    args,
                    leading_comments: std::mem::take(&mut pending_comments),
                    leading_blanks: pending_blanks.saturating_sub(1),
                    inline_comment,
                    block,
                }));
                pending_blanks = 0;
            }
        }
    }
}

fn drain_pending(out: &mut Vec<CaddyNode>, pending: &mut Vec<String>) {
    for c in pending.drain(..) {
        out.push(CaddyNode::Comment {
            text: c,
            leading_blanks: 0,
        });
    }
}

/// Read a heredoc body. The opening `<<DELIM` was just consumed; the
/// body extends from the next line up to (and excluding) a line whose
/// content is exactly `DELIM` (with leading whitespace stripped).
fn read_heredoc_body(lex: &mut Lexer<'_>, delim: &str, errors: &mut Vec<String>) -> String {
    // Skip to the next newline first.
    while let Some(c) = lex.peek() {
        if c == b'\n' {
            lex.pos += 1;
            break;
        }
        lex.pos += 1;
    }
    let mut body = String::new();
    loop {
        let line_start = lex.pos;
        while let Some(c) = lex.peek() {
            if c == b'\n' {
                break;
            }
            lex.pos += 1;
        }
        let raw = &lex.src[line_start..lex.pos];
        let line = String::from_utf8_lossy(raw);
        if line.trim() == delim {
            // Consume the trailing newline (if any) and return.
            if lex.peek() == Some(b'\n') {
                lex.pos += 1;
            }
            return body;
        }
        body.push_str(&line);
        body.push('\n');
        if lex.peek().is_none() {
            errors.push(format!(
                "heredoc not terminated (expected `{delim}`)"
            ));
            return body;
        }
        lex.pos += 1; // consume newline
    }
}

// ── Renderer ────────────────────────────────────────────────────────

fn render_nodes(nodes: &[CaddyNode], depth: usize, out: &mut String) {
    for (i, node) in nodes.iter().enumerate() {
        let leading_blanks = match node {
            CaddyNode::Comment { leading_blanks, .. } => *leading_blanks,
            CaddyNode::Directive(d) => d.leading_blanks,
        };
        // Cap blank-line count at 2 (one visual blank line) and skip
        // the leading blanks for the very first node.
        if i > 0 {
            let n = leading_blanks.min(2);
            for _ in 0..n {
                out.push('\n');
            }
        }
        match node {
            CaddyNode::Comment { text, .. } => {
                indent(out, depth);
                out.push('#');
                if !text.is_empty() {
                    out.push(' ');
                    out.push_str(text);
                }
                out.push('\n');
            }
            CaddyNode::Directive(d) => render_directive(d, depth, out),
        }
    }
}

fn render_directive(d: &CaddyDirective, depth: usize, out: &mut String) {
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
    if !d.name.is_empty() {
        out.push_str(&d.name);
    }
    for arg in &d.args {
        if !d.name.is_empty() || !out.ends_with('\t') {
            // We always want a space between the name and the first
            // arg, and between subsequent args. The `name.is_empty()`
            // case (global block) renders no leading name, so the
            // first arg goes flush.
            if !out.ends_with(' ') && !out.ends_with('\t') && !d.name.is_empty() {
                out.push(' ');
            } else if d.name.is_empty() && !out.ends_with(' ') {
                // No-op: empty name means we skip the space.
            }
        }
        if !out.ends_with(' ') && !d.name.is_empty() {
            out.push(' ');
        }
        out.push_str(arg);
    }
    if let Some(block) = &d.block {
        if !d.name.is_empty() || !d.args.is_empty() {
            out.push(' ');
        }
        out.push('{');
        if let Some(c) = &d.inline_comment {
            out.push_str(" #");
            if !c.is_empty() {
                out.push(' ');
                out.push_str(c);
            }
        }
        out.push('\n');
        render_nodes(block, depth + 1, out);
        indent(out, depth);
        out.push('}');
        out.push('\n');
    } else {
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

    fn roundtrip(src: &str) -> String {
        let r = parse(src);
        assert!(r.errors.is_empty(), "parse errors: {:?}", r.errors);
        render(&r.nodes)
    }

    #[test]
    fn simple_site() {
        let src = "example.com {\n    reverse_proxy 127.0.0.1:8080\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("example.com {"));
        assert!(out.contains("reverse_proxy 127.0.0.1:8080"));
    }

    #[test]
    fn comment_preserved() {
        let src = "# top comment\nexample.com {\n    file_server\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("# top comment"));
    }

    #[test]
    fn snippet() {
        let src = "(common) {\n    encode gzip\n}\nexample.com {\n    import common\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("(common) {"));
        assert!(out.contains("import common"));
    }

    #[test]
    fn nested_block() {
        let src = "example.com {\n    log {\n        output file /var/log/caddy.log\n    }\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("log {"));
        assert!(out.contains("output file /var/log/caddy.log"));
    }

    #[test]
    fn quoted_args() {
        let src = "example.com {\n    respond \"hello world\"\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("\"hello world\""));
    }

    #[test]
    fn line_continuation_collapses() {
        // `\<newline>` joins logical args.
        let src = "example.com {\n    reverse_proxy \\\n        127.0.0.1:8080\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        // The reverse_proxy directive should have one arg (the
        // upstream), even though the source split it across lines.
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        let body = site.block.as_ref().unwrap();
        let rp = match &body[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(rp.name, "reverse_proxy");
        assert_eq!(rp.args, vec!["127.0.0.1:8080".to_string()]);
    }

    #[test]
    fn deep_nesting() {
        // log block inside reverse_proxy block inside site.
        let src = "example.com {\n    reverse_proxy localhost:8080 {\n        header_up Host {host}\n        transport http {\n            tls\n        }\n    }\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let out = render(&r.nodes);
        assert!(out.contains("transport http {"));
        assert!(out.contains("        tls"));
    }

    #[test]
    fn empty_block_round_trips() {
        let src = "example.com {\n}\n";
        let out = roundtrip(src);
        assert!(out.contains("example.com {"));
        assert!(out.contains("}"));
    }

    #[test]
    fn multi_address_site() {
        // First address becomes name, rest are args.
        let src = "example.com www.example.com:443 {\n    file_server\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(site.name, "example.com");
        assert_eq!(
            site.args,
            vec!["www.example.com:443".to_string()],
        );
    }

    #[test]
    fn snippet_with_import() {
        let src =
            "(common) {\n    encode gzip\n    log\n}\n\nexample.com {\n    import common\n    file_server\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        // Two top-level directives: snippet + site.
        assert_eq!(r.nodes.len(), 2);
        let snippet = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(snippet.name, "(common)");
    }

    #[test]
    fn global_options_block() {
        // Top-of-file `{ ... }` with no preceding addresses.
        let src = "{\n    email admin@example.com\n    debug\n}\n\nexample.com {\n    file_server\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        let global = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(global.name, "");
        assert!(global.block.is_some());
        let body = global.block.as_ref().unwrap();
        assert_eq!(body.len(), 2); // email + debug
    }

    #[test]
    fn matcher_in_site() {
        // Matchers like `@api` are just words.
        let src = "example.com {\n    @api path /api/*\n    handle @api {\n        reverse_proxy backend:8080\n    }\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let out = render(&r.nodes);
        assert!(out.contains("@api path /api/*"));
        assert!(out.contains("handle @api {"));
    }

    #[test]
    fn comment_inside_block() {
        let src =
            "example.com {\n    # serve static\n    file_server\n    # gzip everything\n    encode gzip\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        let out = render(&r.nodes);
        assert!(out.contains("# serve static"));
        assert!(out.contains("# gzip everything"));
    }

    #[test]
    fn backtick_string_multiline() {
        // Backtick strings can span newlines without escapes.
        let src =
            "example.com {\n    respond `hello\nworld`\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        let respond = match &site.block.as_ref().unwrap()[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        // Arg keeps its backticks for round-trip.
        assert_eq!(respond.args.len(), 1);
        assert!(respond.args[0].starts_with('`'));
        assert!(respond.args[0].ends_with('`'));
    }

    #[test]
    fn unterminated_brace_recovers() {
        // Missing close — parser should report an error but not panic.
        let src = "example.com {\n    file_server\n";
        let r = parse(src);
        assert!(!r.errors.is_empty());
        // Even with the error, we should have at least the site
        // node populated as far as we got.
        assert!(!r.nodes.is_empty());
    }

    #[test]
    fn double_quote_with_escape() {
        let src =
            "example.com {\n    respond \"she said \\\"hi\\\"\"\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        let out = render(&r.nodes);
        // The escapes should round-trip verbatim.
        assert!(out.contains("\\\"hi\\\""));
    }

    /// For each realistic config, parse → render → parse again and
    /// assert AST equality. Catches asymmetric parser-renderer bugs:
    /// any drift between the two passes indicates the renderer
    /// produced something the parser interprets differently.
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
    fn round_trip_minimal_site() {
        assert_idempotent(
            "example.com {\n    respond \"OK\"\n}\n",
            "minimal_site",
        );
    }

    #[test]
    fn round_trip_reverse_proxy_with_block() {
        let src = "example.com {\n    reverse_proxy localhost:8080 {\n        header_up Host {host}\n        header_up X-Real-IP {remote_host}\n    }\n}\n";
        assert_idempotent(src, "reverse_proxy_with_block");
    }

    #[test]
    fn round_trip_static_site() {
        let src = "example.com {\n    root * /var/www/html\n    file_server\n    encode gzip zstd\n    log {\n        output file /var/log/caddy/access.log\n    }\n}\n";
        assert_idempotent(src, "static_site");
    }

    #[test]
    fn round_trip_multi_site_with_snippet() {
        let src = "(common-headers) {\n    header X-Frame-Options SAMEORIGIN\n    header X-Content-Type-Options nosniff\n}\n\nexample.com {\n    import common-headers\n    reverse_proxy backend1:8080\n}\n\napi.example.com {\n    import common-headers\n    reverse_proxy backend2:8080\n}\n";
        assert_idempotent(src, "multi_site_with_snippet");
    }

    #[test]
    fn round_trip_global_block() {
        let src = "{\n    email admin@example.com\n    debug\n}\n\nexample.com {\n    file_server\n}\n";
        assert_idempotent(src, "global_block");
    }

    #[test]
    fn round_trip_matchers_and_handle() {
        let src = "example.com {\n    @api path /api/*\n    handle @api {\n        reverse_proxy backend:8080\n    }\n    handle {\n        file_server\n    }\n}\n";
        assert_idempotent(src, "matchers_and_handle");
    }

    /// Canonical text the New-site wizard emits for Caddy's
    /// reverse-proxy mode. Parser must accept it without errors and
    /// the directives must round-trip cleanly.
    #[test]
    fn wizard_output_reverse_proxy() {
        let src = "example.com {\n    reverse_proxy 127.0.0.1:8080\n    encode gzip zstd\n    log {\n        output file /var/log/caddy/access.log\n    }\n}\n";
        let r = parse(src);
        assert!(
            r.errors.is_empty(),
            "wizard reverse-proxy output had parse errors: {:?}",
            r.errors
        );
        // Detect that the key directives parsed correctly.
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(site.name, "example.com");
        let body = site.block.as_ref().unwrap();
        let names: Vec<&str> = body
            .iter()
            .filter_map(|n| match n {
                CaddyNode::Directive(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"reverse_proxy"));
        assert!(names.contains(&"encode"));
        assert!(names.contains(&"log"));
        assert_idempotent(src, "wizard_reverse_proxy");
    }

    /// Canonical text the New-site wizard emits for Caddy's
    /// static-file-server mode.
    #[test]
    fn wizard_output_static() {
        let src = "example.com {\n    root * /var/www/html\n    file_server\n    encode gzip zstd\n    log {\n        output file /var/log/caddy/access.log\n    }\n}\n";
        let r = parse(src);
        assert!(
            r.errors.is_empty(),
            "wizard static output had parse errors: {:?}",
            r.errors
        );
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        let body = site.block.as_ref().unwrap();
        let names: Vec<&str> = body
            .iter()
            .filter_map(|n| match n {
                CaddyNode::Directive(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"root"));
        assert!(names.contains(&"file_server"));
        assert_idempotent(src, "wizard_static");
    }

    /// Caddy address with port (`:443`).
    #[test]
    fn wizard_output_port_only_address() {
        let src = ":443 {\n    file_server\n}\n";
        let r = parse(src);
        assert!(r.errors.is_empty());
        let site = match &r.nodes[0] {
            CaddyNode::Directive(d) => d,
            _ => panic!(),
        };
        assert_eq!(site.name, ":443");
    }
}
