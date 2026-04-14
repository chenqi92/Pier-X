import { ActivitySquare } from "lucide-react";
import { useState } from "react";
import * as cmd from "../lib/commands";
import type { ServerSnapshotView, TabState } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { tab: TabState };

export default function ServerMonitorPanel({ tab }: Props) {
  const { t } = useI18n();
  const [snap, setSnap] = useState<ServerSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const hasSsh = tab.backend === "ssh" && tab.sshHost.trim() && tab.sshUser.trim();

  async function probe() {
    if (!hasSsh) { setError("SSH connection required."); return; }
    setBusy(true); setError("");
    try {
      const s = await cmd.serverMonitorProbe({ host: tab.sshHost, port: tab.sshPort, user: tab.sshUser, authMode: tab.sshAuthMode, password: tab.sshPassword, keyPath: tab.sshKeyPath });
      setSnap(s);
    } catch (e) { setSnap(null); setError(String(e)); }
    finally { setBusy(false); }
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><ActivitySquare size={14} /><span>{t("Server Monitor")}</span></div>
        <div className="form-stack">
          <button className="mini-button" disabled={!hasSsh || busy} onClick={() => void probe()} type="button">{busy ? "Probing..." : t("Probe Server")}</button>
          {!hasSsh && <div className="inline-note">SSH connection required.</div>}
          {error && <div className="status-note status-note--error">{error}</div>}
        </div>
      </section>

      {snap && (
        <section className="panel-section">
          <div className="panel-section__title"><span>Resources</span></div>
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
    </div>
  );
}
