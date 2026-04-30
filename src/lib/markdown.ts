// Minimal Markdown→HTML renderer for the SFTP file viewer's "Render"
// mode. Pulls in no dependencies — Pier-X already ships CodeMirror for
// editing, and a full-fat lib like `marked` would dwarf this single
// caller. Coverage is the common subset:
//   ATX headings (#..######), thematic break (--- / ***)
//   fenced code blocks (``` / ~~~ with optional info string)
//   blockquotes (> ...), unordered (-, *, +) and ordered (1.) lists
//   inline: `code`, **bold**, *italic*, [text](url), <url>, hard br (two spaces)
//   inline images: ![alt](url) — rendered as <img> with rel-protected loading
// Anything outside this surface degrades to escaped plain text.
//
// Output is concatenated and trusted only inside a sandboxed render
// container the caller controls. We escape every untrusted leaf with
// `escapeHtml` before composing tags, and never let user-provided HTML
// pass through verbatim.

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function escapeAttr(s: string): string {
  return escapeHtml(s);
}

// Allow http(s), mailto, and relative refs. Reject everything else
// (javascript:, data:, file:) so a malicious .md can't smuggle script
// execution through a link click.
function safeUrl(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) return "#";
  if (/^(https?:|mailto:|#|\/|\.\/|\.\.\/)/i.test(trimmed)) return trimmed;
  if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) return "#";
  return trimmed;
}

function renderInline(src: string): string {
  // Tokenize step by step against the source so we can emit raw HTML
  // for the recognised constructs and escape everything else. We walk
  // a single index forward instead of running a regex pipeline so the
  // output respects the document order of overlapping markers.
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];

    // Hard line break — two spaces at end of line.
    if (ch === "\n") {
      out += "<br/>\n";
      i++;
      continue;
    }

    // Inline code: `code` (no nesting, no escapes).
    if (ch === "`") {
      const end = src.indexOf("`", i + 1);
      if (end > i) {
        out += `<code>${escapeHtml(src.slice(i + 1, end))}</code>`;
        i = end + 1;
        continue;
      }
    }

    // Image: ![alt](url)
    if (ch === "!" && src[i + 1] === "[") {
      const close = src.indexOf("]", i + 2);
      if (close > 0 && src[close + 1] === "(") {
        const urlEnd = src.indexOf(")", close + 2);
        if (urlEnd > 0) {
          const alt = src.slice(i + 2, close);
          const url = src.slice(close + 2, urlEnd);
          out += `<img alt="${escapeAttr(alt)}" src="${escapeAttr(safeUrl(url))}" loading="lazy"/>`;
          i = urlEnd + 1;
          continue;
        }
      }
    }

    // Link: [text](url)
    if (ch === "[") {
      const close = src.indexOf("]", i + 1);
      if (close > 0 && src[close + 1] === "(") {
        const urlEnd = src.indexOf(")", close + 2);
        if (urlEnd > 0) {
          const text = src.slice(i + 1, close);
          const url = src.slice(close + 2, urlEnd);
          out += `<a href="${escapeAttr(safeUrl(url))}" rel="noopener noreferrer" target="_blank">${renderInline(text)}</a>`;
          i = urlEnd + 1;
          continue;
        }
      }
    }

    // Autolink: <https://...> / <user@host>
    if (ch === "<") {
      const close = src.indexOf(">", i + 1);
      if (close > 0) {
        const inner = src.slice(i + 1, close);
        if (/^https?:\/\//i.test(inner)) {
          out += `<a href="${escapeAttr(inner)}" rel="noopener noreferrer" target="_blank">${escapeHtml(inner)}</a>`;
          i = close + 1;
          continue;
        }
        if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(inner)) {
          out += `<a href="mailto:${escapeAttr(inner)}">${escapeHtml(inner)}</a>`;
          i = close + 1;
          continue;
        }
      }
    }

    // Bold: **text**
    if (ch === "*" && src[i + 1] === "*") {
      const end = src.indexOf("**", i + 2);
      if (end > i + 2) {
        out += `<strong>${renderInline(src.slice(i + 2, end))}</strong>`;
        i = end + 2;
        continue;
      }
    }

    // Italic: *text* (skip if followed by another *)
    if (ch === "*" && src[i + 1] !== "*") {
      const end = src.indexOf("*", i + 1);
      if (end > i + 1) {
        out += `<em>${renderInline(src.slice(i + 1, end))}</em>`;
        i = end + 1;
        continue;
      }
    }

    // Backslash escape — emit the next character as a literal.
    if (ch === "\\" && i + 1 < n) {
      out += escapeHtml(src[i + 1]);
      i += 2;
      continue;
    }

    out += escapeHtml(ch);
    i++;
  }
  return out;
}

