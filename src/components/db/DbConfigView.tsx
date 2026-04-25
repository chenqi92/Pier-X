import { AlertTriangle, RefreshCw, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { useI18n } from "../../i18n/useI18n";

export type DbConfigRow = {
  name: string;
  value: string;
  /** Engine-specific origin of the setting (e.g. PG `context`, MySQL section). */
  source?: string;
  /** Optional human-readable description (PG `short_desc`). */
  description?: string;
};

type Props = {
  /** Title shown in the head row (e.g. "MySQL variables"). */
  title: string;
  /** Loader invoked on mount + when the user clicks Refresh. */
  load: () => Promise<DbConfigRow[]>;
  /** Optional note shown next to the title (e.g. read-only badge text). */
  note?: string;
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
export default function DbConfigView({ title, load, note }: Props) {
  const { t } = useI18n();
  const [rows, setRows] = useState<DbConfigRow[] | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [q, setQ] = useState("");

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
              {filtered.map((r) => (
                <tr key={r.name} className="rg-row">
                  <td className="rg-td db2-config__td-name">{r.name}</td>
                  <td className="rg-td db2-config__td-value">
                    {r.value === "" ? <span className="rg-null">∅</span> : r.value}
                  </td>
                  <td className="rg-td db2-config__td-source">{r.source || "—"}</td>
                  <td className="rg-td db2-config__td-desc">{r.description || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
