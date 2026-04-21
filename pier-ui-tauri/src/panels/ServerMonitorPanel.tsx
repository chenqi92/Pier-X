import { ActivitySquare, Database, PackageSearch } from "lucide-react";
import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import type { DetectedServiceView, RightTool, ServerSnapshotView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import DbConnRow from "../components/DbConnRow";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";
import { useTabStore } from "../stores/useTabStore";

type Props = { tab: TabState };

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

export default function ServerMonitorPanel({ tab }: Props) {
  const { t } = useI18n();
  const updateTab = useTabStore((s) => s.updateTab);
  const setTabRightTool = useTabStore((s) => s.setTabRightTool);
  const [snap, setSnap] = useState<ServerSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [services, setServices] = useState<DetectedServiceView[]>([]);
  const [servicesBusy, setServicesBusy] = useState(false);
  const [servicesError, setServicesError] = useState("");
  const [servicesNotice, setServicesNotice] = useState("");

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
    } catch (e) {
      setSnap(null);
      setError(String(e));
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
      setServicesError(String(e));
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
    ? `${tab.sshUser}@${tab.sshHost}:${tab.sshPort}`
    : isLocal
      ? "local"
      : "—";
  const connName = hasSsh
    ? `${tab.sshUser}@${tab.sshHost}`
    : isLocal
      ? t("Local Host")
      : t("Server Monitor");
  const connSub = hasSsh
    ? `port ${tab.sshPort}`
    : isLocal
      ? t("Local probe")
      : t("Not connected");
  const connTag = (
    <>
      <StatusDot tone={snap ? "pos" : "off"} />
      {snap ? t("ready") : t("offline")}
    </>
  );

  return (
    <>
      <PanelHeader
        icon={ActivitySquare}
        title="SERVER MONITOR"
        meta={headerMeta}
      />
      <DbConnRow
        icon={ActivitySquare}
        tint="var(--pos-dim)"
        iconTint="var(--pos)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      <section className="panel-section">
        <div className="form-stack">
          <button className="mini-button" disabled={!canProbe || busy} onClick={() => void probe()} type="button">{busy ? t("Probing...") : t("Probe Server")}</button>
          {!canProbe && <div className="inline-note">{t("No connection available.")}</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {snap && (
        <section className="panel-section">
          <div className="panel-section__title"><span>{t("Resources")}</span></div>
          <ul className="stack-list">
            <li><span>{t("Uptime")}</span><strong>{snap.uptime}</strong></li>
            <li><span>{t("CPU")}</span><strong>{snap.cpuPct.toFixed(1)}%</strong></li>
            <li><span>{t("Load")}</span><strong>{snap.load1.toFixed(2)} / {snap.load5.toFixed(2)} / {snap.load15.toFixed(2)}</strong></li>
            <li><span>{t("Memory")}</span><strong>{snap.memUsedMb.toFixed(0)} / {snap.memTotalMb.toFixed(0)} MB</strong></li>
            <li><span>{t("Swap")}</span><strong>{snap.swapUsedMb.toFixed(0)} / {snap.swapTotalMb.toFixed(0)} MB</strong></li>
            <li><span>{t("Disk")}</span><strong>{snap.diskUsed} / {snap.diskTotal} ({snap.diskUsePct >= 0 ? `${snap.diskUsePct.toFixed(0)}%` : "—"})</strong></li>
          </ul>
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
