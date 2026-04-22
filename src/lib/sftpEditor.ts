/** Pure helpers for the SFTP editor dialog: filename → language,
 *  editable heuristics, and a CodeMirror theme that reads from the
 *  same CSS custom properties the rest of the shell uses. Separated
 *  from the dialog component so the render path stays slim and the
 *  helpers are unit-test friendly. */

import { json } from "@codemirror/lang-json";
import { yaml } from "@codemirror/lang-yaml";
import { python } from "@codemirror/lang-python";
import { javascript } from "@codemirror/lang-javascript";
import { StreamLanguage } from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { xml } from "@codemirror/legacy-modes/mode/xml";
import { css } from "@codemirror/legacy-modes/mode/css";
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

/** Upper bound shipped to the backend and enforced on the UI side
 *  too. Backend caps at 5 MB regardless. */
export const MAX_EDITOR_BYTES = 5 * 1024 * 1024;

/** Extensions the editor opens without a size-gate prompt. Anything
 *  else still opens if under the byte limit, but large unknown files
 *  trip the confirmation. */
const TEXT_EXTENSIONS = new Set([
  "sh", "bash", "zsh", "fish",
  "conf", "cfg", "ini", "properties", "env",
  "json", "yaml", "yml", "toml",
  "js", "mjs", "cjs", "ts", "tsx", "jsx",
  "py", "rb", "go", "rs", "java", "kt", "swift", "php", "pl", "lua",
  "md", "markdown", "rst", "txt", "log",
  "xml", "html", "htm", "svg", "css", "scss", "less",
  "sql", "service", "socket", "timer", "mount",
  "c", "h", "cc", "cpp", "hpp",
  "dockerfile", "tf", "hcl",
]);

/** Filenames treated as editable regardless of extension. */
const TEXT_FILENAMES = new Set([
  "Dockerfile", "Makefile", "Rakefile", "Gemfile", "Vagrantfile",
  ".bashrc", ".zshrc", ".profile", ".bash_profile", ".gitconfig",
  ".vimrc", ".tmux.conf", ".env", ".npmrc",
]);

function extensionOf(name: string): string {
  const idx = name.lastIndexOf(".");
  if (idx < 0 || idx === name.length - 1) return "";
  return name.slice(idx + 1).toLowerCase();
}

export function isEditableFilename(name: string): boolean {
  if (!name) return false;
  if (TEXT_FILENAMES.has(name)) return true;
  const ext = extensionOf(name);
  if (!ext) {
    // no-extension files are often editable (scripts, configs); the
    // backend size gate is the real safety net.
    return true;
  }
  return TEXT_EXTENSIONS.has(ext);
}

/** Pick a CodeMirror language support for a filename. Returns
 *  `null` when no mode matches — the editor then falls back to
 *  plain text, which still has line numbers + search + rectangular
 *  selection. */
export function languageFromFilename(name: string): Extension | null {
  const lower = name.toLowerCase();
  if (lower === "dockerfile" || lower.endsWith(".dockerfile")) {
    return StreamLanguage.define(dockerFile);
  }
  const ext = extensionOf(name);
  switch (ext) {
    case "json":
      return json();
    case "yaml":
    case "yml":
      return yaml();
    case "py":
      return python();
    case "js":
    case "mjs":
    case "cjs":
      return javascript();
    case "ts":
      return javascript({ typescript: true });
    case "jsx":
      return javascript({ jsx: true });
    case "tsx":
      return javascript({ jsx: true, typescript: true });
    case "sh":
    case "bash":
    case "zsh":
    case "fish":
    case "env":
      return StreamLanguage.define(shell);
    case "toml":
      return StreamLanguage.define(toml);
    case "properties":
    case "ini":
    case "cfg":
    case "conf":
      return StreamLanguage.define(properties);
    case "nginx":
      return StreamLanguage.define(nginx);
    case "xml":
    case "html":
    case "htm":
    case "svg":
      return StreamLanguage.define(xml);
    case "css":
    case "scss":
    case "less":
      return StreamLanguage.define(css);
    default:
      return null;
  }
}

/** Short human label of the detected language, shown in the status
 *  bar. Parallel switch to [`languageFromFilename`] but returns a
 *  user-facing name instead of a CM6 extension. */
export function languageLabel(name: string): string {
  const lower = name.toLowerCase();
  if (lower === "dockerfile" || lower.endsWith(".dockerfile")) return "Dockerfile";
  const ext = extensionOf(name);
  switch (ext) {
    case "json": return "JSON";
    case "yaml":
    case "yml": return "YAML";
    case "py": return "Python";
    case "js":
    case "mjs":
    case "cjs": return "JavaScript";
    case "ts": return "TypeScript";
    case "jsx": return "JSX";
    case "tsx": return "TSX";
    case "sh":
    case "bash":
    case "zsh":
    case "fish": return "Shell";
    case "env": return "dotenv";
    case "toml": return "TOML";
    case "properties":
    case "ini":
    case "cfg":
    case "conf": return "Config";
    case "nginx": return "Nginx";
    case "xml":
    case "html":
    case "htm":
    case "svg": return "XML";
    case "css": return "CSS";
    case "scss": return "SCSS";
    case "less": return "LESS";
    case "md":
    case "markdown": return "Markdown";
    case "sql": return "SQL";
    default: return ext ? ext.toUpperCase() : "Plain Text";
  }
}

