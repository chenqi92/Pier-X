import { Cpu, HardDrive, KeyRound, MemoryStick, Network, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type { ServerSnapshotView, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { isMissingKeychainError, localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useUiActionsStore } from "../stores/useUiActionsStore";

type Props = {
  tab: TabState;
  /** Open the saved-connection editor when the keychain has lost the
   *  password for this tab's saved connection. */
  onEditConnection?: (index: number) => void;
};

const MONITOR_ICON = RIGHT_TOOL_META.monitor.icon;

/**
 * Format a bytes-per-second number into a compact human-readable
 * string with units, used by the NETWORK gauge. Returns `null` when
 * the value is below the "no rate yet" sentinel so the gauge can
 * fall back to its placeholder.
 */
function formatRate(bps: number): { value: string; unit: string } | null {
  if (!Number.isFinite(bps) || bps < 0) return null;
  if (bps >= 1024 * 1024) return { value: (bps / (1024 * 1024)).toFixed(1), unit: "MB/s" };
  if (bps >= 1024) return { value: (bps / 1024).toFixed(1), unit: "KB/s" };
  return { value: bps.toFixed(0), unit: "B/s" };
}

type GaugeTone = "accent" | "pos" | "warn" | "off";

function Gauge({
  icon: Icon,
  label,
  value,
  sub,
  pct,
  tone = "accent",
}: {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  sub: string;
  pct: number;
  tone?: GaugeTone;
}) {
  // "off" is the placeholder tone used before the first probe lands —
  // the bar renders empty and the fill color falls back to the muted
  // palette so the chrome stays visually neutral.
  const color =
    tone === "pos" ? "var(--pos)"
      : tone === "warn" ? "var(--warn)"
      : tone === "off" ? "var(--dim)"
      : "var(--accent)";
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <div className="mon-gauge">
      <div className="mon-gauge-label">
        {Icon}
        <span>{label}</span>
      </div>
      <div className="mon-gauge-value">{value}</div>
      <div className="mon-gauge-bar">
        <div className="mon-gauge-fill" style={{ width: `${clamped}%`, background: color }} />
      </div>
      <div className="mon-gauge-sub mono">{sub}</div>
    </div>
  );
}

function toneFromPct(pct: number): GaugeTone {
  if (pct >= 85) return "warn";
  if (pct >= 50) return "accent";
  return "pos";
}

function formatTimestamp(ts: number): string {
  if (!ts) return "—";
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export default function ServerMonitorPanel({ tab, onEditConnection }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [snap, setSnap] = useState<ServerSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  // Mirrors `busy` for the polling interval — reading it via ref
  // means we don't have to put `busy` in the effect's deps and pay
  // the interval-teardown-on-every-probe cost.
  const busyRef = useRef(false);
  busyRef.current = busy;
  const [error, setError] = useState("");
  // Track the missing-keychain condition separately so the recovery
  // button stays available even after a localized error string has
  // been transformed beyond regex recognition.
  const [needsPasswordRecovery, setNeedsPasswordRecovery] = useState(false);
  const [lastProbed, setLastProbed] = useState(0);

  // SSH context is "available" any time the tab has the addressing
  // bits filled in — either via the primary fields (real SSH tab),
  // mirrored fields (local terminal that ran `ssh user@host`), or
  // the nested-ssh overlay (`ssh user@host` inside an existing SSH
  // session). `effectiveSshTarget` collapses all three into one
  // shape so the probe / detect commands always reach the host the
  // user thinks they are looking at.
  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  // Only treat the tab as "local probe" when there is no SSH target
  // overlay; otherwise the SSH path takes priority.
  const isLocal = tab.backend === "local" && !hasSsh;

  async function probe() {
    setBusy(true);
    setError("");
    setNeedsPasswordRecovery(false);
    try {
      const s = isLocal
        ? await cmd.localSystemInfo()
        : sshTarget
          ? await cmd.serverMonitorProbe({
              host: sshTarget.host,
              port: sshTarget.port,
              user: sshTarget.user,
              authMode: sshTarget.authMode,
              password: sshTarget.password,
              keyPath: sshTarget.keyPath,
              savedConnectionIndex: sshTarget.savedConnectionIndex,
            })
          : null;
      if (!s) {
        setError(t("No connection available."));
        return;
      }
      setSnap(s);
      setLastProbed(Date.now());
    } catch (e) {
      // Keep the last good snapshot visible instead of blanking the whole
      // panel — a transient SSH hiccup shouldn't unmount the gauges.
      setError(formatError(e));
      if (isMissingKeychainError(e)) setNeedsPasswordRecovery(true);
    } finally {
      setBusy(false);
    }
  }

  // The recovery button dispatches via the global UI-action bus —
  // App.tsx subscribes to it and opens the saved-connection editor.
  // Going through the bus instead of a prop callback keeps the
  // affordance working no matter which wrapper renders this panel,
  // since props can be silently dropped if a parent forgets to
  // forward them.
  const requestEditConnection = useUiActionsStore((s) => s.requestEditConnection);
  const recoverableSavedIndex = sshTarget?.savedConnectionIndex ?? null;
  const canRecoverPassword =
    needsPasswordRecovery && recoverableSavedIndex !== null;
  const recoverPassword = () => {
    if (!canRecoverPassword || recoverableSavedIndex === null) return;
    requestEditConnection(recoverableSavedIndex);
    onEditConnection?.(recoverableSavedIndex);
  };

  const canProbe = isLocal || hasSsh;

  // Auto-probe + detect when this panel mounts for an SSH or local tab —
  // the component is keyed by tab.id in RightSidebar so this fires on
  // tab switch too. Password-auth saved tabs that haven't primed their
  // password yet will no-op here; user can tap "探测服务器" to retry.
  // Also installs a 5-second polling interval so the gauges actually
  // move; without it the panel reads as "frozen" because the backend
  // probe is one-shot (`server_monitor::probe` runs `uptime + free +
  // df + /proc/stat` in a single SSH exec call). The poll skips when
  // a previous probe is still in flight (`busy` guard) so a slow
  // remote can't pile up overlapping requests.
  useEffect(() => {
    const haveCreds =
      sshTarget !== null &&
      (sshTarget.authMode !== "password" ||
        sshTarget.password.length > 0 ||
        sshTarget.savedConnectionIndex !== null);
    // For real SSH-backend tabs, hold off the first probe until the
    // terminal session is up. The backend's `terminal_create_ssh_*`
    // call seeds the shared SSH cache as soon as the russh handshake
    // completes; once we wait for it, the probe (and the 5-second
    // polling that follows) reuses that cached session instead of
    // racing the terminal handshake with a parallel one. On the
    // user's LAN this drops "double-click → usable terminal" from
    // several seconds (sshd serializing 3+ concurrent password
    // logins) to roughly one round-trip.
    //
    // Local tabs that mirrored an `ssh user@host` invocation have a
    // local-PTY `terminalSessionId` but the russh session is on the
    // panel side, so we don't need to wait — the first probe primes
    // the cache and subsequent ones reuse.
    const waitingForTerminal =
      tab.backend === "ssh" && tab.terminalSessionId === null;
    const ready = (isLocal || haveCreds) && !waitingForTerminal;
    if (!ready) return;
    void probe();
    const interval = window.setInterval(() => {
      // Re-read busy from the latest closure via a state check —
      // intentionally letting the JS engine grab the freshest value
      // since `busy` isn't in the deps (we don't want the interval
      // teardown/recreate cycle every time it flips).
      if (busyRef.current) return;
      void probe();
    }, 5000);
    return () => window.clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    tab.backend,
    tab.terminalSessionId !== null,
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    // Re-run once the async password resolution lands:
    (sshTarget?.password.length ?? 0) > 0,
  ]);

  const headerMeta = sshTarget
    ? `${sshTarget.host} · :${sshTarget.port}`
    : isLocal
      ? t("local")
      : "—";
  const connName = sshTarget
    ? `${sshTarget.user}@${sshTarget.host}`
    : isLocal
      ? t("Local Host")
      : t("Server Monitor");
  const connSub = sshTarget
    ? t("Port {port}", { port: sshTarget.port })
    : isLocal
      ? t("Local probe")
      : t("Not connected");
  const connTag = (
    <>
      <StatusDot tone={snap ? "pos" : "off"} />
      {snap ? t("ready") : t("offline")}
    </>
  );

  const memPct = snap && snap.memTotalMb > 0 ? (snap.memUsedMb / snap.memTotalMb) * 100 : 0;
  const cpuPct = snap?.cpuPct ?? 0;
  const diskPct = snap && snap.diskUsePct >= 0 ? snap.diskUsePct : 0;
  const netRate = snap ? formatRate(snap.netRxBps + snap.netTxBps) : null;
  // Cap the network gauge at 100MB/s for the bar fill — pure cosmetic
  // ceiling, the readout itself shows the actual rate.
  const netPct = snap && snap.netRxBps >= 0 && snap.netTxBps >= 0
    ? Math.min(100, ((snap.netRxBps + snap.netTxBps) / (100 * 1024 * 1024)) * 100)
    : 0;
  const rxRate = snap ? formatRate(snap.netRxBps) : null;
  const txRate = snap ? formatRate(snap.netTxBps) : null;

  return (
    <>
      <PanelHeader
        icon={MONITOR_ICON}
        title={t("Server Monitor")}
        meta={headerMeta}
      />
      <DbConnRow
        icon={MONITOR_ICON}
        tint="var(--pos-dim)"
        iconTint="var(--pos)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      {/*
        Always-visible monitor section: chrome (host bar + gauges + probe
        button row) renders immediately so clicking the Monitor tool
        never flashes a blank panel. When snapshot is null we render
        placeholder "—" values; the four Gauge shells stay in place and
        fill in when probe() lands.
      */}
      <section className="mon">
        <div className="mon-host">
          <div className="mon-host-top">
            <StatusDot tone={snap ? "pos" : "off"} />
            <div className="mon-host-name">{connName}</div>
            <span className="mono mon-host-uptime">
              {snap ? `${t("uptime")} ${snap.uptime}` : t("not yet probed")}
            </span>
          </div>
          <div className="mon-host-meta mono">
            {snap?.osLabel || headerMeta}
            {snap && snap.load1 >= 0 ? (
              <> · {t("load")} {snap.load1.toFixed(2)} / {snap.load5.toFixed(2)} / {snap.load15.toFixed(2)}</>
            ) : null}
          </div>
        </div>

        <div className="mon-grid">
          <Gauge
            icon={<Cpu size={10} />}
            label={t("CPU")}
            value={snap ? <>{cpuPct.toFixed(1)}<span className="mon-gauge-unit">%</span></> : <>—</>}
            sub={snap && snap.load1 >= 0
              ? `${t("load")} ${snap.load1.toFixed(2)} · ${snap.load5.toFixed(2)} · ${snap.load15.toFixed(2)}`
              : "—"}
            pct={snap ? cpuPct : 0}
            tone={snap ? toneFromPct(cpuPct) : "off"}
          />
          <Gauge
            icon={<MemoryStick size={10} />}
            label={t("MEMORY")}
            value={snap ? <>{memPct.toFixed(0)}<span className="mon-gauge-unit">%</span></> : <>—</>}
            sub={snap
              ? `${(snap.memUsedMb / 1024).toFixed(1)} / ${(snap.memTotalMb / 1024).toFixed(1)} GB`
              : "—"}
            pct={snap ? memPct : 0}
            tone={snap ? toneFromPct(memPct) : "off"}
          />
          <Gauge
            icon={<HardDrive size={10} />}
            label={t("DISK")}
            value={snap
              ? <>{snap.diskUsePct >= 0 ? snap.diskUsePct.toFixed(0) : "—"}<span className="mon-gauge-unit">%</span></>
              : <>—</>}
            sub={snap ? `${snap.diskAvail} ${t("free of")} ${snap.diskTotal}` : "—"}
            pct={snap ? diskPct : 0}
            tone={snap ? toneFromPct(diskPct) : "off"}
          />
          <Gauge
            icon={<Network size={10} />}
            label={t("NETWORK")}
            value={netRate ? <>{netRate.value}<span className="mon-gauge-unit"> {netRate.unit}</span></> : <>—</>}
            sub={rxRate && txRate
              ? `↓ ${rxRate.value} ${rxRate.unit} · ↑ ${txRate.value} ${txRate.unit}`
              : t("warming up...")}
            pct={netPct}
            tone={netRate ? "pos" : "off"}
          />
        </div>

        {/*
          System-stats strip — pier-x-copy reference shows vCPU /
          total RAM / total disk / process count as compact pills
          underneath the gauges. Each pill stays as "—" until the
          backend probe fills the corresponding field, so the chrome
          doesn't shift after the first probe lands.
        */}
        <div className="mon-strip">
          <span className="mon-pill">
            <Cpu size={10} />
            {snap && snap.cpuCount > 0 ? `${snap.cpuCount} vCPU` : "—"}
          </span>
          <span className="mon-pill">
            <MemoryStick size={10} />
            {snap && snap.memTotalMb > 0
              ? `${(snap.memTotalMb / 1024).toFixed(1)} GB`
              : "—"}
          </span>
          <span className="mon-pill">
            <HardDrive size={10} />
            {snap?.diskTotal || "—"}
          </span>
          <span className="mon-pill">
            <Network size={10} />
            {snap && snap.procCount > 0
              ? t("{count} procs", { count: snap.procCount })
              : "—"}
          </span>
        </div>

        {/*
          Top processes table — populated from `ps -eo
          pid,comm,pcpu,pmem,etime --sort=-pcpu | head -8`. Empty
          tbody renders an "—" placeholder so the block is always
          present (matches pier-x-copy's stable layout).
        */}
        <div className="mon-block">
          <div className="mon-block-head">
            <span>{t("TOP PROCESSES")}</span>
            <span className="mono mon-block-meta">{t("by CPU")}</span>
          </div>
          <table className="mon-table">
            <thead>
              <tr>
                <th style={{ width: 60 }}>{t("PID")}</th>
                <th>{t("COMMAND")}</th>
                <th style={{ width: 56, textAlign: "right" }}>{t("CPU%")}</th>
                <th style={{ width: 56, textAlign: "right" }}>{t("MEM%")}</th>
                <th style={{ width: 80, textAlign: "right" }}>{t("TIME")}</th>
              </tr>
            </thead>
            <tbody>
              {snap && snap.topProcesses.length > 0 ? (
                snap.topProcesses.map((row, i) => (
                  <tr key={`${row.pid}-${i}`}>
                    <td className="mono mon-cell-muted">{row.pid}</td>
                    <td className="mono">{row.command}</td>
                    <td className="mono mon-cell-right">{row.cpuPct}</td>
                    <td className="mono mon-cell-right">{row.memPct}</td>
                    <td className="mono mon-cell-muted mon-cell-right">{row.elapsed}</td>
                  </tr>
                ))
              ) : (
                <tr>
                  <td colSpan={5} className="mon-empty mono">
                    {snap ? t("(no process data)") : "—"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        <div className="mon-actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={!canProbe || busy}
            onClick={() => void probe()}
          >
            <RefreshCw size={11} /> {busy ? t("Probing...") : snap ? t("Probe now") : t("Probe Server")}
          </button>
          <span className="mono mon-actions-hint">
            {!canProbe
              ? t("No connection available.")
              : lastProbed
                ? `${t("last")}: ${formatTimestamp(lastProbed)}`
                : t("not yet probed")}
          </span>
        </div>
        {error && (
          <div className="status-note status-note--error">
            <span>{error}</span>
            {canRecoverPassword && (
              <button
                type="button"
                className="mini-button"
                onClick={recoverPassword}
              >
                <KeyRound size={11} /> {t("Re-enter password")}
              </button>
            )}
          </div>
        )}
      </section>
    </div>
    </>
  );
}
