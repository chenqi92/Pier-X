import { useEffect, useMemo, useState } from "react";
import { Activity, RefreshCw, Square, X } from "lucide-react";
import { useI18n } from "../../i18n/useI18n";
import { localizeError } from "../../i18n/localizeMessage";
import * as cmd from "../../lib/commands";
import type { PgActivityRow } from "../../lib/commands";

type Props = {
  open: boolean;
  onClose: () => void;
  /** Connection params — reused for the activity query AND for
   *  cancel / terminate. The dialog runs each call as a fresh
   *  connection so it doesn't fight with the panel's editor session. */
  connection: {
    host: string;
    port: number;
    user: string;
    password: string;
    database?: string | null;
  };
};

function formatDuration(ms: number | null): string {
  if (ms === null) return "—";
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  const m = s / 60;
  if (m < 60) return `${m.toFixed(1)} m`;
  const h = m / 60;
  return `${h.toFixed(1)} h`;
}

function durationSeverity(ms: number | null): "" | "warn" | "alert" {
  if (ms === null) return "";
  if (ms >= 60_000) return "alert";
  if (ms >= 5_000) return "warn";
  return "";
}

export default function PostgresActivityDialog({
  open,
  onClose,
  connection,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const [rows, setRows] = useState<PgActivityRow[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [actingPid, setActingPid] = useState<number | null>(null);
  const [actionError, setActionError] = useState("");
  const [stateFilter, setStateFilter] = useState<"all" | "active" | "idleTx">(
    "all",
  );
  const [autoRefresh, setAutoRefresh] = useState(false);

  const refresh = async () => {
    if (loading) return;
    setLoading(true);
    setError("");
    try {
      const list = await cmd.postgresListActivity(connection);
      setRows(list);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!open) return;
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Optional 5s auto-refresh — opt-in so we don't burn extra
  // backend connections on every open.
  useEffect(() => {
    if (!open || !autoRefresh) return;
    const id = window.setInterval(() => {
      void refresh();
    }, 5000);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, autoRefresh]);

  const visibleRows = useMemo(() => {
    if (!rows) return [];
    if (stateFilter === "active") {
      return rows.filter((r) => r.state === "active");
    }
    if (stateFilter === "idleTx") {
      return rows.filter(
        (r) => r.state === "idle in transaction" ||
               r.state === "idle in transaction (aborted)",
      );
    }
    return rows;
  }, [rows, stateFilter]);

  const handleCancel = async (pid: number) => {
    if (actingPid !== null) return;
    setActingPid(pid);
    setActionError("");
    try {
      await cmd.postgresCancelQuery({ ...connection, pid });
      // Brief pause then refresh so the user sees the result.
      await new Promise((r) => setTimeout(r, 250));
      await refresh();
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setActingPid(null);
    }
  };

  const handleTerminate = async (pid: number) => {
    if (actingPid !== null) return;
    if (
      !window.confirm(
        t(
          "Force-terminate backend {pid}? The connection will drop and any open transaction is rolled back.",
          { pid },
        ),
      )
    ) {
      return;
    }
    setActingPid(pid);
    setActionError("");
    try {
      await cmd.postgresTerminateBackend({ ...connection, pid });
      await new Promise((r) => setTimeout(r, 250));
      await refresh();
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setActingPid(null);
    }
  };

  if (!open) return null;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="dlg pg-activity"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="dlg-head">
          <span className="dlg-title">
            <Activity size={14} /> {t("Server activity (pg_stat_activity)")}
          </span>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={onClose}
            title={t("Close")}
          >
            <X size={12} />
          </button>
        </div>

        <div className="pg-activity__toolbar mono">
          <div className="pg-activity__filter" role="tablist">
            <button
              type="button"
              role="tab"
              aria-selected={stateFilter === "all"}
              className={`btn btn--ghost btn--sm ${stateFilter === "all" ? "is-active" : ""}`}
              onClick={() => setStateFilter("all")}
            >
              {t("All")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={stateFilter === "active"}
              className={`btn btn--ghost btn--sm ${stateFilter === "active" ? "is-active" : ""}`}
              onClick={() => setStateFilter("active")}
            >
              {t("Active")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={stateFilter === "idleTx"}
              className={`btn btn--ghost btn--sm ${stateFilter === "idleTx" ? "is-active" : ""}`}
              onClick={() => setStateFilter("idleTx")}
              title={t("Idle in transaction — long ones are usually a leak")}
            >
              {t("Idle-Tx")}
            </button>
          </div>
          <label className="pg-activity__auto">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
            />
            <span className="mono">{t("Auto-refresh 5s")}</span>
          </label>
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => void refresh()}
            disabled={loading}
            title={t("Refresh")}
          >
            <RefreshCw size={11} /> {loading ? t("…") : t("Refresh")}
          </button>
          <span className="pg-activity__count">
            {rows ? t("{n} backends", { n: visibleRows.length }) : ""}
          </span>
        </div>

        {error && (
          <div className="status-note mono status-note--error">{error}</div>
        )}
        {actionError && (
          <div className="status-note mono status-note--error">
            {actionError}
          </div>
        )}

        <div className="pg-activity__body">
          {!error && visibleRows.length === 0 && !loading && (
            <div className="status-note mono">
              {t("(no matching backends)")}
            </div>
          )}
          {visibleRows.map((r) => (
            <ActivityCard
              key={r.pid}
              row={r}
              acting={actingPid === r.pid}
              onCancel={() => void handleCancel(r.pid)}
              onTerminate={() => void handleTerminate(r.pid)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function ActivityCard({
  row,
  acting,
  onCancel,
  onTerminate,
}: {
  row: PgActivityRow;
  acting: boolean;
  onCancel: () => void;
  onTerminate: () => void;
}) {
  const { t } = useI18n();
  const sev = durationSeverity(row.queryDurationMs);
  const stateLabel = row.state ?? "(no state)";
  const isIdleTx =
    row.state === "idle in transaction" ||
    row.state === "idle in transaction (aborted)";

  return (
    <div className={`pg-activity__row pg-activity__row--${sev || "ok"}`}>
      <div className="pg-activity__head mono">
        <span className="pg-activity__pid">#{row.pid}</span>
        <span className={`pg-activity__state pg-activity__state--${stateLabel.replace(/\W+/g, "-")}`}>
          {stateLabel}
        </span>
        <span className="pg-activity__user">
          {row.usename ?? "—"}@{row.datname ?? "—"}
        </span>
        {row.clientAddr && (
          <span className="pg-activity__addr">{row.clientAddr}</span>
        )}
        {row.applicationName && (
          <span className="pg-activity__app">{row.applicationName}</span>
        )}
        <span className={`pg-activity__dur pg-activity__dur--${sev}`}>
          {t("query")}: {formatDuration(row.queryDurationMs)}
        </span>
        {isIdleTx && row.stateDurationMs !== null && (
          <span className="pg-activity__dur pg-activity__dur--warn">
            {t("in-tx")}: {formatDuration(row.stateDurationMs)}
          </span>
        )}
        {row.waitEvent && (
          <span className="pg-activity__wait" title={row.waitEventType ?? ""}>
            {row.waitEventType ?? "?"}: {row.waitEvent}
          </span>
        )}
        <span className="pg-activity__spacer" />
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onCancel}
          disabled={acting}
          title={t(
            "pg_cancel_backend({pid}) — interrupts the running statement",
            { pid: row.pid },
          )}
        >
          <Square size={11} /> {t("Cancel")}
        </button>
        <button
          type="button"
          className="btn btn--neg btn--sm"
          onClick={onTerminate}
          disabled={acting}
          title={t(
            "pg_terminate_backend({pid}) — drops the entire connection",
            { pid: row.pid },
          )}
        >
          {t("Terminate")}
        </button>
      </div>
      {row.query && (
        <pre className="pg-activity__sql mono" title={row.query}>
          {row.query}
        </pre>
      )}
    </div>
  );
}
