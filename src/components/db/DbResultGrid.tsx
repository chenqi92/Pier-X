import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  Filter,
  KeyRound,
  Lock,
  Plus,
  Save,
  Trash2,
  Undo2,
  Unlock,
  X,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../../i18n/useI18n";
import type { DataPreview } from "../../lib/types";
import { prettyJsonish } from "./cellFormat";
import type { DbMutation, GridColumnMeta } from "./dbColumnRules";

type SortDir = "asc" | "desc";

type Props = {
  preview: DataPreview | null;
  /** Primary-key column names (rendered with a PK badge and right-aligned).
   *  Required for inline edit / delete to be enabled. */
  pkColumns?: string[];
  /** Right-aligned numeric columns (style hint + sort coerces). */
  numericColumns?: string[];
  toolbar?: ReactNode;
  /** Enables per-row click → opens the row detail drawer. */
  onOpenRow?: (row: string[]) => void;
  emptyLabel?: string;
  /** Default page size — user can flip in the pager. */
  defaultPageSize?: 50 | 100 | 200 | 500;

  /** Engine-aware metadata for inline editing — when omitted, the grid
   *  silently drops to read-only behaviour. */
  columnsMeta?: GridColumnMeta[];
  /** When true, double-click cells to edit, "Insert row", per-row delete,
   *  Commit / Discard footer all render. Parent gates this on the user
   *  having unlocked writes + typed the WRITE confirmation. */
  writable?: boolean;
  /** Receives the staged mutations on Commit. Parent assembles the SQL
   *  via `mutationToSql` and ships it through its *_execute command,
   *  then refreshes the preview. Returning a rejected promise keeps
   *  the dirty state intact so the user can retry. */
  onCommit?: (mutations: DbMutation[]) => Promise<void>;
  /** Optional spinner state from the parent — disables Commit. */
  committing?: boolean;
  /** When provided, the grid toolbar gets a Lock/Unlock chip that
   *  flips the parent's read-only state — same handler the SQL editor's
   *  lock uses. Surfaces the gate next to the data instead of burying
   *  it in the editor footer. */
  onToggleWritable?: () => void;
  /** Scoping key for persisted column widths. Pass something stable
   *  per (engine, database, table) — e.g. `mysql:foo.users`. When
   *  omitted, widths default to auto and aren't persisted. */
  storageKey?: string;
};

const PAGE_SIZES: Array<50 | 100 | 200 | 500> = [50, 100, 200, 500];

/** Internal record of a cell that's been edited but not committed. */
type DirtyCell = { row: number; col: string; original: string; next: string };

/**
 * Sticky-header, mono-font, token-coloured result grid. Client-side
 * sort + per-column filter + pagination over the snapshot returned by
 * the backend's `*_browse` call. The backend itself caps the preview
 * (e.g. 1000 rows for MySQL) so this stays cheap to filter in JS.
 *
 * When `writable` + `columnsMeta` + `onCommit` are all provided, the
 * grid enables inline cell editing, row insertion, row deletion, and
 * a dirty-tracking footer. The grid only emits abstract mutations —
 * the parent panel translates them into SQL per dialect.
 */