/** CodeMirror theme that reads from the same CSS custom properties
 *  as the rest of the shell. Rebuilt every mount — cheap (string
 *  concatenation + a small object tree) and sidesteps stale closures
 *  when the user switches themes mid-session.
 *
 *  All colors route through `var(--…)` so dark/light and accent
 *  swaps apply without re-mounting the editor. */
export function buildEditorTheme(): Extension[] {
  const theme = EditorView.theme(
    {
      "&": {
        color: "var(--ink)",
        backgroundColor: "var(--panel)",
        height: "100%",
        fontSize: "var(--ui-fs)",
      },
      ".cm-scroller": {
        fontFamily: "var(--mono)",
        lineHeight: "1.5",
      },
      ".cm-content": {
        caretColor: "var(--accent)",
        padding: "var(--sp-2) 0",
      },
      ".cm-cursor, .cm-dropCursor": {
        borderLeftColor: "var(--accent)",
      },
      "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection":
        { backgroundColor: "var(--accent-dim)" },
      ".cm-gutters": {
        backgroundColor: "var(--surface)",
        color: "var(--muted)",
        border: "none",
        borderRight: "1px solid var(--line)",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "var(--panel-2)",
        color: "var(--ink)",
      },
      ".cm-activeLine": {
        backgroundColor: "var(--surface-2)",
      },
      ".cm-lineNumbers .cm-gutterElement": {
        padding: "0 var(--sp-2) 0 var(--sp-3)",
        fontSize: "var(--ui-fs-sm)",
      },
      ".cm-selectionMatch": {
        backgroundColor: "var(--accent-subtle)",
      },
      ".cm-matchingBracket": {
        backgroundColor: "var(--accent-dim)",
        color: "var(--ink)",
      },
      ".cm-searchMatch": {
        backgroundColor: "var(--warn-dim)",
        outline: "1px solid var(--warn)",
      },
      ".cm-searchMatch.cm-searchMatch-selected": {
        backgroundColor: "var(--warn)",
        color: "var(--accent-ink)",
      },
      ".cm-panels": {
        backgroundColor: "var(--surface-2)",
        color: "var(--ink)",
        borderTop: "1px solid var(--line)",
      },
      ".cm-panels .cm-panel": {
        padding: "var(--sp-1-5) var(--sp-2)",
      },
      ".cm-textfield": {
        backgroundColor: "var(--panel)",
        color: "var(--ink)",
        border: "1px solid var(--line-2)",
        borderRadius: "var(--radius-xs)",
        padding: "2px 6px",
        fontFamily: "var(--mono)",
        fontSize: "var(--ui-fs-sm)",
      },
      ".cm-button": {
        backgroundColor: "var(--panel-2)",
        color: "var(--ink)",
        border: "1px solid var(--line-2)",
        borderRadius: "var(--radius-xs)",
        padding: "2px 8px",
        fontFamily: "var(--sans)",
        fontSize: "var(--ui-fs-sm)",
      },
      ".cm-button:hover": {
        backgroundColor: "var(--elev)",
        borderColor: "var(--line-3)",
      },
      ".cm-tooltip": {
        backgroundColor: "var(--elev)",
        color: "var(--ink)",
        border: "1px solid var(--line-3)",
        borderRadius: "var(--radius-sm)",
      },
    },
    { dark: true },
  );

  const highlight = HighlightStyle.define([
    { tag: t.keyword, color: "var(--info)" },
    { tag: [t.name, t.deleted, t.character, t.macroName], color: "var(--ink)" },
    { tag: [t.propertyName], color: "var(--accent-hover)" },
    { tag: [t.variableName], color: "var(--ink)" },
    { tag: [t.function(t.variableName)], color: "var(--accent-hover)" },
    { tag: [t.labelName], color: "var(--ink-2)" },
    { tag: [t.color, t.constant(t.name), t.standard(t.name)], color: "var(--warn)" },
    { tag: [t.definition(t.name), t.separator], color: "var(--ink)" },
    { tag: [t.typeName, t.className, t.number, t.changed, t.annotation, t.modifier, t.self, t.namespace], color: "var(--warn)" },
    { tag: [t.operator, t.operatorKeyword, t.url, t.escape, t.regexp, t.link, t.special(t.string)], color: "var(--accent)" },
    { tag: [t.meta, t.comment], color: "var(--muted)", fontStyle: "italic" },
    { tag: t.strong, fontWeight: "bold" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.strikethrough, textDecoration: "line-through" },
    { tag: t.link, color: "var(--accent-hover)", textDecoration: "underline" },
    { tag: t.heading, fontWeight: "bold", color: "var(--ink)" },
    { tag: [t.atom, t.bool, t.special(t.variableName)], color: "var(--warn)" },
    { tag: [t.processingInstruction, t.string, t.inserted], color: "var(--pos)" },
    { tag: t.invalid, color: "var(--neg)" },
  ]);

  return [theme, syntaxHighlighting(highlight)];
}

/** Format octal mode as `rwxrwxrwx`. Used by both the chmod dialog
 *  (live preview) and the properties view. */
export function modeToSymbolic(mode: number): string {
  const bits = mode & 0o777;
  const flag = (b: number, ch: string) => (b ? ch : "-");
  const trio = (m: number) =>
    flag(m & 0o4, "r") + flag(m & 0o2, "w") + flag(m & 0o1, "x");
  return trio((bits >> 6) & 0o7) + trio((bits >> 3) & 0o7) + trio(bits & 0o7);
}

/** Lucide size="?" acceptor — keeps the typing consistent between
 *  panel rows and dialog toolbars. */
export type LucideIconProps = { size?: number };