export function renderMarkdown(src: string): string {
  // Normalize line endings; trim trailing whitespace per-line so
  // "two-space hard break" still works without breaking equality.
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  let out = "";
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code: ``` or ~~~ with optional info string.
    const fenceMatch = /^([`~]{3,})\s*(.*)$/.exec(line);
    if (fenceMatch) {
      const fence = fenceMatch[1];
      const lang = fenceMatch[2].trim();
      const buf: string[] = [];
      i++;
      while (i < lines.length) {
        const ln = lines[i];
        if (ln.startsWith(fence[0].repeat(fence.length)) && /^[`~]+\s*$/.test(ln)) {
          i++;
          break;
        }
        buf.push(ln);
        i++;
      }
      const cls = lang ? ` class="language-${escapeAttr(lang)}"` : "";
      out += `<pre><code${cls}>${escapeHtml(buf.join("\n"))}</code></pre>`;
      continue;
    }

    // Blank line — flushes paragraphs.
    if (/^\s*$/.test(line)) {
      i++;
      continue;
    }

    // Thematic break.
    if (/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      out += "<hr/>";
      i++;
      continue;
    }

    // ATX heading.
    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading) {
      const level = heading[1].length;
      out += `<h${level}>${renderInline(heading[2])}</h${level}>`;
      i++;
      continue;
    }

    // Blockquote — fold consecutive ">" lines.
    if (/^\s*>\s?/.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      out += `<blockquote>${renderMarkdown(buf.join("\n"))}</blockquote>`;
      continue;
    }

    // Unordered list — -, *, + at start.
    if (/^\s*[-*+]\s+/.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) {
        buf.push(`<li>${renderInline(lines[i].replace(/^\s*[-*+]\s+/, ""))}</li>`);
        i++;
      }
      out += `<ul>${buf.join("")}</ul>`;
      continue;
    }

    // Ordered list — 1. / 2. / ...
    if (/^\s*\d+\.\s+/.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        buf.push(`<li>${renderInline(lines[i].replace(/^\s*\d+\.\s+/, ""))}</li>`);
        i++;
      }
      out += `<ol>${buf.join("")}</ol>`;
      continue;
    }

    // Paragraph — gather adjacent non-blank, non-block lines.
    const buf: string[] = [];
    while (i < lines.length && !/^\s*$/.test(lines[i])) {
      const ln = lines[i];
      if (
        /^([`~]{3,})/.test(ln) ||
        /^(#{1,6})\s+/.test(ln) ||
        /^\s*>\s?/.test(ln) ||
        /^\s*[-*+]\s+/.test(ln) ||
        /^\s*\d+\.\s+/.test(ln) ||
        /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(ln)
      ) {
        break;
      }
      buf.push(ln);
      i++;
    }
    if (buf.length > 0) {
      const joined = buf.join("\n").replace(/  $/gm, "\n");
      out += `<p>${renderInline(joined)}</p>`;
    }
  }

  return out;
}

export function isMarkdownFilename(name: string): boolean {
  return /\.(md|markdown|mdown|mkd|mkdn)$/i.test(name);
}
