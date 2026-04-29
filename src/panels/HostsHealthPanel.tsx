// Top-level "host health" dashboard. Lists every saved SSH
// connection and shows whether its SSH port is currently reachable
// from this machine via a single TCP probe per host. Pure triage
// surface — credential-free, no SSH handshake; click-through into a
// real terminal tab for the deep dive.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Download,
  KeyRound,
  Loader2,
  Pencil,
  Plug,
  RefreshCw,
  Search,
  Server,
  Square,
} from "lucide-react";

import { useI18n } from "../i18n/useI18n";
import ContextMenu, { type ContextMenuItem } from "../components/ContextMenu";
import { writeClipboardText } from "../lib/clipboard";
import * as cmd from "../lib/commands";
import EnvTagChip from "../components/EnvTagChip";
import Sparkline from "../components/Sparkline";
import { desktopNotify } from "../lib/notify";
import { toast } from "../stores/useToastStore";
import { useConnectionStore } from "../stores/useConnectionStore";
import type { SavedSshConnection, TabState } from "../lib/types";

type Props = {
  tab: TabState;
  isActive: boolean;
  onConnectSaved: (index: number) => void;
  onEditConnection: (index: number) => void;
  onNewConnection: () => void;
};

type ProbeMap = Record<number, cmd.HostHealthReport>;

// Refresh once per minute by default — the dashboard is supposed to
// reflect a recent state without becoming a sustained background
// load on the user's network. The manual Refresh button is the
// fast path; this interval just keeps the picture from getting
// arbitrarily stale.
const AUTO_REFRESH_MS = 60_000;

// 3-second TCP probe is the sweet spot: long enough that a slowly
// responding firewall doesn't flag a healthy host as `Timeout`, but
// short enough that a 50-host batch still finishes within ~5 s
// because probes run concurrently.
const PROBE_TIMEOUT_MS = 3000;
/** Sparkline retention: enough to show a few minutes of auto-refresh
 *  cadence (60s × 30 = 30 min) without making the localStorage write
 *  hot path expensive. Trimmed inside the probe callback. */
const LATENCY_HISTORY_CAP = 30;
/** Persistence bucket for the rolling latency window. Keyed by saved-
 *  connection index, so removing a connection naturally orphans its
 *  history — small enough that we don't bother garbage-collecting. */
const LATENCY_HISTORY_KEY = "pier-x:hosts-latency-history";

