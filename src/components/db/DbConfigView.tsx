import { AlertTriangle, Lock, RefreshCw, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { useI18n } from "../../i18n/useI18n";

export type DbConfigRow = {
  name: string;
  value: string;
  /** Engine-specific origin of the setting (e.g. PG `context`, MySQL section). */
  source?: string;
  /** Optional human-readable description (PG `short_desc`). */
  description?: string;
  /** Whether the engine reports this setting as runtime-writable.
   *  MySQL = !readOnly && IS_DYNAMIC=YES; PG = `context` in `(user,
   *  superuser, sighup)`; SQLite = whitelisted PRAGMA names. Defaults
   *  to false so unknown rows stay safe. */
  editable?: boolean;
  /** Hint shown next to the lock icon when the row is not editable,
   *  or alongside the value when the row is editable but has caveats
   *  ("connection-scoped", "restart required", …). Free-form. */
  editHint?: string;
};

type Props = {
  /** Title shown in the head row (e.g. "MySQL variables"). */
  title: string;
  /** Loader invoked on mount + when the user clicks Refresh. */
  load: () => Promise<DbConfigRow[]>;
  /** Optional note shown next to the title (e.g. read-only badge text). */
  note?: string;
  /** When provided, editable rows render an inline editor that calls
   *  this on commit. The view re-loads (via the existing `load`
   *  loader) on success, so the panel doesn't have to thread state
   *  back. Reject the promise with a human-readable Error to surface
   *  an inline error band under the row. */
  onEdit?: (name: string, newValue: string) => Promise<void>;
};

/**
 * Read-only viewer for engine config / variables. Each panel passes a
 * loader that pulls from `*_execute` ("SHOW VARIABLES" / `pg_settings`
 * / `pragma_*`) and maps the rows into the engine-agnostic shape.
 *
 * Editing is intentionally out of scope for this pass — `SET GLOBAL`
 * (MySQL), `ALTER SYSTEM` (PG), and `PRAGMA name=value` (SQLite) all
 * have very different reload / permission semantics that are worth a
 * dedicated follow-up.
 */
export default function DbConfigView({ title, load, note, onEdit }: Props) {
  const { t } = useI18n();
  const [rows, setRows] = useState<DbConfigRow[] | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [q, setQ] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [pendingRow, setPendingRow] = useState<string | null>(null);
  const [rowError, setRowError] = useState<{ name: string; msg: string } | null>(null);

  const refresh = () => {
    setBusy(true);
    setError("");
    load()
      .then((r) => setRows(r))
      .catch((e) => {
        setRows(null);
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => setBusy(false));
  };

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function commitEdit(name: string, newValue: string) {
    if (!onEdit) {
      setEditing(null);
      return;
    }
    setEditing(null);
    setPendingRow(name);
    setRowError(null);
    try {
      await onEdit(name, newValue);
      // Optimistically patch the value so the user sees their change
      // even before the refresh round-trips. The refresh that follows
      // is the source of truth (some engines coerce / round values).
      setRows((prev) =>
        prev ? prev.map((r) => (r.name === name ? { ...r, value: newValue } : r)) : prev,
      );
      refresh();
    } catch (e) {
      setRowError({ name, msg: e instanceof Error ? e.message : String(e) });
    } finally {
      setPendingRow(null);
    }
  }

  const filtered = useMemo(() => {
    if (!rows) return [] as DbConfigRow[];
    const ql = q.trim().toLowerCase();
    if (!ql) return rows;
    return rows.filter(
      (r) =>
        r.name.toLowerCase().includes(ql) ||
        r.value.toLowerCase().includes(ql) ||
        (r.description ?? "").toLowerCase().includes(ql),
    );
  }, [rows, q]);

  return (
    <div className="db2-config">
      <div className="db2-config__head">
        <span className="db2-config__title">{title}</span>
        {note && <span className="db2-config__note">{note}</span>}
        <span className="db2-config__count">
          {rows ? rows.length.toLocaleString() : "—"}
        </span>
        <span className="db2-config__spacer" />
        <div className="db2-config__search">
          <Search size={11} />
          <input
            type="search"
            className="db2-config__search-input"
            placeholder={t("Filter…")}
            value={q}
            onChange={(e) => setQ(e.currentTarget.value)}
          />
        </div>
        <button
          type="button"
          className="btn is-ghost is-compact"
          disabled={busy}
          onClick={refresh}
          title={t("Refresh")}
        >
          <RefreshCw size={10} className={busy ? "db2-config__spin" : undefined} />
          {t("Refresh")}
        </button>
      </div>
      {error ? (
        <div className="db2-config__error">
          <AlertTriangle size={12} />
          <span>{error}</span>
        </div>
      ) : busy && !rows ? (
        <div className="db2-config__hint">{t("Loading…")}</div>
      ) : filtered.length === 0 ? (
        <div className="db2-config__hint">
          {q ? t("No settings match the filter.") : t("No settings available.")}
        </div>
      ) : (
        <div className="db2-config__scroll">
          <table className="rg-table db2-config__table">
            <thead>
              <tr>
                <th>
                  <div className="rg-th-body">
                    <span className="rg-th-name">{t("Name")}</span>
                  </div>
                </th>
                <th>
                  <div className="rg-th-body">
                    <span className="rg-th-name">{t("Value")}</span>
                  </div>
                </th>
                <th>
                  <div className="rg-th-body">
                    <span className="rg-th-name">{t("Source")}</span>
                  </div>
                </th>
                <th>
                  <div className="rg-th-body">
                    <span className="rg-th-name">{t("Description")}</span>
                  </div>
                </th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((r) => {
                const rowEditable = !!onEdit && r.editable === true;
                const isEditing = editing === r.name;
                const isPending = pendingRow === r.name;
                const hasRowError = rowError?.name === r.name;
                return (
                  <tr key={r.name} className="rg-row">
                    <td
                      className="rg-td db2-config__td-name"
                      title={r.name}
                    >
                      {r.name}
                    </td>
                    <td
                      className={
                        "rg-td db2-config__td-value" +
                        (rowEditable ? " rg-td-editable" : "") +
                        (isPending ? " db2-config__td-value--pending" : "")
                      }
                      title={
                        rowEditable
                          ? (r.editHint
                              ? t("Click to edit · {hint}", { hint: r.editHint })
                              : t("Click to edit"))
                          : (r.editHint || r.value)
                      }
                      onClick={() => {
                        if (!rowEditable || isEditing || isPending) return;
                        setEditing(r.name);
                        setRowError(null);
                      }}
                    >
                      {isEditing ? (
                        <ConfigCellEditor
                          initial={r.value}
                          onCommit={(v) => void commitEdit(r.name, v)}
                          onCancel={() => setEditing(null)}
                        />
                      ) : (
                        <>
                          {r.value === "" ? <span className="rg-null">∅</span> : r.value}
                          {!rowEditable && onEdit && (
                            <Lock size={9} className="db2-config__lock" />
                          )}
                          {hasRowError && (
                            <span className="db2-config__row-error">{rowError!.msg}</span>
                          )}
                        </>
                      )}
                    </td>
                    <td className="rg-td db2-config__td-source">{r.source || "—"}</td>
                    <td className="rg-td db2-config__td-desc">{r.description || "—"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** Inline editor for a single config value. Mirrors the
 *  rename / type editors in the structure view: focuses on mount,
 *  commits on Enter or blur, cancels on Escape. */
function ConfigCellEditor({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
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
      className="rg-td-input"
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
