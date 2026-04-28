import { useMemo } from "react";
import { Minus, Plus } from "lucide-react";

// Line-level diff display: shows added / removed / unchanged lines
// between two text blobs. Used by the web-server editor to preview
// backup → draft before the user commits a save.
//
// We hand-roll an LCS-based diff rather than pulling in `diff` from
// npm — for typical config files (tens to a few hundred lines) the
// O(n·m) LCS table fits in memory comfortably and the naive O(nm)
// time is sub-millisecond.

type Props = {
  /** "Before" text — usually the on-disk content. */
  oldText: string;
  /** "After" text — the dirty buffer about to be saved. */
  newText: string;
  /** Cap on visible context lines around each change. Defaults to 3. */
  context?: number;
};

type DiffLine =
  | { kind: "ctx"; oldNo: number; newNo: number; text: string }
  | { kind: "del"; oldNo: number; text: string }
  | { kind: "add"; newNo: number; text: string };

export default function DiffPreview({ oldText, newText, context = 3 }: Props) {
  const lines = useMemo(
    () => collapseContext(diffLines(oldText, newText), context),
    [oldText, newText, context],
  );

  if (oldText === newText) {
    return (
      <div className="diff-preview diff-preview--empty mono">
        No changes — the buffer matches what's on disk.
      </div>
    );
  }

  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l && (l as DiffLine).kind === "add") added++;
    else if (l && (l as DiffLine).kind === "del") removed++;
  }

  return (
    <div className="diff-preview">
      <div className="diff-preview__head mono">
        <span className="diff-preview__stat is-add">+{added}</span>
        <span className="diff-preview__stat is-del">−{removed}</span>
      </div>
      <pre className="diff-preview__body mono">
        {lines.map((l, i) => {
          if (l === null) {
            return (
              <span key={i} className="diff-preview__skip">
                @@ … @@
                {"\n"}
              </span>
            );
          }
          if (l.kind === "del") {
            return (
              <span key={i} className="diff-preview__line is-del">
                <span className="diff-preview__gutter mono">
                  {l.oldNo.toString().padStart(4, " ")}
                </span>
                <Minus size={9} />
                <span className="diff-preview__text">{l.text}</span>
              </span>
            );
          }
          if (l.kind === "add") {
            return (
              <span key={i} className="diff-preview__line is-add">
                <span className="diff-preview__gutter mono">
                  {l.newNo.toString().padStart(4, " ")}
                </span>
                <Plus size={9} />
                <span className="diff-preview__text">{l.text}</span>
              </span>
            );
          }
          return (
            <span key={i} className="diff-preview__line">
              <span className="diff-preview__gutter mono">
                {l.oldNo.toString().padStart(4, " ")}
              </span>
              <span className="diff-preview__sp"> </span>
              <span className="diff-preview__text">{l.text}</span>
            </span>
          );
        })}
      </pre>
    </div>
  );
}

// ── LCS line diff ───────────────────────────────────────────────────

/** Compute a line-by-line diff with del/add/context entries. Uses an
 *  LCS table (O(n*m) space + time). For typical config files this is
 *  trivially fast; for very large diffs we'd switch to Myers, but
 *  this is fine for the use case (saving an nginx / apache / caddy
 *  config). */
function diffLines(a: string, b: string): DiffLine[] {
  const aL = a.length === 0 ? [] : a.split("\n");
  const bL = b.length === 0 ? [] : b.split("\n");
  // Trailing newline: split() leaves a final "" — drop it so we don't
  // emit a phantom "added empty line" diff.
  if (aL.length > 0 && aL[aL.length - 1] === "") aL.pop();
  if (bL.length > 0 && bL[bL.length - 1] === "") bL.pop();

  const n = aL.length;
  const m = bL.length;
  // dp[i][j] = LCS length of aL[0..i] vs bL[0..j]
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < m; j++) {
      if (aL[i] === bL[j]) dp[i + 1][j + 1] = dp[i][j] + 1;
      else dp[i + 1][j + 1] = Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }
  // Walk back to produce the diff in forward order.
  const out: DiffLine[] = [];
  let i = n;
  let j = m;
  const stack: DiffLine[] = [];
  while (i > 0 && j > 0) {
    if (aL[i - 1] === bL[j - 1]) {
      stack.push({ kind: "ctx", oldNo: i, newNo: j, text: aL[i - 1] });
      i--;
      j--;
    } else if (dp[i - 1][j] >= dp[i][j - 1]) {
      stack.push({ kind: "del", oldNo: i, text: aL[i - 1] });
      i--;
    } else {
      stack.push({ kind: "add", newNo: j, text: bL[j - 1] });
      j--;
    }
  }
  while (i > 0) {
    stack.push({ kind: "del", oldNo: i, text: aL[i - 1] });
    i--;
  }
  while (j > 0) {
    stack.push({ kind: "add", newNo: j, text: bL[j - 1] });
    j--;
  }
  for (let k = stack.length - 1; k >= 0; k--) out.push(stack[k]);
  return out;
}

/** Collapse runs of unchanged lines longer than `context*2 + 1` to
 *  show only `context` lines on each side of every change, separated
 *  by a `null` "skip" marker (rendered as `@@ … @@`). */
function collapseContext(
  lines: DiffLine[],
  context: number,
): (DiffLine | null)[] {
  const isChange = (l: DiffLine) => l.kind !== "ctx";
  // Find indices of every change line.
  const changeIdx: number[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (isChange(lines[i])) changeIdx.push(i);
  }
  if (changeIdx.length === 0) return lines.slice();

  // Build a set of indices we want to keep (changes + their context).
  const keep = new Set<number>();
  for (const idx of changeIdx) {
    for (let k = idx - context; k <= idx + context; k++) {
      if (k >= 0 && k < lines.length) keep.add(k);
    }
  }

  // Walk in order; insert `null` between non-contiguous kept indices.
  const out: (DiffLine | null)[] = [];
  let prev = -2;
  for (let i = 0; i < lines.length; i++) {
    if (!keep.has(i)) continue;
    if (prev !== -1 && i !== prev + 1) {
      out.push(null);
    }
    out.push(lines[i]);
    prev = i;
  }
  return out;
}