export default function HostsHealthPanel({
  isActive,
  onConnectSaved,
  onEditConnection,
  onNewConnection,
}: Props) {
  const { t } = useI18n();
  const connections = useConnectionStore((s) => s.connections);
  const refreshConnections = useConnectionStore((s) => s.refresh);

  const [probes, setProbes] = useState<ProbeMap>({});
  // Per-host rolling window of recent latencies. `null` entries
  // mark probes that came back offline / timeout — kept in the
  // history so the sparkline can show drops as gaps rather than
  // synthetic zeros. Capped at LATENCY_HISTORY_CAP samples per host.
  // Persisted to localStorage so the trail survives a panel remount
  // / app restart — without it the spark line is empty on every
  // first probe and the user has to wait several auto-refresh ticks
  // to see anything useful.
  const [latencyHistory, setLatencyHistory] = useState<
    Record<number, (number | null)[]>
  >(() => {
    try {
      const raw = localStorage.getItem(LATENCY_HISTORY_KEY);
      if (!raw) return {};
      const parsed = JSON.parse(raw);
      if (!parsed || typeof parsed !== "object") return {};
      const out: Record<number, (number | null)[]> = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (!Array.isArray(v)) continue;
        const idx = Number(k);
        if (!Number.isFinite(idx)) continue;
        out[idx] = v
          .filter((x) => x === null || (typeof x === "number" && Number.isFinite(x)))
          .slice(-LATENCY_HISTORY_CAP) as (number | null)[];
      }
      return out;
    } catch {
      return {};
    }
  });
  useEffect(() => {
    try {
      if (Object.keys(latencyHistory).length === 0) {
        localStorage.removeItem(LATENCY_HISTORY_KEY);
      } else {
        localStorage.setItem(
          LATENCY_HISTORY_KEY,
          JSON.stringify(latencyHistory),
        );
      }
    } catch {
      /* localStorage full — silent, history is best-effort */
    }
  }, [latencyHistory]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [filter, setFilter] = useState("");
  // Deep-probe results keyed by savedConnectionIndex. Populated
  // lazily on the per-row "Deep probe" button — the dashboard
  // never auto-runs deep probes because they require an existing
  // SSH session and the dashboard is a no-credentials view.
  const [deepProbes, setDeepProbes] = useState<
    Record<number, cmd.HostDeepProbeReport | "no-cached" | "running">
  >({});

  const runDeepProbe = useCallback(async (index: number) => {
    setDeepProbes((prev) => ({ ...prev, [index]: "running" }));
    try {
      const result = await cmd.hostHealthDeepProbe(index);
      setDeepProbes((prev) => ({
        ...prev,
        [index]: result === null ? "no-cached" : result,
      }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setDeepProbes((prev) => {
        const next = { ...prev };
        delete next[index];
        return next;
      });
    }
  }, []);
  // "grouped" buckets rows by `conn.group`; "flat" keeps the flat
  // list. Default is grouped because most users actually use the
  // group label — and a flat 30-host list is harder to scan than
  // 4 buckets of 8 each.
  const [viewMode, setViewMode] = useState<"grouped" | "flat" | "bus">(
    "grouped",
  );
  // Bus-view sort state. Click a column header to cycle:
  // unsorted → asc → desc → unsorted. We treat "unsorted" as
  // input order (i.e. the same order `filtered` produces) so
  // users always have a path back to the natural list.
  //
  // Persists to sessionStorage so re-opening the dashboard tab
  // (or the user briefly switching to flat/grouped and back)
  // keeps the sort the user picked. We deliberately use session
  // (not local) storage: a fresh launch shouldn't inherit a
  // session-specific sort that was useful for triaging one
  // incident.
  const BUS_SORT_KEY = "pier-x.hosts-health.bus-sort";
  const [busSort, setBusSort] = useState<{
    column: "name" | "host" | "latency" | "status" | "auth" | "group";
    dir: "asc" | "desc";
  } | null>(() => {
    try {
      const raw = window.sessionStorage.getItem(BUS_SORT_KEY);
      if (!raw) return null;
      const parsed = JSON.parse(raw);
      const validCols = ["name", "host", "latency", "status", "auth", "group"];
      if (
        parsed &&
        typeof parsed === "object" &&
        validCols.includes(parsed.column) &&
        (parsed.dir === "asc" || parsed.dir === "desc")
      ) {
        return parsed;
      }
    } catch {
      // Storage disabled (private browsing, custom CSP) →
      // silently fall through to default. Same fallback if the
      // value got hand-edited into something invalid.
    }
    return null;
  });
  // Mirror every change to sessionStorage. Skipping `null` would
  // leave a stale entry behind after the user clears the sort,
  // so we always write — `removeItem` for null, `setItem` for
  // a real value.
  useEffect(() => {
    try {
      if (busSort === null) {
        window.sessionStorage.removeItem(BUS_SORT_KEY);
      } else {
        window.sessionStorage.setItem(BUS_SORT_KEY, JSON.stringify(busSort));
      }
    } catch {
      /* see read-side fallback */
    }
  }, [busSort]);
  function cycleBusSort(
    column:
      | "name"
      | "host"
      | "latency"
      | "status"
      | "auth"
      | "group",
  ) {
    setBusSort((prev) => {
      if (!prev || prev.column !== column) return { column, dir: "asc" };
      if (prev.dir === "asc") return { column, dir: "desc" };
      return null; // third click clears the sort
    });
  }
  // Collapsed-group state lives in the component, not the store —
  // it's a per-tab UI preference, not something to sync across
  // hosts. The set holds group keys (the literal `group` string,
  // or the empty string for ungrouped).
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(
    () => new Set(),
  );
  /** Right-click context menu anchor + target connection.
   *  `null` = closed. Single-instance is fine because we never
   *  show two menus at once. */
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    conn: SavedSshConnection;
  } | null>(null);

  /** Multi-select state for the Bus view. Set of saved-connection
   *  indices. We only enable selection in the Bus view because
   *  that's where it makes sense visually (table row + checkbox);
   *  flat / grouped lists already crowd their rows.
   *
   *  The set is cleared when the user switches away from Bus view
   *  to avoid invisible state — rejoining the view starts fresh. */
  const [busSelected, setBusSelected] = useState<Set<number>>(
    () => new Set(),
  );
  useEffect(() => {
    if (viewMode !== "bus") setBusSelected(new Set());
  }, [viewMode]);

  /** Toggle a single host's selection bit. */
  function toggleBusSelected(idx: number) {
    setBusSelected((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }

  /** Select / clear all currently-filtered Bus rows. The
   *  filtered subset (not the full saved list) is the pragmatic
   *  target — a user typing a filter and clicking "select all"
   *  expects to grab only the matches they're looking at. */
  function toggleBusSelectAll() {
    setBusSelected((prev) => {
      // If every filtered row is already selected, clear; else
      // select them all. Preserves any already-selected rows
      // that are filtered OUT (rare but defensive).
      const filteredIds = filtered.map((c) => c.index);
      const allSelected = filteredIds.every((i) => prev.has(i));
      const next = new Set(prev);
      if (allSelected) {
        for (const i of filteredIds) next.delete(i);
      } else {
        for (const i of filteredIds) next.add(i);
      }
      return next;
    });
  }

  /** Connect to every selected host — opens one SSH tab per
   *  index. Sequential calls so the existing SSH session cache
   *  has a chance to dedupe (`get_or_open_ssh_session`); also
   *  avoids hammering sshd's `MaxStartups` on a fleet-wide
   *  click. Clears the selection on completion so a second
   *  click doesn't unintentionally re-open the same set. A
   *  single toast announces the batch so the user has feedback
   *  while the new tabs spool up — opening 30 SSH tabs without
   *  acknowledgement feels broken. */
  /** Export the current saved-connection dashboard to CSV — latest
   *  probe + a small summary of the rolling latency window. Lives at
   *  the panel level rather than `lib/commands.ts` so we can lean on
   *  the in-memory `probes` and `latencyHistory` maps without a
   *  round-trip. CSV format follows the same RFC-4180 quoting as
   *  `queryResultToCsv` (CRLF + quoted-on-special-char). */
  async function exportHostsCsv() {
    if (connections.length === 0) return;
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.save({
        title: t("Export hosts dashboard"),
        defaultPath: "pier-x-hosts.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (typeof picked !== "string") return;
      const escape = (v: string): string =>
        /[,"\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
      const header = [
        "name",
        "user",
        "host",
        "port",
        "group",
        "envTag",
        "status",
        "latency_ms",
        "checked_at",
        "samples",
        "min_ms",
        "max_ms",
        "avg_ms",
        "fail_count",
      ];
      const rows: string[] = [header.join(",")];
      for (const c of connections) {
        const r = probes[c.index];
        const series = latencyHistory[c.index] ?? [];
        const numeric = series.filter(
          (v): v is number => v != null && Number.isFinite(v),
        );
        const min = numeric.length > 0 ? Math.min(...numeric) : "";
        const max = numeric.length > 0 ? Math.max(...numeric) : "";
        const avg =
          numeric.length > 0
            ? Math.round(
                numeric.reduce((a, b) => a + b, 0) / numeric.length,
              )
            : "";
        const fails = series.filter((v) => v == null).length;
        const cells = [
          c.name || `${c.user}@${c.host}`,
          c.user,
          c.host,
          String(c.port),
          c.group ?? "",
          c.envTag ?? "",
          r?.status ?? "",
          r?.latencyMs != null ? String(r.latencyMs) : "",
          r?.checkedAt ? new Date(r.checkedAt * 1000).toISOString() : "",
          String(series.length),
          String(min),
          String(max),
          String(avg),
          String(fails),
        ];
        rows.push(cells.map(escape).join(","));
      }
      const blob = rows.join("\r\n");
      await cmd.localWriteTextFile(picked, blob);
      toast.info(
        t("Exported {n} host(s) to {path}", {
          n: connections.length,
          path: picked,
        }),
      );
    } catch (e) {
      toast.warn(e instanceof Error ? e.message : String(e));
    }
  }

  function connectAllSelected() {
    if (busSelected.size === 0) return;
    // Snapshot before clearing so React's state batching can't
    // race the iteration.
    const ids = Array.from(busSelected);
    setBusSelected(new Set());
    const total = ids.length;
    let opened = 0;
    for (const i of ids) {
      onConnectSaved(i);
      opened += 1;
    }
    toast.info(t("Opened {n}/{total} SSH tabs", { n: opened, total }));
  }

  /** Bulk-fire a test webhook for every selected host. We iterate
   *  selected hosts × configured webhook URLs, firing one HTTP POST
   *  per cross product. Slow on big fleets * many endpoints — toast
   *  reports a running count so the user knows it's making progress.
   *  Disabled webhooks are skipped. Errors are aggregated into a
   *  single end-of-run toast so individual failures don't stack up. */
  const [webhookFiring, setWebhookFiring] = useState(false);
  async function fireWebhookForSelected() {
    if (busSelected.size === 0 || webhookFiring) return;
    const ids = Array.from(busSelected);
    setWebhookFiring(true);
    try {
      const cfg = await cmd.softwareWebhooksLoad();
      const endpoints = cfg.entries.filter(
        (e) => !e.disabled && e.url.trim().length > 0,
      );
      if (endpoints.length === 0) {
        toast.warn(
          t(
            "No active webhooks configured. Add one in Software → Webhooks first.",
          ),
        );
        return;
      }
      let ok = 0;
      let bad = 0;
      for (const i of ids) {
        const conn = connections.find((c) => c.index === i);
        if (!conn) continue;
        const hostStr = `${conn.user}@${conn.host}:${conn.port ?? 22}`;
        for (const ep of endpoints) {
          try {
            const r = await cmd.softwareWebhooksTestFire({
              url: ep.url.trim(),
              bodyTemplate: ep.bodyTemplate ?? "",
              headers: ep.headers ?? [],
              host: hostStr,
              hmacSecret: ep.hmacSecret ?? "",
            });
            if (r.error) bad += 1;
            else ok += 1;
          } catch {
            bad += 1;
          }
        }
      }
      toast.info(
        t("Fired webhooks: {ok} ok, {bad} failed", { ok, bad }),
      );
    } catch (e) {
      toast.warn(
        t("Could not load webhook config: {err}", {
          err: e instanceof Error ? e.message : String(e),
        }),
      );
    } finally {
      setWebhookFiring(false);
    }
  }

  function openHostMenu(
    e: React.MouseEvent,
    conn: SavedSshConnection,
  ) {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ x: e.clientX, y: e.clientY, conn });
  }

  function buildHostMenu(conn: SavedSshConnection): ContextMenuItem[] {
    // ssh user@host[:port] — port appears only when non-default
    // so the copied command runs as-is from a default OpenSSH
    // config. Keeps the canonical short form for the common case.
    const portPart = conn.port && conn.port !== 22 ? ` -p ${conn.port}` : "";
    const sshCmd = `ssh${portPart} ${conn.user}@${conn.host}`;
    return [
      {
        label: t("Connect"),
        action: () => onConnectSaved(conn.index),
      },
      {
        label: t("Re-probe"),
        action: () => void probe([conn.index]),
        disabled: busy,
      },
      {
        label: t("Deep probe"),
        action: () => void runDeepProbe(conn.index),
      },
      { divider: true },
      {
        label: t("Edit"),
        action: () => onEditConnection(conn.index),
      },
      {
        label: t("Copy SSH command"),
        action: () => {
          void writeClipboardText(sshCmd).then(() =>
            toast.info(t("Copied: {cmd}", { cmd: sshCmd })),
          );
        },
      },
    ];
  }

  function toggleGroup(key: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  // Pull a snapshot of saved connections on first mount. The
  // connection store is shared across the app — most callers have
  // already populated it — but the dashboard is happy to re-check.
  useEffect(() => {
    if (connections.length === 0) {
      void refreshConnections();
    }
  }, [connections.length, refreshConnections]);

  const probe = useCallback(
    async (indices: number[]) => {
      if (indices.length === 0) return;
      setBusy(true);
      setError("");
      try {
        const reports = await cmd.hostHealthProbe({
          indices,
          timeoutMs: PROBE_TIMEOUT_MS,
        });
        // Roll the latency history first — independent of the
        // online → offline desktop-notify path below, and we want
        // failed probes (latencyMs == null) to appear as gaps in the
        // sparkline.
        setLatencyHistory((prev) => {
          const next = { ...prev };
          for (const r of reports) {
            if (r.savedConnectionIndex < 0) continue;
            const series = next[r.savedConnectionIndex] ?? [];
            const sample = r.latencyMs ?? null;
            const grown = [...series, sample];
            next[r.savedConnectionIndex] = grown.slice(-LATENCY_HISTORY_CAP);
          }
          return next;
        });
        setProbes((prev) => {
          const next = { ...prev };
          for (const r of reports) {
            if (r.savedConnectionIndex >= 0) {
              const before = prev[r.savedConnectionIndex];
              next[r.savedConnectionIndex] = r;
              // Fire a desktop notification only on the
              // online → offline/timeout edge — the inverse
              // (offline → online) is good news the user can
              // discover at their own pace from the dashboard.
              // Skip the very first observation per host (when
              // `before` is undefined) so launching the app
              // with already-down hosts doesn't blast the user
              // with N popups.
              if (
                before &&
                before.status === "online" &&
                (r.status === "offline" || r.status === "timeout")
              ) {
                const conn = connections.find(
                  (c) => c.index === r.savedConnectionIndex,
                );
                const target =
                  conn?.name || (conn ? `${conn.user}@${conn.host}` : "?");
                desktopNotify(
                  "warning",
                  t("Host went offline: {target}", { target }),
                  r.errorMessage || t("TCP probe failed."),
                );
              }
            }
          }
          return next;
        });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    // `connections` and `t` participate in the transition-alert
    // path (target name resolution + localised body); React would
    // rebuild this callback on every change anyway, but explicit
    // deps stop ESLint from complaining.
    [connections, t],
  );

  // Initial probe + auto-refresh loop. Re-runs whenever the saved
  // connection list changes (someone added a host elsewhere) so the
  // dashboard immediately reflects the new row's state.
  const allIndices = useMemo(
    () => connections.map((c) => c.index),
    [connections],
  );

  // Keep a ref to the latest indices so the interval handler sees
  // the up-to-date list without re-arming on every probe completion.
  const indicesRef = useRef(allIndices);
  indicesRef.current = allIndices;

  useEffect(() => {
    if (!isActive) return;
    if (allIndices.length === 0) return;
    void probe(allIndices);
    const id = window.setInterval(() => {
      // Use the live ref — `allIndices` from the closure could be
      // stale if the user added/removed a host between refreshes.
      void probe(indicesRef.current);
    }, AUTO_REFRESH_MS);
    return () => {
      window.clearInterval(id);
    };
    // We intentionally re-run when `isActive` flips on so the user
    // gets a fresh set of probes the moment they return to the
    // dashboard tab; otherwise rely on the interval + manual button.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive, allIndices.length]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return connections;
    return connections.filter(
      (c) =>
        c.name.toLowerCase().includes(q) ||
        c.host.toLowerCase().includes(q) ||
        c.user.toLowerCase().includes(q) ||
        (c.group ?? "").toLowerCase().includes(q),
    );
  }, [connections, filter]);

  const counts = useMemo(() => {
    let online = 0;
    let offline = 0;
    let timeout = 0;
    let unknown = 0;
    for (const c of connections) {
      const r = probes[c.index];
      if (!r) {
        unknown += 1;
        continue;
      }
      if (r.status === "online") online += 1;
      else if (r.status === "offline") offline += 1;
      else if (r.status === "timeout") timeout += 1;
      else unknown += 1;
    }
    return { online, offline, timeout, unknown, total: connections.length };
  }, [connections, probes]);

  // Group filtered rows by `conn.group`. Empty / null group lands
  // in a synthetic "(default)" bucket at the top of the list so a
  // mixed-state catalog (some grouped, some not) still renders
  // top-down without surprises. Group order is the order in which
  // groups first appear in the connections array — preserves the
  // user's manual ordering from the sidebar.
  const groupedRows = useMemo(() => {
    type Group = {
      key: string;
      label: string;
      rows: SavedSshConnection[];
      online: number;
      offline: number;
      unknown: number;
    };
    const groups: Group[] = [];
    const indexByKey = new Map<string, number>();
    for (const c of filtered) {
      const key = (c.group ?? "").trim();
      let idx = indexByKey.get(key);
      if (idx === undefined) {
        idx = groups.length;
        indexByKey.set(key, idx);
        groups.push({
          key,
          label: key === "" ? "" /* localised at render time */ : key,
          rows: [],
          online: 0,
          offline: 0,
          unknown: 0,
        });
      }
      const g = groups[idx];
      g.rows.push(c);
      const r = probes[c.index];
      if (!r) g.unknown += 1;
      else if (r.status === "online") g.online += 1;
      else if (r.status === "offline" || r.status === "timeout")
        g.offline += 1;
      else g.unknown += 1;
    }
    return groups;
  }, [filtered, probes]);

  return (
    <section
      className="hosts-health-panel"
      style={{ display: isActive ? "flex" : "none" }}
    >
      <header className="hosts-health-panel__header">
        <div className="hosts-health-panel__title">
          <Activity size={16} />
          <span>{t("Host health")}</span>
          <span className="hosts-health-panel__subtitle">
            {t("TCP-only reachability across saved SSH connections")}
          </span>
        </div>
        <div className="hosts-health-panel__actions">
          <span className="hosts-health-panel__counts mono">
            <span className="meta-pill meta-pill--success">
              {t("Online {n}", { n: counts.online })}
            </span>
            <span className="meta-pill meta-pill--danger">
              {t("Offline {n}", { n: counts.offline + counts.timeout })}
            </span>
            <span className="meta-pill">
              {t("Unknown {n}", { n: counts.unknown })}
            </span>
            <span className="muted">
              {t("/ {total} total", { total: counts.total })}
            </span>
          </span>
          <button
            type="button"
            className="mini-button"
            onClick={() =>
              setViewMode((m) =>
                // Cycle: grouped → flat → bus → grouped. The
                // single button keeps the toolbar tidy and most
                // users only swap once per session anyway.
                m === "grouped" ? "flat" : m === "flat" ? "bus" : "grouped",
              )
            }
            title={
              viewMode === "grouped"
                ? t("Switch to a flat list — show all hosts ungrouped")
                : viewMode === "flat"
                  ? t(
                      "Switch to bus view — dense one-line-per-host table for 50+ hosts",
                    )
                  : t("Switch to grouped view — bucket by saved group label")
            }
          >
            {viewMode === "grouped"
              ? t("Flat list")
              : viewMode === "flat"
                ? t("Bus view")
                : t("Group by label")}
          </button>
          <button
            type="button"
            className="mini-button"
            onClick={() => void probe(allIndices)}
            disabled={busy || allIndices.length === 0}
            title={t("Re-probe all saved connections now")}
          >
            {busy ? (
              <Loader2 size={12} className="hosts-health-spin" />
            ) : (
              <RefreshCw size={12} />
            )}
            <span>{t("Refresh")}</span>
          </button>
          <button
            type="button"
            className="mini-button"
            onClick={() => void exportHostsCsv()}
            disabled={connections.length === 0}
            title={t(
              "Export the current dashboard (latest probe + recent latency stats) to a CSV file.",
            )}
          >
            <Download size={12} /> <span>{t("Export CSV")}</span>
          </button>
          <button
            type="button"
            className="mini-button"
            onClick={onNewConnection}
            title={t("Add a new SSH connection to the saved list")}
          >
            <KeyRound size={12} /> <span>{t("New connection")}</span>
          </button>
        </div>
      </header>

      <div className="hosts-health-panel__filterbar">
        <Search size={12} className="muted" />
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("Filter by name, host, user, group…")}
          className="hosts-health-filter"
        />
      </div>

      {error && (
        <div className="status-note status-note--error mono hosts-health-panel__error">
          {error}
        </div>
      )}

      {connections.length === 0 ? (
        <div className="hosts-health-panel__empty">
          <Server size={28} className="muted" />
          <div className="hosts-health-panel__empty-title">
            {t("No saved SSH connections yet")}
          </div>
          <div className="muted">
            {t(
              "Add hosts from the sidebar or via 新建连接 to populate this dashboard.",
            )}
          </div>
          <button
            type="button"
            className="primary-button"
            onClick={onNewConnection}
          >
            <KeyRound size={12} /> {t("New connection")}
          </button>
        </div>
      ) : viewMode === "bus" ? (
        <div className="hosts-health-bus">
          {busSelected.size > 0 && (
            <div className="hosts-health-bus__selectionbar">
              <span className="mono muted">
                {t("{n} selected", { n: busSelected.size })}
              </span>
              <div className="hosts-health-bus__selectionbar-actions">
                <button
                  type="button"
                  className="mini-button"
                  onClick={() => setBusSelected(new Set())}
                >
                  {t("Clear")}
                </button>
                <button
                  type="button"
                  className="mini-button"
                  onClick={() => void probe(Array.from(busSelected))}
                  disabled={busy}
                  title={t("Re-probe all selected hosts")}
                >
                  {t("Re-probe selected")}
                </button>
                <button
                  type="button"
                  className="mini-button mini-button--primary"
                  onClick={connectAllSelected}
                  title={t(
                    "Open one new SSH tab for each selected host",
                  )}
                >
                  {t("Connect to {n}", { n: busSelected.size })}
                </button>
                <button
                  type="button"
                  className="mini-button"
                  onClick={() => void fireWebhookForSelected()}
                  disabled={webhookFiring}
                  title={t(
                    "Fire a test webhook for every configured endpoint, once per selected host",
                  )}
                >
                  {webhookFiring
                    ? t("Firing…")
                    : t("Fire test webhook")}
                </button>
              </div>
            </div>
          )}
          <table className="hosts-health-bus__table">
            <thead>
              <tr>
                <th
                  aria-label={t("Select all")}
                  className="hosts-health-bus__th hosts-health-bus__select-cell"
                >
                  <input
                    type="checkbox"
                    aria-label={t("Select all")}
                    checked={
                      filtered.length > 0 &&
                      filtered.every((c) => busSelected.has(c.index))
                    }
                    onChange={toggleBusSelectAll}
                  />
                </th>
                <BusHeader
                  column="status"
                  label=""
                  ariaLabel={t("Status")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <BusHeader
                  column="name"
                  label={t("Name")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <BusHeader
                  column="host"
                  label={t("Endpoint")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <BusHeader
                  column="latency"
                  label={t("Latency")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <BusHeader
                  column="auth"
                  label={t("Auth")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <BusHeader
                  column="group"
                  label={t("Group")}
                  sort={busSort}
                  onClick={cycleBusSort}
                />
                <th aria-label={t("Actions")} />
              </tr>
            </thead>
            <tbody>
              {sortedBusRows(filtered, probes, busSort).map((c) => {
                const r = probes[c.index];
                let dotClass = "hosts-health-dot--unknown";
                let statusText = t("?");
                if (r) {
                  switch (r.status) {
                    case "online":
                      dotClass = "hosts-health-dot--online";
                      statusText = t("ok");
                      break;
                    case "offline":
                    case "timeout":
                      dotClass = "hosts-health-dot--offline";
                      statusText =
                        r.status === "offline" ? t("down") : t("t/o");
                      break;
                    case "error":
                      dotClass = "hosts-health-dot--error";
                      statusText = t("err");
                      break;
                  }
                }
                return (
                  <tr
                    key={c.index}
                    className={
                      "hosts-health-bus__row" +
                      (busSelected.has(c.index)
                        ? " hosts-health-bus__row--selected"
                        : "")
                    }
                    onContextMenu={(e) => openHostMenu(e, c)}
                  >
                    <td className="hosts-health-bus__select-cell">
                      <input
                        type="checkbox"
                        aria-label={t("Select")}
                        checked={busSelected.has(c.index)}
                        onChange={() => toggleBusSelected(c.index)}
                      />
                    </td>
                    <td>
                      <span
                        className={`hosts-health-dot ${dotClass}`}
                        title={statusText}
                      />
                    </td>
                    <td className="hosts-health-bus__name">
                      <EnvTagChip tag={c.envTag} compact />
                      {highlightFilter(
                        c.name || `${c.user}@${c.host}`,
                        filter,
                      )}
                    </td>
                    <td className="mono muted">
                      {highlightFilter(
                        `${c.user}@${c.host}:${c.port}`,
                        filter,
                      )}
                    </td>
                    <td className="mono muted hosts-health-bus__latency">
                      <span>{r?.latencyMs != null ? `${r.latencyMs} ms` : "—"}</span>
                      <Sparkline values={latencyHistory[c.index] ?? []} />
                    </td>
                    <td className="muted">{authLabel(c.authKind, t)}</td>
                    <td className="muted">{c.group ?? ""}</td>
                    <td className="hosts-health-bus__actions">
                      <button
                        type="button"
                        className="mini-button"
                        onClick={() => void probe([c.index])}
                        disabled={busy}
                        title={t("Re-probe just this host")}
                      >
                        ↻
                      </button>
                      <button
                        type="button"
                        className="mini-button mini-button--primary"
                        onClick={() => onConnectSaved(c.index)}
                        title={t("Open a new terminal tab against this host")}
                      >
                        {t("Connect")}
                      </button>
                    </td>
                  </tr>
                );
              })}
              {filtered.length === 0 && filter && (
                <tr>
                  <td colSpan={8} className="hosts-health-bus__empty muted">
                    {t("No saved connections match your filter.")}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      ) : viewMode === "flat" ? (
        <ul className="hosts-health-list">
          {filtered.map((c) => (
            <HostRow
              key={c.index}
              conn={c}
              report={probes[c.index]}
              deep={deepProbes[c.index]}
              onConnect={() => onConnectSaved(c.index)}
              onEdit={() => onEditConnection(c.index)}
              onRecheck={() => void probe([c.index])}
              onDeepProbe={() => void runDeepProbe(c.index)}
              onContextMenu={(e) => openHostMenu(e, c)}
              busy={busy}
            />
          ))}
          {filtered.length === 0 && filter && (
            <li className="hosts-health-list__empty muted">
              {t("No saved connections match your filter.")}
            </li>
          )}
        </ul>
      ) : (
        <ul className="hosts-health-list hosts-health-list--grouped">
          {groupedRows.map((g) => {
            const collapsed = collapsedGroups.has(g.key);
            const labelText = g.key === "" ? t("(ungrouped)") : g.label;
            return (
              <li key={g.key || "__default__"} className="hosts-health-group">
                <button
                  type="button"
                  className="hosts-health-group__header"
                  onClick={() => toggleGroup(g.key)}
                  aria-expanded={!collapsed}
                  title={
                    collapsed
                      ? t("Expand group")
                      : t("Collapse group")
                  }
                >
                  {collapsed ? (
                    <ChevronRight size={12} />
                  ) : (
                    <ChevronDown size={12} />
                  )}
                  <span className="hosts-health-group__label">
                    {labelText}
                  </span>
                  <span className="hosts-health-group__count mono">
                    {g.rows.length}
                  </span>
                  <span className="hosts-health-group__pills mono">
                    {g.online > 0 && (
                      <span className="meta-pill meta-pill--success">
                        {g.online}
                      </span>
                    )}
                    {g.offline > 0 && (
                      <span className="meta-pill meta-pill--danger">
                        {g.offline}
                      </span>
                    )}
                    {g.unknown > 0 && (
                      <span className="meta-pill">{g.unknown}</span>
                    )}
                  </span>
                  <span className="hosts-health-group__actions">
                    <span
                      role="button"
                      tabIndex={0}
                      className="mini-button"
                      onClick={(e) => {
                        // Stop propagation so the group header
                        // toggle doesn't fire when the user
                        // clicks the per-group re-probe button.
                        e.stopPropagation();
                        void probe(g.rows.map((r) => r.index));
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          e.stopPropagation();
                          void probe(g.rows.map((r) => r.index));
                        }
                      }}
                    >
                      {t("Re-probe group")}
                    </span>
                  </span>
                </button>
                {!collapsed && (
                  <ul className="hosts-health-group__rows">
                    {g.rows.map((c) => (
                      <HostRow
                        key={c.index}
                        conn={c}
                        report={probes[c.index]}
                        deep={deepProbes[c.index]}
                        onConnect={() => onConnectSaved(c.index)}
                        onEdit={() => onEditConnection(c.index)}
                        onRecheck={() => void probe([c.index])}
                        onDeepProbe={() => void runDeepProbe(c.index)}
                        onContextMenu={(e) => openHostMenu(e, c)}
                        busy={busy}
                      />
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
          {groupedRows.length === 0 && filter && (
            <li className="hosts-health-list__empty muted">
              {t("No saved connections match your filter.")}
            </li>
          )}
        </ul>
      )}
      {ctxMenu && (
        <ContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          items={buildHostMenu(ctxMenu.conn)}
          onClose={() => setCtxMenu(null)}
        />
      )}
    </section>
  );
}

type BusSortColumn =
  | "name"
  | "host"
  | "latency"
  | "status"
  | "auth"
  | "group";
type BusSort = { column: BusSortColumn; dir: "asc" | "desc" } | null;

/// Click-to-sort table header for the bus view. Renders an arrow
/// indicator next to the active column. The unsorted state is
/// represented by `sort === null` AND the cycle returns there
/// after asc → desc → unsorted (so users always have a path back
/// to the natural order without a separate "clear sort" button).
function BusHeader({
  column,
  label,
  ariaLabel,
  sort,
  onClick,
}: {
  column: BusSortColumn;
  label: string;
  ariaLabel?: string;
  sort: BusSort;
  onClick: (col: BusSortColumn) => void;
}) {
  const active = sort?.column === column;
  const arrow = !active ? "" : sort!.dir === "asc" ? " ▲" : " ▼";
  return (
    <th
      aria-label={ariaLabel ?? label}
      aria-sort={
        !active ? "none" : sort!.dir === "asc" ? "ascending" : "descending"
      }
      className="hosts-health-bus__th"
      onClick={() => onClick(column)}
    >
      {label}
      {arrow}
    </th>
  );
}

/// Sort helper for the bus view. Returns a NEW array (input is
/// untouched) ordered by `sort.column` with `sort.dir`. When
/// `sort` is null, returns the input unchanged so the underlying
/// `filtered` order is preserved.
///
/// Status sort weight matches the welcome view's `probeWeight`
/// helper: online (0) < unknown (1) < error (2) < offline/
/// timeout (3) — so ascending = "good first", descending =
/// "broken first" which is what an oncall engineer wants when
/// triaging.
function sortedBusRows(
  rows: SavedSshConnection[],
  probes: Record<number, cmd.HostHealthReport>,
  sort: BusSort,
): SavedSshConnection[] {
  if (!sort) return rows;
  const dir = sort.dir === "asc" ? 1 : -1;
  const weight = (status: cmd.HostHealthReport["status"] | undefined) => {
    if (!status) return 1; // "probing" / unknown
    switch (status) {
      case "online":
        return 0;
      case "error":
        return 2;
      case "offline":
      case "timeout":
        return 3;
    }
  };
  const keyFor = (c: SavedSshConnection): string | number => {
    const r = probes[c.index];
    switch (sort.column) {
      case "name":
        return (c.name || `${c.user}@${c.host}`).toLowerCase();
      case "host":
        return `${c.host}:${c.port}`.toLowerCase();
      case "latency":
        // Use Number.MAX_SAFE_INTEGER for unknown latency so the
        // ascending sort buries them at the end. Descending puts
        // them at the top, also fine — they're unmeasured.
        return r?.latencyMs ?? Number.MAX_SAFE_INTEGER;
      case "status":
        return weight(r?.status);
      case "auth":
        return c.authKind;
      case "group":
        return (c.group ?? "").toLowerCase();
    }
  };
  return [...rows].sort((a, b) => {
    const ka = keyFor(a);
    const kb = keyFor(b);
    if (ka < kb) return -1 * dir;
    if (ka > kb) return 1 * dir;
    return 0;
  });
}

function HostRow({
  conn,
  report,
  deep,
  onConnect,
  onEdit,
  onRecheck,
  onDeepProbe,
  onContextMenu,
  busy,
}: {
  conn: SavedSshConnection;
  report: cmd.HostHealthReport | undefined;
  /** `undefined` = never run; `"running"` = in-flight; `"no-cached"`
   *  = backend reported no cached SSH session for this target;
   *  otherwise the report. */
  deep: cmd.HostDeepProbeReport | "running" | "no-cached" | undefined;
  onConnect: () => void;
  onEdit: () => void;
  onRecheck: () => void;
  onDeepProbe: () => void;
  /** Right-click handler — opens the panel-level context menu
   *  with Connect / Edit / Re-probe / Copy SSH command. The
   *  panel owns the menu state so we don't end up with one
   *  menu instance per row. */
  onContextMenu: (e: React.MouseEvent) => void;
  busy: boolean;
}) {
  const { t } = useI18n();

  let statusClass = "hosts-health-dot--unknown";
  let statusText = t("Probing…");
  if (report) {
    switch (report.status) {
      case "online":
        statusClass = "hosts-health-dot--online";
        statusText = report.latencyMs != null
          ? t("Online · {ms} ms", { ms: report.latencyMs })
          : t("Online");
        break;
      case "offline":
        statusClass = "hosts-health-dot--offline";
        statusText = t("Offline");
        break;
      case "timeout":
        statusClass = "hosts-health-dot--offline";
        statusText = t("Timeout");
        break;
      case "error":
        statusClass = "hosts-health-dot--error";
        statusText = t("Error");
        break;
    }
  }

  const checkedRel = report ? formatChecked(report.checkedAt, t) : "";

  return (
    <li className="hosts-health-row" onContextMenu={onContextMenu}>
      <div className="hosts-health-row__status">
        <span
          className={`hosts-health-dot ${statusClass}`}
          aria-label={statusText}
          title={statusText}
        />
        <span className="hosts-health-row__status-text mono">
          {statusText}
        </span>
      </div>

      <div className="hosts-health-row__main">
        <div className="hosts-health-row__title">
          <Server size={12} className="muted" />
          <span className="hosts-health-row__name">
            {conn.name || `${conn.user}@${conn.host}`}
          </span>
          <EnvTagChip tag={conn.envTag} />
          {conn.group && (
            <span className="meta-pill meta-pill--ghost">{conn.group}</span>
          )}
        </div>
        <div className="hosts-health-row__meta mono">
          <span>{conn.user}@{conn.host}:{conn.port}</span>
          <span className="muted">·</span>
          <span className="muted">{authLabel(conn.authKind, t)}</span>
          {checkedRel && (
            <>
              <span className="muted">·</span>
              <span className="muted">{checkedRel}</span>
            </>
          )}
        </div>
        {report && report.status !== "online" && report.errorMessage && (
          <div className="hosts-health-row__detail mono muted">
            {report.errorMessage}
          </div>
        )}
        {deep === "no-cached" && (
          <div className="hosts-health-row__detail mono muted">
            {t(
              "Deep probe needs an existing SSH session — open a panel for this host first.",
            )}
          </div>
        )}
        {deep && deep !== "no-cached" && deep !== "running" && (
          <div className="hosts-health-row__deep mono">
            {deep.distro && (
              <span className="hosts-health-row__deep-pill">
                {deep.distro}
              </span>
            )}
            {deep.uptime && (
              <span className="muted">
                {t("up {u}", { u: deep.uptime })}
              </span>
            )}
            {deep.loadAvg && (
              <span className="muted">
                {t("load {l}", { l: deep.loadAvg })}
              </span>
            )}
            {deep.diskRootUse && deep.diskRootAvail && (
              <span className="muted">
                {t("/ {use} ({avail} free)", {
                  use: deep.diskRootUse,
                  avail: deep.diskRootAvail,
                })}
              </span>
            )}
          </div>
        )}
      </div>

      <div className="hosts-health-row__actions">
        <button
          type="button"
          className="mini-button"
          onClick={onRecheck}
          disabled={busy}
          title={t("Re-probe just this host")}
        >
          <Square size={10} /> {t("Re-probe")}
        </button>
        <button
          type="button"
          className="mini-button"
          onClick={onDeepProbe}
          disabled={deep === "running"}
          title={t(
            "Pull uptime / disk / distro over the cached SSH session, when one exists.",
          )}
        >
          {deep === "running" ? t("Probing…") : t("Deep probe")}
        </button>
        <button
          type="button"
          className="mini-button"
          onClick={onEdit}
          title={t("Edit this saved connection")}
        >
          <Pencil size={10} /> {t("Edit")}
        </button>
        <button
          type="button"
          className="mini-button mini-button--primary"
          onClick={onConnect}
          title={t("Open a new terminal tab against this host")}
        >
          <Plug size={10} /> {t("Connect")}
        </button>
      </div>
    </li>
  );
}

/// Wrap every case-insensitive occurrence of `filter` inside
/// `text` with a `<mark>` element so the Bus view shows users
/// where their filter matched. Empty filter returns the text
/// untouched. Splits on the literal substring (no regex) so
/// special characters in the filter don't blow up.
function highlightFilter(text: string, filter: string): React.ReactNode {
  const q = filter.trim();
  if (!q) return text;
  const lower = text.toLowerCase();
  const needle = q.toLowerCase();
  const out: React.ReactNode[] = [];
  let i = 0;
  let key = 0;
  while (i < text.length) {
    const hit = lower.indexOf(needle, i);
    if (hit < 0) {
      out.push(text.slice(i));
      break;
    }
    if (hit > i) out.push(text.slice(i, hit));
    out.push(
      <mark key={key++} className="hosts-health-bus__hl">
        {text.slice(hit, hit + q.length)}
      </mark>,
    );
    i = hit + q.length;
  }
  return out;
}

function authLabel(
  kind: SavedSshConnection["authKind"],
  t: (k: string) => string,
): string {
  switch (kind) {
    case "password":
      return t("Password");
    case "agent":
      return t("Agent");
    case "key":
      return t("Key");
  }
}

function formatChecked(
  epochSecs: number,
  t: (k: string, vars?: Record<string, string | number>) => string,
): string {
  if (!epochSecs) return "";
  const deltaMs = Date.now() - epochSecs * 1000;
  const seconds = Math.max(0, Math.floor(deltaMs / 1000));
  if (seconds < 5) return t("Checked just now");
  if (seconds < 60) return t("Checked {n}s ago", { n: seconds });
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return t("Checked {n}m ago", { n: minutes });
  const hours = Math.floor(minutes / 60);
  return t("Checked {n}h ago", { n: hours });
}

