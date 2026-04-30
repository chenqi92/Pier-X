import { useEffect, useMemo, useState } from "react";
import { Activity, RefreshCw, Square, X } from "lucide-react";
import { useI18n } from "../../i18n/useI18n";
import { localizeError } from "../../i18n/localizeMessage";
import * as cmd from "../../lib/commands";
import type { MysqlProcessRow } from "../../lib/commands";

type Props = {
  open: boolean;
  onClose: () => void;
  connection: {
    host: string;
    port: number;
    user: string;
    password: string;
    database?: string | null;
  };
};

function formatSeconds(s: number): string {
  if (s < 1) return `${s} s`;
  if (s < 60) return `${s} s`;
  const m = s / 60;
  if (m < 60) return `${m.toFixed(1)} m`;
  const h = m / 60;
  return `${h.toFixed(1)} h`;
}

function severity(s: number, command: string | null): "" | "warn" | "alert" {
  // Sleep / Binlog Dump can sit for hours legitimately; only flag
  // long-running Query/Execute/Locked sessions.
  const cmd = (command ?? "").toLowerCase();
  const watchful =
    cmd === "query" || cmd === "execute" || cmd === "killed" || cmd === "init db";
  if (!watchful) return "";
  if (s >= 60) return "alert";
  if (s >= 5) return "warn";
  return "";
}

export default function MysqlProcessListDialog({
  open,
  onClose,
  connection,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const [rows, setRows] = useState<MysqlProcessRow[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [actingId, setActingId] = useState<number | null>(null);
  const [actionError, setActionError] = useState("");
  const [filter, setFilter] = useState<"all" | "active" | "long">("all");
  const [autoRefresh, setAutoRefresh] = useState(false);

  const refresh = async () => {
    if (loading) return;
    setLoading(true);
    setError("");
    try {
      const list = await cmd.mysqlListProcesses(connection);
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

  useEffect(() => {
    if (!open || !autoRefresh) return;
    const id = window.setInterval(() => {
      void refresh();
    }, 5000);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, autoRefresh]);

  const visible = useMemo(() => {
    if (!rows) return [];
    if (filter === "active") {
      return rows.filter((r) => {
        const c = (r.command ?? "").toLowerCase();
        return c !== "sleep" && c !== "binlog dump" && c !== "binlog dump gtid";
      });
    }
    if (filter === "long") {
      return rows.filter((r) => r.timeSeconds >= 5);
    }
    return rows;
  }, [rows, filter]);

  const handleKillQuery = async (id: number) => {
    if (actingId !== null) return;
    setActingId(id);
    setActionError("");
    try {
      await cmd.mysqlKillQuery({ ...connection, id });
      await new Promise((r) => setTimeout(r, 250));
      await refresh();
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setActingId(null);
    }
  };

  const handleKillConnection = async (id: number) => {
    if (actingId !== null) return;
    if (
      !window.confirm(
        t(
          "Force-kill connection {id}? The session will drop and any open transaction is rolled back.",
          { id },
        ),
      )
    ) {
      return;
    }
    setActingId(id);
    setActionError("");
    try {
      await cmd.mysqlKillConnection({ ...connection, id });
      await new Promise((r) => setTimeout(r, 250));
      await refresh();
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setActingId(null);
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
            <Activity size={14} /> {t("Server activity (SHOW PROCESSLIST)")}
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
              aria-selected={filter === "all"}
              className={`btn btn--ghost btn--sm ${filter === "all" ? "is-active" : ""}`}
              onClick={() => setFilter("all")}
            >
              {t("All")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={filter === "active"}
              className={`btn btn--ghost btn--sm ${filter === "active" ? "is-active" : ""}`}
              onClick={() => setFilter("active")}
              title={t("Hide Sleep / Binlog Dump backends")}
            >
              {t("Active")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={filter === "long"}
              className={`btn btn--ghost btn--sm ${filter === "long" ? "is-active" : ""}`}
              onClick={() => setFilter("long")}
              title={t("Sessions running ≥ 5s")}
            >
              {t("Long-running")}
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
            {rows ? t("{n} backends", { n: visible.length }) : ""}
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
          {!error && visible.length === 0 && !loading && (
            <div className="status-note mono">
              {t("(no matching backends)")}
            </div>
          )}
          {visible.map((r) => (
            <ProcessCard
              key={r.id}
              row={r}
              acting={actingId === r.id}
              onKillQuery={() => void handleKillQuery(r.id)}
              onKillConnection={() => void handleKillConnection(r.id)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function ProcessCard({
  row,
  acting,
  onKillQuery,
  onKillConnection,
}: {
  row: MysqlProcessRow;
  acting: boolean;
  onKillQuery: () => void;
  onKillConnection: () => void;
}) {
  const { t } = useI18n();
  const sev = severity(row.timeSeconds, row.command);
  const command = row.command ?? "—";
  const canKillQuery =
    !!row.command && row.command.toLowerCase() !== "sleep";

  return (
    <div className={`pg-activity__row pg-activity__row--${sev || "ok"}`}>
      <div className="pg-activity__head mono">
        <span className="pg-activity__pid">#{row.id}</span>
        <span className={`pg-activity__state pg-activity__state--${command.replace(/\W+/g, "-")}`}>
          {command}
        </span>
        <span className="pg-activity__user">
          {row.user ?? "—"}@{row.host ?? "—"}
        </span>
        {row.db && <span className="pg-activity__app">{row.db}</span>}
        <span className={`pg-activity__dur pg-activity__dur--${sev}`}>
          {formatSeconds(row.timeSeconds)}
        </span>
        {row.state && <span className="pg-activity__wait">{row.state}</span>}
        <span className="pg-activity__spacer" />
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onKillQuery}
          disabled={acting || !canKillQuery}
          title={t(
            "KILL QUERY {id} — interrupts the running statement",
            { id: row.id },
          )}
        >
          <Square size={11} /> {t("Cancel")}
        </button>
        <button
          type="button"
          className="btn btn--neg btn--sm"
          onClick={onKillConnection}
          disabled={acting}
          title={t(
            "KILL {id} — drops the entire session connection",
            { id: row.id },
          )}
        >
          {t("Terminate")}
        </button>
      </div>
      {row.info && (
        <pre className="pg-activity__sql mono" title={row.info}>
          {row.info}
        </pre>
      )}
    </div>
  );
}