export default function DbResultGrid({
  preview,
  pkColumns,
  numericColumns,
  toolbar,
  onOpenRow,
  emptyLabel,
  defaultPageSize = 100,
  columnsMeta,
  writable = false,
  onCommit,
  committing = false,
  onToggleWritable,
  storageKey,
}: Props) {
  const { t } = useI18n();
  const [sortCol, setSortCol] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [filters, setFilters] = useState<Record<string, string>>({});
  const [filterOpen, setFilterOpen] = useState(false);
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useState<number>(defaultPageSize);

  // Edit-mode state. Keyed by absolute row index (in the *original*
  // preview.rows) so sort/filter/pager don't desync.
  const [editing, setEditing] = useState<{ row: number; col: string } | null>(null);
  const [dirtyMap, setDirtyMap] = useState<Map<string, DirtyCell>>(new Map());
  const [pendingInserts, setPendingInserts] = useState<Record<string, string>[]>([]);
  const [pendingDeletes, setPendingDeletes] = useState<Set<number>>(new Set());
  // Per-column width overrides, keyed by column name. Loaded from
  // localStorage when `storageKey` is provided; falls back to "auto"
  // (browser-decided) when no override is set. Stored as integer
  // pixels — sub-pixel precision isn't worth the JSON noise.
  const widthsKey = storageKey ? `pier-x:rg-widths:${storageKey}` : null;
  const [colWidths, setColWidths] = useState<Record<string, number>>(() => {
    if (!widthsKey) return {};
    try {
      const raw = localStorage.getItem(widthsKey);
      if (!raw) return {};
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        const out: Record<string, number> = {};
        for (const [k, v] of Object.entries(parsed)) {
          if (typeof v === "number" && v > 0 && v < 2000) out[k] = v;
        }
        return out;
      }
    } catch {
      // localStorage parse fail — drop and start with empty overrides.
    }
    return {};
  });
  // Reload widths when the storage key changes (e.g. user switched
  // tables). Saves the previous table's edits to its own bucket via
  // the persistence effect below.
  useEffect(() => {
    if (!widthsKey) {
      setColWidths({});
      return;
    }
    try {
      const raw = localStorage.getItem(widthsKey);
      const parsed = raw ? JSON.parse(raw) : null;
      if (parsed && typeof parsed === "object") {
        const out: Record<string, number> = {};
        for (const [k, v] of Object.entries(parsed)) {
          if (typeof v === "number" && v > 0 && v < 2000) out[k] = v;
        }
        setColWidths(out);
      } else {
        setColWidths({});
      }
    } catch {
      setColWidths({});
    }
  }, [widthsKey]);
  // Persist on each change. Cheap — 0..N entries, single localStorage
  // write per drag commit.
  useEffect(() => {
    if (!widthsKey) return;
    try {
      if (Object.keys(colWidths).length === 0) {
        localStorage.removeItem(widthsKey);
      } else {
        localStorage.setItem(widthsKey, JSON.stringify(colWidths));
      }
    } catch {
      /* ignore */
    }
  }, [widthsKey, colWidths]);

  // Per-table column-order override. Stored as the desired column
  // sequence — names that vanish when the table changes are dropped,
  // names that newly appear get appended after the saved ones. `null`
  // / empty array means "use the natural order from preview".
  const orderKey = storageKey ? `pier-x:rg-order:${storageKey}` : null;
  const [colOrder, setColOrder] = useState<string[] | null>(() => {
    if (!orderKey) return null;
    try {
      const raw = localStorage.getItem(orderKey);
      if (!raw) return null;
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
        return parsed as string[];
      }
    } catch {
      /* fall through */
    }
    return null;
  });
  useEffect(() => {
    if (!orderKey) {
      setColOrder(null);
      return;
    }
    try {
      const raw = localStorage.getItem(orderKey);
      if (!raw) {
        setColOrder(null);
        return;
      }
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
        setColOrder(parsed as string[]);
      } else {
        setColOrder(null);
      }
    } catch {
      setColOrder(null);
    }
  }, [orderKey]);
  useEffect(() => {
    if (!orderKey) return;
    try {
      if (!colOrder || colOrder.length === 0) {
        localStorage.removeItem(orderKey);
      } else {
        localStorage.setItem(orderKey, JSON.stringify(colOrder));
      }
    } catch {
      /* ignore */
    }
  }, [orderKey, colOrder]);

  // Active drag-target column for the reorder UI. Highlights the th
  // a drop would land on without committing yet — matches the IDE
  // file-tree drag behaviour the user already trusts.
  const [dragOverCol, setDragOverCol] = useState<string | null>(null);

  /** Apply `colOrder` to a preview by reshuffling columns + every
   *  row's cells. Memoized: a 1000-row preview reshuffles in one tick
   *  per (preview, order) pair. Returns the input unchanged when no
   *  reorder is active. */
  const viewPreview = useMemo(() => {
    if (!preview) return preview;
    if (!colOrder || colOrder.length === 0) return preview;
    const have = new Set(preview.columns);
    const ordered: string[] = [];
    for (const c of colOrder) {
      if (have.has(c) && !ordered.includes(c)) ordered.push(c);
    }
    for (const c of preview.columns) {
      if (!ordered.includes(c)) ordered.push(c);
    }
    if (ordered.every((c, i) => c === preview.columns[i])) return preview;
    const idxOf = ordered.map((c) => preview.columns.indexOf(c));
    const rows = preview.rows.map((r) => idxOf.map((i) => r[i]));
    return { ...preview, columns: ordered, rows };
  }, [preview, colOrder]);

  /** Persist the user's drop. We commit the FULL displayed order
   *  (not just the changed pair) so the bucket reflects the final
   *  layout — easier to reason about than a sparse delta. */
  function commitColOrder(next: string[]) {
    setColOrder(next);
  }

  /** Reset to the natural column order from the source query. Wired
   *  to a one-shot button shown only when an override is active. */
  function resetColOrder() {
    setColOrder(null);
  }

  /** Begin a column-width drag from the th's right-edge grip.
   *  We capture pointer events so the drag continues even when the
   *  cursor leaves the grip; releases the capture on mouseup. */
  function startColResize(col: string, e: React.PointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const th = (e.currentTarget as HTMLElement).closest("th");
    const startW = th ? th.getBoundingClientRect().width : 120;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const onMove = (ev: PointerEvent) => {
      const delta = ev.clientX - startX;
      const next = Math.max(40, Math.min(1500, Math.round(startW + delta)));
      setColWidths((prev) => ({ ...prev, [col]: next }));
    };
    const onUp = (ev: PointerEvent) => {
      target.releasePointerCapture(ev.pointerId);
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
      target.removeEventListener("pointercancel", onUp);
    };
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
    target.addEventListener("pointercancel", onUp);
  }

  // Reset paging + edit state when the preview swaps under us — otherwise
  // switching tables can leave us showing "page 12 of 3" or stale dirty
  // cells pointing at a different shape.
  useEffect(() => {
    setPage(0);
    setSortCol(null);
    setFilters({});
    setEditing(null);
    setDirtyMap(new Map());
    setPendingInserts([]);
    setPendingDeletes(new Set());
  }, [preview]);

  const numericSet = useMemo(() => new Set(numericColumns ?? []), [numericColumns]);
  const pkSet = useMemo(() => new Set(pkColumns ?? []), [pkColumns]);

  const cols = viewPreview?.columns ?? columnsMeta?.map((c) => c.name) ?? [];
  const editEnabled = writable && !!columnsMeta && pkSet.size > 0 && !!onCommit;
  const insertEnabled = writable && !!columnsMeta && !!onCommit;

  const filteredSorted = useMemo(() => {
    if (!viewPreview) return [] as { row: string[]; absIdx: number }[];
    let pairs = viewPreview.rows.map((row, absIdx) => ({ row, absIdx }));
    // Filter
    const activeFilters = Object.entries(filters).filter(([, q]) => q.trim() !== "");
    if (activeFilters.length > 0) {
      pairs = pairs.filter(({ row }) =>
        activeFilters.every(([col, q]) => {
          const ci = cols.indexOf(col);
          if (ci < 0) return true;
          const cell = row[ci];
          return (cell ?? "").toString().toLowerCase().includes(q.toLowerCase());
        }),
      );
    }
    // Sort
    if (sortCol) {
      const ci = cols.indexOf(sortCol);
      if (ci >= 0) {
        const numeric = numericSet.has(sortCol);
        pairs = [...pairs].sort((a, b) => {
          const va = a.row[ci];
          const vb = b.row[ci];
          if (numeric) {
            const na = Number(va);
            const nb = Number(vb);
            if (Number.isFinite(na) && Number.isFinite(nb)) {
              return sortDir === "asc" ? na - nb : nb - na;
            }
          }
          const sa = (va ?? "").toString();
          const sb = (vb ?? "").toString();
          return sortDir === "asc" ? sa.localeCompare(sb) : sb.localeCompare(sa);
        });
      }
    }
    return pairs;
  }, [viewPreview, filters, sortCol, sortDir, cols, numericSet]);

  const pageCount = Math.max(1, Math.ceil(filteredSorted.length / pageSize));
  const safePage = Math.min(page, pageCount - 1);
  const slice = useMemo(
    () => filteredSorted.slice(safePage * pageSize, (safePage + 1) * pageSize),
    [filteredSorted, safePage, pageSize],
  );

  const dirtyCount = dirtyMap.size + pendingInserts.length + pendingDeletes.size;

  const toggleSort = (col: string) => {
    if (sortCol === col) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortCol(col);
      setSortDir("asc");
    }
  };

  const dirtyKey = (rowIdx: number, col: string) => `${rowIdx}:${col}`;

  const dirtyValueFor = useCallback(
    (rowIdx: number, col: string): string | null => {
      const k = dirtyKey(rowIdx, col);
      const entry = dirtyMap.get(k);
      return entry ? entry.next : null;
    },
    [dirtyMap],
  );

  const startEdit = (rowIdx: number, col: string) => {
    if (!editEnabled) return;
    if (pendingDeletes.has(rowIdx)) return;
    if (pkSet.has(col)) return; // PK is the row identity — never edit
    setEditing({ row: rowIdx, col });
  };

  const commitCellEdit = (rowIdx: number, col: string, original: string, next: string) => {
    setEditing(null);
    if (next === original) {
      // No change — clear any pre-existing dirty entry for this cell.
      setDirtyMap((prev) => {
        const m = new Map(prev);
        m.delete(dirtyKey(rowIdx, col));
        return m;
      });
      return;
    }
    setDirtyMap((prev) => {
      const m = new Map(prev);
      m.set(dirtyKey(rowIdx, col), { row: rowIdx, col, original, next });
      return m;
    });
  };

  const cancelEdit = () => setEditing(null);

  const togglePendingDelete = (rowIdx: number) => {
    setPendingDeletes((prev) => {
      const next = new Set(prev);
      if (next.has(rowIdx)) next.delete(rowIdx);
      else next.add(rowIdx);
      return next;
    });
  };

  const startInsert = () => {
    if (!insertEnabled) return;
    const init: Record<string, string> = {};
    for (const c of cols) init[c] = "";
    setPendingInserts((prev) => [...prev, init]);
  };

  /**
   * Splits a clipboard payload into rows, parses each as TSV (or
   * comma-separated when no tabs are present), and stages them as
   * pending insert rows. Empty trailing lines are dropped. When the
   * payload has more columns than the current table, extras are
   * ignored; when fewer, the missing columns stay empty (NULL on
   * commit). Returns the number of rows staged so the caller can
   * surface a toast.
   */
  const stageRowsFromTsv = (raw: string): number => {
    if (!insertEnabled) return 0;
    const lines = raw
      .replace(/\r\n/g, "\n")
      .replace(/\r/g, "\n")
      .split("\n")
      .filter((l) => l.length > 0);
    if (lines.length === 0) return 0;
    const sep = lines[0].includes("\t") ? "\t" : ",";
    const staged: Record<string, string>[] = [];
    for (const line of lines) {
      const fields = line.split(sep);
      const row: Record<string, string> = {};
      for (let i = 0; i < cols.length; i++) {
        row[cols[i]] = (fields[i] ?? "").trim();
      }
      staged.push(row);
    }
    setPendingInserts((prev) => [...prev, ...staged]);
    return staged.length;
  };

  const discardAll = () => {
    setDirtyMap(new Map());
    setPendingInserts([]);
    setPendingDeletes(new Set());
    setEditing(null);
  };

  const collectMutations = useCallback((): DbMutation[] => {
    if (!viewPreview) return [];
    const muts: DbMutation[] = [];
    // Collect cell edits per row
    const byRow = new Map<number, DirtyCell[]>();
    for (const cell of dirtyMap.values()) {
      const list = byRow.get(cell.row) ?? [];
      list.push(cell);
      byRow.set(cell.row, list);
    }
    for (const [rowIdx, cells] of byRow.entries()) {
      const original = viewPreview?.rows[rowIdx];
      if (!original) continue;
      const pk: Record<string, string> = {};
      for (const c of cols) {
        if (pkSet.has(c)) pk[c] = original[cols.indexOf(c)] ?? "";
      }
      const changes: Record<string, string | null> = {};
      for (const cell of cells) {
        changes[cell.col] = cell.next === "" ? null : cell.next;
      }
      muts.push({ kind: "update", pk, changes });
    }
    // Pending inserts — emit one mutation per staged row.
    for (const draft of pendingInserts) {
      const values: Record<string, string | null> = {};
      for (const c of cols) {
        const v = draft[c];
        // Skip empty + PK columns (let the DB auto-generate). For non-PK
        // empty columns, send NULL so the DB applies its default.
        if (pkSet.has(c) && (v === undefined || v === "")) continue;
        values[c] = v === undefined || v === "" ? null : v;
      }
      muts.push({ kind: "insert", values });
    }
    // Pending deletes
    for (const rowIdx of pendingDeletes) {
      const original = viewPreview?.rows[rowIdx];
      if (!original) continue;
      const pk: Record<string, string> = {};
      for (const c of cols) {
        if (pkSet.has(c)) pk[c] = original[cols.indexOf(c)] ?? "";
      }
      muts.push({ kind: "delete", pk });
    }
    return muts;
  }, [preview, dirtyMap, pendingInserts, pendingDeletes, cols, pkSet]);

  const onCommitClick = async () => {
    if (!onCommit || dirtyCount === 0) return;
    const muts = collectMutations();
    if (muts.length === 0) return;
    // Confirm destructive deletes — INSERT/UPDATE are recoverable
    // by re-editing, but DELETE goes through immediately and there's
    // no undo. Skip the prompt when only one row is being dropped
    // (the per-row trash interaction is itself a deliberate gesture).
    const deleteCount = muts.filter((m) => m.kind === "delete").length;
    if (deleteCount > 1) {
      const confirmed = window.confirm(
        t("Commit will permanently delete {n} row(s). Continue?", {
          n: deleteCount,
        }),
      );
      if (!confirmed) return;
    }
    try {
      await onCommit(muts);
      // Parent is responsible for re-browsing; clear our local state.
      discardAll();
    } catch {
      // Keep dirty state — parent surfaced the error already.
    }
  };

  if (!preview && cols.length === 0) {
    return (
      <div className="rg">
        {toolbar && <div className="rg-toolbar">{toolbar}</div>}
        <div className="rg-empty">{emptyLabel ?? t("No rows to show.")}</div>
      </div>
    );
  }

  const totalRows = preview?.rows.length ?? 0;
  const activeFilterCount = Object.values(filters).filter((v) => v.trim() !== "").length;

  return (
    <div className="rg">
      <div className="rg-toolbar">
        <span className="rg-stat">
          <b>{filteredSorted.length.toLocaleString()}</b>
          <span className="rg-stat-muted"> {t("rows")}</span>
          {filteredSorted.length !== totalRows && (
            <span className="rg-stat-muted">
              {" · "}{t("filtered from {total}", { total: totalRows.toLocaleString() })}
            </span>
          )}
          {preview?.truncated && (
            <span className="rg-stat-muted"> · {t("truncated")}</span>
          )}
        </span>
        {dirtyCount > 0 && (
          <span className="rg-pending">
            <Save size={9} />
            {t("{n} pending writes", { n: dirtyCount })}
          </span>
        )}
        <button
          type="button"
          className={"btn is-ghost is-compact" + (filterOpen ? " is-active" : "")}
          onClick={() => setFilterOpen((v) => !v)}
          title={t("Filter")}
        >
          <Filter size={10} />
          {t("Filter")}
          {activeFilterCount > 0 && <span className="rg-filter-count">{activeFilterCount}</span>}
        </button>
        {colOrder && colOrder.length > 0 && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={resetColOrder}
            title={t(
              "Restore the original column order from the source query.",
            )}
          >
            {t("Reset order")}
          </button>
        )}
        {insertEnabled && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={startInsert}
            title={t("Insert row")}
          >
            <Plus size={10} /> {t("Insert row")}
          </button>
        )}
        {insertEnabled && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={async () => {
              try {
                const txt = await navigator.clipboard.readText();
                const n = stageRowsFromTsv(txt);
                if (n === 0 && !txt.trim()) return;
              } catch {
                // Clipboard read denied — fall back to a prompt so
                // the feature is still useful in environments where
                // the permission isn't granted.
                const txt = window.prompt(
                  t(
                    "Paste TSV (one row per line, columns separated by Tab):",
                  ),
                  "",
                );
                if (txt) stageRowsFromTsv(txt);
              }
            }}
            title={t(
              "Paste a TSV block from the clipboard — each line becomes a pending INSERT.",
            )}
          >
            <Plus size={10} /> {t("Paste TSV")}
          </button>
        )}
        {onToggleWritable && !!columnsMeta && (
          <button
            type="button"
            className={
              "btn is-ghost is-compact rg-write-toggle" +
              (writable ? " is-on" : "")
            }
            onClick={onToggleWritable}
            title={
              writable
                ? t("Lock writes (return grid to read-only)")
                : t("Unlock writes (enables double-click cell edit)")
            }
          >
            {writable ? <Unlock size={10} /> : <Lock size={10} />}{" "}
            {writable ? t("Writes unlocked") : t("Read-only")}
          </button>
        )}
        {toolbar}
      </div>

      <div className="rg-scroll">
        <table className="rg-table">
          <colgroup>
            <col className="rg-col-n" />
            {cols.map((col) => (
              <col
                key={col}
                style={
                  colWidths[col]
                    ? { width: `${colWidths[col]}px` }
                    : undefined
                }
              />
            ))}
            {(editEnabled || insertEnabled) && <col className="rg-col-acts" />}
          </colgroup>
          <thead>
            <tr>
              <th className="rg-th-n">#</th>
              {cols.map((col) => {
                const isPk = pkSet.has(col);
                const isNum = numericSet.has(col);
                const align = isNum ? "right" : "left";
                const sorted = sortCol === col;
                const isDropTarget = dragOverCol === col;
                return (
                  <th
                    key={col}
                    className={
                      "rg-th" +
                      (sorted ? " rg-th-sorted" : "") +
                      (isDropTarget ? " rg-th-drop" : "")
                    }
                    style={{ textAlign: align }}
                    onClick={() => toggleSort(col)}
                    draggable={!!storageKey}
                    onDragStart={(e) => {
                      // Block resize-grip drags from spuriously firing
                      // a column-reorder. The grip lives inside the th
                      // and would otherwise propagate.
                      const target = e.target as HTMLElement;
                      if (target.classList?.contains("rg-th-grip")) {
                        e.preventDefault();
                        return;
                      }
                      e.dataTransfer.effectAllowed = "move";
                      e.dataTransfer.setData("text/plain", col);
                    }}
                    onDragOver={(e) => {
                      // dataTransfer types check: only react to our own
                      // payload. Without this an outside-the-grid drag
                      // (a file from the OS) would also light up.
                      if (
                        e.dataTransfer.types.includes("text/plain") &&
                        storageKey
                      ) {
                        e.preventDefault();
                        e.dataTransfer.dropEffect = "move";
                        if (dragOverCol !== col) setDragOverCol(col);
                      }
                    }}
                    onDragLeave={() => {
                      if (dragOverCol === col) setDragOverCol(null);
                    }}
                    onDrop={(e) => {
                      e.preventDefault();
                      setDragOverCol(null);
                      const src = e.dataTransfer.getData("text/plain");
                      if (!src || src === col) return;
                      // Build the new order by removing src then
                      // re-inserting it before the drop target. Works
                      // for moves left-to-right (src earlier than dst)
                      // and right-to-left (src later than dst).
                      const withoutSrc = cols.filter((c) => c !== src);
                      const dstAt = withoutSrc.indexOf(col);
                      const next = [
                        ...withoutSrc.slice(0, dstAt),
                        src,
                        ...withoutSrc.slice(dstAt),
                      ];
                      commitColOrder(next);
                    }}
                  >
                    <div className="rg-th-body">
                      {isPk && (
                        <span className="rg-pk" title={t("Primary key")} aria-label={t("Primary key")}>
                          <KeyRound size={10} />
                        </span>
                      )}
                      <span className="rg-th-name">{col}</span>
                      {sorted && (
                        <span className="rg-th-sort">
                          {sortDir === "asc" ? <ArrowUp size={9} /> : <ArrowDown size={9} />}
                        </span>
                      )}
                    </div>
                    <span
                      className="rg-th-grip"
                      onPointerDown={(e) => startColResize(col, e)}
                      onClick={(e) => e.stopPropagation()}
                      onDoubleClick={(e) => {
                        e.stopPropagation();
                        // Double-click resets this column to auto width.
                        setColWidths((prev) => {
                          const next = { ...prev };
                          delete next[col];
                          return next;
                        });
                      }}
                      title={t("Drag to resize · double-click to reset")}
                    />
                  </th>
                );
              })}
              {(editEnabled || insertEnabled) && <th className="rg-th-acts" />}
            </tr>
            {filterOpen && (
              <tr className="rg-filter-row">
                <th />
                {cols.map((col) => (
                  <th key={col}>
                    <div className="rg-filter-cell">
                      <input
                        className="rg-filter-input"
                        placeholder="…"
                        value={filters[col] ?? ""}
                        onChange={(e) =>
                          setFilters((prev) => ({ ...prev, [col]: e.currentTarget.value }))
                        }
                      />
                      {filters[col] && (
                        <button
                          type="button"
                          className="rg-filter-x"
                          onClick={() =>
                            setFilters((prev) => {
                              const next = { ...prev };
                              delete next[col];
                              return next;
                            })
                          }
                          title={t("Clear")}
                        >
                          <X size={9} />
                        </button>
                      )}
                    </div>
                  </th>
                ))}
                {(editEnabled || insertEnabled) && <th />}
              </tr>
            )}
          </thead>
          <tbody>
            {pendingInserts.map((draft, i) => (
              <PendingInsertRow
                key={`pending-insert-${i}`}
                cols={cols}
                values={draft}
                pkSet={pkSet}
                numericSet={numericSet}
                onChange={(col, v) =>
                  setPendingInserts((prev) => {
                    const out = prev.slice();
                    out[i] = { ...out[i], [col]: v };
                    return out;
                  })
                }
                onCancel={() =>
                  setPendingInserts((prev) => prev.filter((_, idx) => idx !== i))
                }
                t={t}
              />
            ))}
            {slice.length === 0 && pendingInserts.length === 0 ? (
              <tr>
                <td
                  className="rg-empty"
                  colSpan={cols.length + 1 + (editEnabled || insertEnabled ? 1 : 0)}
                  style={{ textAlign: "center" }}
                >
                  {emptyLabel ?? t("No rows to show.")}
                </td>
              </tr>
            ) : (
              slice.map(({ row, absIdx }, sliceIdx) => {
                const isDeleted = pendingDeletes.has(absIdx);
                const displayIdx = safePage * pageSize + sliceIdx + 1;
                const rowIsDirty = isDeleted ||
                  Array.from(dirtyMap.values()).some((d) => d.row === absIdx);
                return (
                  <tr
                    key={absIdx}
                    className={
                      "rg-row" +
                      (isDeleted ? " rg-row-deleted" : "") +
                      (rowIsDirty && !isDeleted ? " rg-row-dirty" : "")
                    }
                    onClick={() => onOpenRow?.(row)}
                    style={{ cursor: onOpenRow ? "pointer" : undefined }}
                  >
                    <td className="rg-td-n">{displayIdx}</td>
                    {row.map((cell, ci) => {
                      const col = cols[ci];
                      const isPk = pkSet.has(col);
                      const isNum = numericSet.has(col);
                      const meta = columnsMeta?.find((m) => m.name === col);
                      const isEditing = editing?.row === absIdx && editing?.col === col;
                      const dirtyVal = dirtyValueFor(absIdx, col);
                      const isDirty = dirtyVal !== null;
                      const display = dirtyVal !== null ? dirtyVal : cell;
                      const isNull = display === null || display === "" || display === "NULL";
                      // JSONB / json / array-as-json values come
                      // back compact. Show the pretty-printed form
                      // on hover so the user can read it without
                      // expanding the row. Plain text returns null
                      // and the title attr just stays absent.
                      const prettyTip =
                        !isNull && typeof display === "string"
                          ? prettyJsonish(display)
                          : null;
                      // Per-cell editability — drives both the cursor
                      // hint and a hover title that explains *why* a
                      // cell can't be edited (locked, no PK, or PK).
                      const cellEditable = editEnabled && !isPk;
                      const lockHint = !editEnabled
                        ? !writable
                          ? t("Writes locked — unlock to double-click edit")
                          : !columnsMeta || pkSet.size === 0
                            ? t("This table has no primary key — inline edit is disabled.")
                            : null
                        : isPk
                          ? t("Primary key columns are not editable.")
                          : null;
                      const className =
                        "rg-td" +
                        (isNum ? " rg-td-num" : "") +
                        (isPk ? " rg-td-pk" : "") +
                        (isDirty ? " rg-td-dirty" : "") +
                        (isEditing ? " rg-td-editing" : "") +
                        (cellEditable ? " rg-td-editable" : "") +
                        (prettyTip ? " rg-td-jsonish" : "");
                      return (
                        <td
                          key={ci}
                          className={className}
                          style={{ textAlign: isNum ? "right" : "left" }}
                          title={prettyTip ?? lockHint ?? undefined}
                          onDoubleClick={(e) => {
                            e.stopPropagation();
                            startEdit(absIdx, col);
                          }}
                        >
                          {isEditing ? (
                            <CellEditor
                              initial={display ?? ""}
                              numeric={isNum}
                              enumValues={meta?.enumValues}
                              onCommit={(v) => commitCellEdit(absIdx, col, cell ?? "", v)}
                              onCancel={cancelEdit}
                            />
                          ) : isNull ? (
                            <span className="rg-null">NULL</span>
                          ) : (
                            String(display)
                          )}
                        </td>
                      );
                    })}
                    {(editEnabled || insertEnabled) && (
                      <td className="rg-td-acts" onClick={(e) => e.stopPropagation()}>
                        {editEnabled && (
                          <button
                            type="button"
                            className={"mini-button mini-button--ghost" + (isDeleted ? " is-active" : "")}
                            onClick={() => togglePendingDelete(absIdx)}
                            title={isDeleted ? t("Undo delete") : t("Delete row")}
                          >
                            {isDeleted ? <Undo2 size={10} /> : <Trash2 size={10} />}
                          </button>
                        )}
                      </td>
                    )}
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      <div className="rg-pager">
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={() => setPage(0)}
          disabled={safePage === 0}
          title={t("First page")}
        >
          «
        </button>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={() => setPage((p) => Math.max(0, p - 1))}
          disabled={safePage === 0}
          title={t("Previous page")}
        >
          <ArrowLeft size={10} />
        </button>
        <span className="rg-pager-n">
          {t("Page")} <b>{safePage + 1}</b>
          <span className="rg-stat-muted"> {t("of {n}", { n: pageCount })}</span>
          {filteredSorted.length > 0 && (
            <span className="rg-stat-muted">
              {" · "}{t("rows {from}–{to}", {
                from: safePage * pageSize + 1,
                to: Math.min(filteredSorted.length, (safePage + 1) * pageSize),
              })}
            </span>
          )}
        </span>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={() => setPage((p) => Math.min(pageCount - 1, p + 1))}
          disabled={safePage >= pageCount - 1}
          title={t("Next page")}
        >
          <ArrowRight size={10} />
        </button>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={() => setPage(pageCount - 1)}
          disabled={safePage >= pageCount - 1}
          title={t("Last page")}
        >
          »
        </button>
        <span className="rg-pager-spacer" />
        {dirtyCount > 0 && onCommit && (
          <>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={discardAll}
              disabled={committing}
            >
              <X size={10} /> {t("Discard")}
            </button>
            <button
              type="button"
              className="btn is-primary is-compact"
              onClick={() => void onCommitClick()}
              disabled={committing}
            >
              <Save size={10} /> {committing ? t("Committing...") : t("Commit {n} changes", { n: dirtyCount })}
            </button>
          </>
        )}
        <label className="rg-pager-size">
          <span className="rg-stat-muted">{t("page size")}</span>
          <select
            value={pageSize}
            onChange={(e) => {
              setPageSize(Number(e.currentTarget.value));
              setPage(0);
            }}
          >
            {PAGE_SIZES.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
      </div>
    </div>
  );
}

/** Inline cell editor — commits on blur or Enter, cancels on Escape.
 *  When `enumValues` is set, the input is wired to a `<datalist>` so
 *  the user gets a typeahead dropdown of valid enum members. The
 *  input is still free-form (so users can paste an unusual value if
 *  the catalog is stale), but the suggestions cover the happy path. */
function CellEditor({
  initial,
  numeric,
  enumValues,
  onCommit,
  onCancel,
}: {
  initial: string;
  numeric: boolean;
  enumValues?: string[];
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  const [val, setVal] = useState(initial);
  // Stable id so multiple grids on the page don't collide — the
  // datalist itself is rendered as a sibling node and referenced by
  // the input's `list=` attribute.
  const datalistId = useMemo(
    () => `rg-enum-${Math.random().toString(36).slice(2, 8)}`,
    [],
  );
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <>
      <input
        ref={ref}
        className={"rg-td-input" + (numeric ? " rg-td-input-num" : "")}
        value={val}
        list={enumValues && enumValues.length > 0 ? datalistId : undefined}
        onChange={(e) => setVal(e.currentTarget.value)}
        onBlur={() => onCommit(val)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            (e.currentTarget as HTMLInputElement).blur();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
      />
      {enumValues && enumValues.length > 0 && (
        <datalist id={datalistId}>
          {enumValues.map((v) => (
            <option key={v} value={v} />
          ))}
        </datalist>
      )}
    </>
  );
}

/** Pending-insert row rendered above the existing rows so the user can
 *  see what they're staging without scrolling. */
function PendingInsertRow({
  cols,
  values,
  pkSet,
  numericSet,
  onChange,
  onCancel,
  t,
}: {
  cols: string[];
  values: Record<string, string>;
  pkSet: Set<string>;
  numericSet: Set<string>;
  onChange: (col: string, v: string) => void;
  onCancel: () => void;
  t: (s: string, vars?: Record<string, string | number>) => string;
}) {
  return (
    <tr className="rg-row rg-row-new">
      <td className="rg-td-n">
        <span className="rg-row-badge">{t("NEW")}</span>
      </td>
      {cols.map((col) => {
        const isPk = pkSet.has(col);
        const isNum = numericSet.has(col);
        return (
          <td
            key={col}
            className={"rg-td rg-td-edit" + (isNum ? " rg-td-num" : "")}
            style={{ textAlign: isNum ? "right" : "left" }}
          >
            <input
              className={"rg-td-input" + (isNum ? " rg-td-input-num" : "")}
              placeholder={isPk ? t("auto") : t("NULL")}
              value={values[col] ?? ""}
              onChange={(e) => onChange(col, e.currentTarget.value)}
            />
          </td>
        );
      })}
      <td className="rg-td-acts" onClick={(e) => e.stopPropagation()}>
        <button
          type="button"
          className="mini-button mini-button--ghost"
          title={t("Discard insert")}
          onClick={onCancel}
        >
          <X size={10} />
        </button>
        <span className="rg-row-staged">
          <Check size={10} />
        </span>
      </td>
    </tr>
  );
}
