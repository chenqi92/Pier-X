/** Lightweight filename / mode helpers shared by SFTP surfaces.
 *
 * Keep this file free of CodeMirror imports. Directory browsing and
 * chmod previews need these helpers without pulling the full editor
 * runtime into their chunks.
 */

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

export function extensionOf(name: string): string {
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

/** How the SFTP double-click previewer should render a file. `text`
 *  covers everything not matched below — the streaming text viewer
 *  auto-detects binary content and falls back to a hex view, so
 *  unknown/extension-less files still open instantly and gracefully. */
export type PreviewKind =
  | "image"
  | "svg"
  | "tiff"
  | "pdf"
  | "spreadsheet"
  | "csv"
  | "docx"
  | "video"
  | "audio"
  | "text";

const IMAGE_EXTENSIONS = new Set([
  "png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "avif",
]);
const VIDEO_EXTENSIONS = new Set(["mp4", "m4v", "webm", "ogv", "mov"]);
const AUDIO_EXTENSIONS = new Set(["mp3", "wav", "ogg", "oga", "flac", "m4a", "aac"]);
const SPREADSHEET_EXTENSIONS = new Set(["xlsx", "xlsm", "xlsb", "xls", "ods"]);

/** Classify a filename into a {@link PreviewKind} for the double-click
 *  previewer. Pure extension match; content sniffing happens in the
 *  backend for the `text` path. */
export function previewKindOf(name: string): PreviewKind {
  const ext = extensionOf(name);
  if (ext === "svg") return "svg";
  if (ext === "tif" || ext === "tiff") return "tiff";
  if (IMAGE_EXTENSIONS.has(ext)) return "image";
  if (ext === "pdf") return "pdf";
  if (SPREADSHEET_EXTENSIONS.has(ext)) return "spreadsheet";
  if (ext === "csv" || ext === "tsv") return "csv";
  if (ext === "docx") return "docx";
  if (VIDEO_EXTENSIONS.has(ext)) return "video";
  if (AUDIO_EXTENSIONS.has(ext)) return "audio";
  return "text";
}

/** Short human label of the detected language, shown in the status bar. */
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

/** Format octal mode as `rwxrwxrwx`. Used by both the chmod dialog
 *  and the SFTP properties view. */
export function modeToSymbolic(mode: number): string {
  const bits = mode & 0o777;
  const flag = (b: number, ch: string) => (b ? ch : "-");
  const trio = (m: number) =>
    flag(m & 0o4, "r") + flag(m & 0o2, "w") + flag(m & 0o1, "x");
  return trio((bits >> 6) & 0o7) + trio((bits >> 3) & 0o7) + trio(bits & 0o7);
}
