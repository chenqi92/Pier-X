import { Activity, Cpu, Database, HardDrive, MemoryStick, PackageSearch, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type { DetectedServiceView, RightTool, ServerSnapshotView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

const MONITOR_ICON = RIGHT_TOOL_META.monitor.icon;

function serviceTone(status: string) {
  switch (String(status || "").toLowerCase()) {
    case "running":
      return "success";
    case "stopped":
      return "warning";
    case "installed":
      return "info";
    default:
      return "neutral";
  }
}

function serviceLabel(service: DetectedServiceView) {
  switch (service.name) {
    case "postgresql":
      return "PostgreSQL";
    case "mysql":
      return "MySQL";
    case "redis":
      return "Redis";
    case "docker":
      return "Docker";
    default:
      return service.name;
  }
}

function serviceTool(service: DetectedServiceView): RightTool | null {
  switch (service.name) {
    case "mysql":
      return "mysql";
    case "postgresql":
      return "postgres";
    case "redis":
      return "redis";
    case "docker":
      return "docker";
    default:
      return null;
  }
}

type GaugeTone = "accent" | "pos" | "warn";

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
  const color =
    tone === "pos" ? "var(--pos)" : tone === "warn" ? "var(--warn)" : "var(--accent)";
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

export default function ServerMonitorPanel({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const updateTab = useTabStore((s) => s.updateTab);
  const setTabRightTool = useTabStore((s) => s.setTabRightTool);
  const [snap, setSnap] = useState<ServerSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [services, setServices] = useState<DetectedServiceView[]>([]);
  const [servicesBusy, setServicesBusy] = useState(false);
  const [servicesError, setServicesError] = useState("");
  const [servicesNotice, setServicesNotice] = useState("");
  const [lastProbed, setLastProbed] = useState(0);

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();
  const isLocal = tab.backend === "local";

  async function probe() {
    setBusy(true);
    setError("");
    try {
      const s = isLocal
        ? await cmd.localSystemInfo()
        : hasSsh
          ? await cmd.serverMonitorProbe({
              host: tab.sshHost,
              port: tab.sshPort,
              user: tab.sshUser,
              authMode: tab.sshAuthMode,
              password: tab.sshPassword,
              keyPath: tab.sshKeyPath,
            })
          : null;
      if (!s) {
        setError(t("No connection available."));
        return;
      }
      setSnap(s);
      setLastProbed(Date.now());
    } catch (e) {
      setSnap(null);
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function detect() {
    if (!hasSsh) {
      setServicesError(t("SSH connection required."));
      return;
    }
    setServicesBusy(true);
    setServicesError("");
    setServicesNotice("");
    try {
      const next = await cmd.detectServices({
        host: tab.sshHost,
        port: tab.sshPort,
        user: tab.sshUser,
        authMode: tab.sshAuthMode,
        password: tab.sshPassword,
        keyPath: tab.sshKeyPath,
      });
      setServices(next);
      if (next.length === 0) {
        setServicesNotice(t("No supported services detected."));
      }
    } catch (e) {
      setServices([]);
      setServicesError(formatError(e));
    } finally {
      setServicesBusy(false);
    }
  }

  function openService(service: DetectedServiceView) {
    const tool = serviceTool(service);
    if (!tool) {
      return;
    }

    switch (service.name) {
      case "mysql":
        if (tab.mysqlTunnelId) {
          void cmd.sshTunnelClose(tab.mysqlTunnelId).catch(() => {});
        }
        updateTab(tab.id, {
          mysqlHost: "127.0.0.1",
          mysqlPort: service.port || tab.mysqlPort,
          mysqlTunnelId: null,
          mysqlTunnelPort: null,
        });
        break;
      case "postgresql":
        if (tab.pgTunnelId) {
          void cmd.sshTunnelClose(tab.pgTunnelId).catch(() => {});
        }
        updateTab(tab.id, {
          pgHost: "127.0.0.1",
          pgPort: service.port || tab.pgPort,
          pgTunnelId: null,
          pgTunnelPort: null,
        });
        break;
      case "redis":
        if (tab.redisTunnelId) {
          void cmd.sshTunnelClose(tab.redisTunnelId).catch(() => {});
        }
        updateTab(tab.id, {
          redisHost: "127.0.0.1",
          redisPort: service.port || tab.redisPort,
          redisTunnelId: null,
          redisTunnelPort: null,
        });
        break;
      default:
        break;
    }

    setTabRightTool(tab.id, tool);
    setServicesNotice(
      tool === "docker"
        ? t("Opened Docker tools for this SSH tab.")
        : t("Applied remote host and detected port to {tool}.", {
            tool: t(serviceLabel(service)),
          }),
    );
  }

  const canProbe = isLocal || hasSsh;

  // Auto-probe + detect when this panel mounts for an SSH or local tab —
  // the component is keyed by tab.id in RightSidebar so this fires on
  // tab switch too. Password-auth saved tabs that haven't primed their
  // password yet will no-op here; user can tap "探测服务器" to retry.
  useEffect(() => {
    const ready =
      isLocal ||
      (hasSsh &&
        (tab.sshAuthMode !== "password" ||
          tab.sshPassword.length > 0 ||
          tab.sshSavedConnectionIndex !== null));
    if (!ready) return;
    void probe();
    if (hasSsh) {
      void detect();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    tab.backend,
    tab.sshHost,
    tab.sshPort,
    tab.sshUser,
    tab.sshAuthMode,
    // Re-run once the async password resolution lands:
    tab.sshPassword.length > 0,
  ]);

  const headerMeta = hasSsh
    ? `${tab.sshHost} · :${tab.sshPort}`
    : isLocal
      ? t("local")
      : "—";
  const connName = hasSsh
    ? `${tab.sshUser}@${tab.sshHost}`
    : isLocal
      ? t("Local Host")
      : t("Server Monitor");
  const connSub = hasSsh
    ? t("Port {port}", { port: tab.sshPort })
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
  const swapPct = snap && snap.swapTotalMb > 0 ? (snap.swapUsedMb / snap.swapTotalMb) * 100 : 0;
  const cpuPct = snap?.cpuPct ?? 0;
  const diskPct = snap && snap.diskUsePct >= 0 ? snap.diskUsePct : 0;

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
      {snap ? (
        <section className="mon">
          <div className="mon-host">
            <div className="mon-host-top">
              <StatusDot tone="pos" />
              <div className="mon-host-name">{connName}</div>
              <span className="mono mon-host-uptime">{t("uptime")} {snap.uptime}</span>
            </div>
            <div className="mon-host-meta mono">
              {headerMeta}
              {snap.load1 >= 0 ? (
                <> · {t("load")} {snap.load1.toFixed(2)} / {snap.load5.toFixed(2)} / {snap.load15.toFixed(2)}</>
              ) : null}
            </div>
          </div>

          <div className="mon-grid">
            <Gauge
              icon={<Cpu size={10} />}
              label={t("CPU")}
              value={<>{cpuPct.toFixed(1)}<span className="mon-gauge-unit">%</span></>}
              sub={snap.load1 >= 0 ? `${t("load")} ${snap.load1.toFixed(2)} · ${snap.load5.toFixed(2)} · ${snap.load15.toFixed(2)}` : "—"}
              pct={cpuPct}
              tone={toneFromPct(cpuPct)}
            />
            <Gauge
              icon={<MemoryStick size={10} />}
              label={t("MEMORY")}
              value={<>{memPct.toFixed(0)}<span className="mon-gauge-unit">%</span></>}
              sub={`${(snap.memUsedMb / 1024).toFixed(1)} / ${(snap.memTotalMb / 1024).toFixed(1)} GB`}
              pct={memPct}
              tone={toneFromPct(memPct)}
            />
            <Gauge
              icon={<Activity size={10} />}
              label={t("SWAP")}
              value={<>{snap.swapTotalMb > 0 ? swapPct.toFixed(0) : "0"}<span className="mon-gauge-unit">%</span></>}
              sub={snap.swapTotalMb > 0
                ? `${snap.swapUsedMb.toFixed(0)} / ${snap.swapTotalMb.toFixed(0)} MB`
                : t("no swap")}
              pct={swapPct}
              tone={toneFromPct(swapPct)}
            />
            <Gauge
              icon={<HardDrive size={10} />}
              label={t("DISK")}
              value={<>{snap.diskUsePct >= 0 ? snap.diskUsePct.toFixed(0) : "—"}<span className="mon-gauge-unit">%</span></>}
              sub={`${snap.diskAvail} ${t("free of")} ${snap.diskTotal}`}
              pct={diskPct}
              tone={toneFromPct(diskPct)}
            />
          </div>

          <div className="mon-actions">
            <button
              type="button"
              className="btn is-ghost is-compact"
              disabled={!canProbe || busy}
              onClick={() => void probe()}
            >
              <RefreshCw size={11} /> {busy ? t("Probing...") : t("Probe now")}
            </button>
            <span className="mono mon-actions-hint">
              {lastProbed ? `${t("last")}: ${formatTimestamp(lastProbed)}` : t("not yet probed")}
            </span>
          </div>
          {error && <div className="status-note status-note--error">{error}</div>}
        </section>
      ) : (
        <section className="panel-section">
          <div className="form-stack">
            <button
              className="btn is-compact"
              disabled={!canProbe || busy}
              onClick={() => void probe()}
              type="button"
            >
              <RefreshCw size={11} /> {busy ? t("Probing...") : t("Probe Server")}
            </button>
            {!canProbe && <div className="inline-note">{t("No connection available.")}</div>}
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </section>
      )}

      {hasSsh && (
        <section className="panel-section">
          <div className="panel-section__title"><PackageSearch size={14} /><span>{t("Detected Services")}</span></div>
          <div className="form-stack">
            <div className="button-row">
              <button className="mini-button" disabled={servicesBusy} onClick={() => void detect()} type="button">
                {servicesBusy ? t("Detecting...") : t("Detect Services")}
              </button>
            </div>
            <div className="inline-note">{t("Service discovery runs over the active SSH target and can prefill the matching tool.")}</div>
            {servicesNotice && <div className="status-note">{servicesNotice}</div>}
            {servicesError && <div className="status-note status-note--error">{servicesError}</div>}
          </div>

          {services.length > 0 && (
            <div className="service-grid">
              {services.map((service) => {
                const tool = serviceTool(service);
                const tone = serviceTone(service.status);
                return (
                  <div className="connection-row" key={`${service.name}-${service.port}`}>
                    <div className="connection-row__head">
                      <strong>{t(serviceLabel(service))}</strong>
                      <div className="button-row">
                        <span className={`connection-pill connection-pill--${tone}`}>{t(service.status)}</span>
                        {service.port > 0 ? <span className="connection-pill">{service.port}</span> : null}
                      </div>
                    </div>
                    <div className="connection-row__meta">
                      {service.version || t("Version unavailable")}
                    </div>
                    {tool && (
                      <div className="connection-row__actions">
                        <button className="mini-button" onClick={() => openService(service)} type="button">
                          {t("Open {tool}", { tool: t(serviceLabel(service)) })}
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </section>
      )}

      {hasSsh && services.length === 0 && !servicesBusy && !servicesError && (
        <section className="panel-section">
          <div className="empty-note">{t("No service scan has been run yet.")}</div>
        </section>
      )}

      {hasSsh && (
        <section className="panel-section">
          <div className="panel-section__title"><Database size={14} /><span>{t("Remote Endpoint")}</span></div>
          <div className="inline-note">{tab.sshUser}@{tab.sshHost}:{tab.sshPort}</div>
        </section>
      )}
    </div>
    </>
  );
}
