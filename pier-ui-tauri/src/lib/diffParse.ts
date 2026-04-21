// Unified diff parser → hunk lines with old/new line numbers.
// Accepts the raw `git diff` output from the backend and produces a
// structured list of hunks suitable for unified or split-view rendering.

export type DiffLineKind = "ctx" | "add" | "del";

export type DiffLine = {
  kind: DiffLineKind;
  oldLine: number | null;
  newLine: number | null;
  text: string;
};

export type DiffHunk = {
  header: string;
  lines: DiffLine[];
};

export type ParsedDiff = {
  oldPath: string;
  newPath: string;
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
};

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;

export function parseUnifiedDiff(text: string): ParsedDiff {
  const out: ParsedDiff = {
    oldPath: "",
    newPath: "",
    hunks: [],
    additions: 0,
    deletions: 0,
  };
  if (!text) return out;

  const lines = text.split("\n");
  let current: DiffHunk | null = null;
  let oldLineNo = 0;
  let newLineNo = 0;

  for (const raw of lines) {
    if (raw.startsWith("--- ")) {
      out.oldPath = raw.slice(4).replace(/^a\//, "");
      continue;
    }
    if (raw.startsWith("+++ ")) {
      out.newPath = raw.slice(4).replace(/^b\//, "");
      continue;
    }
    if (raw.startsWith("diff --git") || raw.startsWith("index ") || raw.startsWith("new file") || raw.startsWith("deleted file") || raw.startsWith("old mode") || raw.startsWith("new mode") || raw.startsWith("similarity ") || raw.startsWith("rename ") || raw.startsWith("copy ")) {
      continue;
    }
    const m = HUNK_RE.exec(raw);
    if (m) {
      current = { header: raw, lines: [] };
      out.hunks.push(current);
      oldLineNo = Number.parseInt(m[1], 10);
      newLineNo = Number.parseInt(m[3], 10);
      continue;
    }
    if (!current) continue;

    // Skip "\ No newline at end of file" markers
    if (raw.startsWith("\\")) continue;

    const prefix = raw.charAt(0);
    const body = raw.slice(1);

    if (prefix === "+") {
      current.lines.push({ kind: "add", oldLine: null, newLine: newLineNo, text: body });
      newLineNo++;
      out.additions++;
    } else if (prefix === "-") {
      current.lines.push({ kind: "del", oldLine: oldLineNo, newLine: null, text: body });
      oldLineNo++;
      out.deletions++;
    } else {
      // Context line (leading space) or blank line inside a hunk
      current.lines.push({ kind: "ctx", oldLine: oldLineNo, newLine: newLineNo, text: body });
      oldLineNo++;
      newLineNo++;
    }
  }

  return out;
}

export type SplitPair = { left: DiffLine | null; right: DiffLine | null };

export function pairHunkLines(lines: DiffLine[]): SplitPair[] {
  const out: SplitPair[] = [];
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (l.kind === "ctx") {
      out.push({ left: l, right: l });
      i++;
      continue;
    }
    if (l.kind === "del") {
      const dels: DiffLine[] = [];
      while (i < lines.length && lines[i].kind === "del") { dels.push(lines[i]); i++; }
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].kind === "add") { adds.push(lines[i]); i++; }
      const n = Math.max(dels.length, adds.length);
      for (let k = 0; k < n; k++) out.push({ left: dels[k] ?? null, right: adds[k] ?? null });
      continue;
    }
    if (l.kind === "add") {
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].kind === "add") { adds.push(lines[i]); i++; }
      for (const a of adds) out.push({ left: null, right: a });
      continue;
    }
    i++;
  }
  return out;
}
