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
  const [pendingInsert, setPendingInsert] = useState<Record<string, string> | null>(null);
  const [pendingDeletes, setPendingDeletes] = useState<Set<number>>(new Set());

  // Reset paging + edit state when the preview swaps under us — otherwise
  // switching tables can leave us showing "page 12 of 3" or stale dirty
  // cells pointing at a different shape.
  useEffect(() => {
    setPage(0);
    setSortCol(null);
    setFilters({});
    setEditing(null);
    setDirtyMap(new Map());
    setPendingInsert(null);
    setPendingDeletes(new Set());
  }, [preview]);

  const numericSet = useMemo(() => new Set(numericColumns ?? []), [numericColumns]);
  const pkSet = useMemo(() => new Set(pkColumns ?? []), [pkColumns]);

  const cols = preview?.columns ?? columnsMeta?.map((c) => c.name) ?? [];
  const editEnabled = writable && !!columnsMeta && pkSet.size > 0 && !!onCommit;
  const insertEnabled = writable && !!columnsMeta && !!onCommit;

  const filteredSorted = useMemo(() => {
    if (!preview) return [] as { row: string[]; absIdx: number }[];
    let pairs = preview.rows.map((row, absIdx) => ({ row, absIdx }));
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
  }, [preview, filters, sortCol, sortDir, cols, numericSet]);

  const pageCount = Math.max(1, Math.ceil(filteredSorted.length / pageSize));
  const safePage = Math.min(page, pageCount - 1);
  const slice = useMemo(
    () => filteredSorted.slice(safePage * pageSize, (safePage + 1) * pageSize),
    [filteredSorted, safePage, pageSize],
  );

  const dirtyCount = dirtyMap.size + (pendingInsert ? 1 : 0) + pendingDeletes.size;

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
    if (!insertEnabled || pendingInsert) return;
    const init: Record<string, string> = {};
    for (const c of cols) init[c] = "";
    setPendingInsert(init);
  };

  const discardAll = () => {
    setDirtyMap(new Map());
    setPendingInsert(null);
    setPendingDeletes(new Set());
    setEditing(null);
  };

  const collectMutations = useCallback((): DbMutation[] => {
    if (!preview) return [];
    const muts: DbMutation[] = [];
    // Collect cell edits per row
    const byRow = new Map<number, DirtyCell[]>();
    for (const cell of dirtyMap.values()) {
      const list = byRow.get(cell.row) ?? [];
      list.push(cell);
      byRow.set(cell.row, list);
    }
    for (const [rowIdx, cells] of byRow.entries()) {
      const original = preview.rows[rowIdx];
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
    // Pending inserts
    if (pendingInsert) {
      const values: Record<string, string | null> = {};
      for (const c of cols) {
        const v = pendingInsert[c];
        // Skip empty + PK columns (let the DB auto-generate). For non-PK
        // empty columns, send NULL so the DB applies its default.
        if (pkSet.has(c) && (v === undefined || v === "")) continue;
        values[c] = v === undefined || v === "" ? null : v;
      }
      muts.push({ kind: "insert", values });
    }
    // Pending deletes
    for (const rowIdx of pendingDeletes) {
      const original = preview.rows[rowIdx];
      if (!original) continue;
      const pk: Record<string, string> = {};
      for (const c of cols) {
        if (pkSet.has(c)) pk[c] = original[cols.indexOf(c)] ?? "";
      }
      muts.push({ kind: "delete", pk });
    }
    return muts;
  }, [preview, dirtyMap, pendingInsert, pendingDeletes, cols, pkSet]);

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
        {insertEnabled && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={startInsert}
            disabled={!!pendingInsert}
            title={t("Insert row")}
          >
            <Plus size={10} /> {t("Insert row")}
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
          <thead>
            <tr>
              <th className="rg-th-n">#</th>
              {cols.map((col) => {
                const isPk = pkSet.has(col);
                const isNum = numericSet.has(col);
                const align = isNum ? "right" : "left";
                const sorted = sortCol === col;
                return (
                  <th
                    key={col}
                    className={"rg-th" + (sorted ? " rg-th-sorted" : "")}
                    style={{ textAlign: align }}
                    onClick={() => toggleSort(col)}
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
            {pendingInsert && (
              <PendingInsertRow
                cols={cols}
                values={pendingInsert}
                pkSet={pkSet}
                numericSet={numericSet}
                onChange={(col, v) =>
                  setPendingInsert((prev) => (prev ? { ...prev, [col]: v } : prev))
                }
                onCancel={() => setPendingInsert(null)}
                t={t}
              />
            )}
            {slice.length === 0 && !pendingInsert ? (
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

/** Inline cell editor — commits on blur or Enter, cancels on Escape. */
function CellEditor({
  initial,
  numeric,
  onCommit,
  onCancel,
}: {
  initial: string;
  numeric: boolean;
  onCommit: (v: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  const [val, setVal] = useState(initial);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className={"rg-td-input" + (numeric ? " rg-td-input-num" : "")}
      value={val}
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
