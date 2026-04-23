import { useCallback, useEffect, useMemo, useState } from "react";
import { Plug, Plus, RefreshCw, Trash2, X } from "lucide-react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { toast } from "../stores/useToastStore";
import * as cmd from "../lib/commands";
import type { TunnelInfoView } from "../lib/types";

type Props = {
  open: boolean;
  onClose: () => void;
};

/**
 * Manage local SSH port forwards (`ssh -L` equivalents).
 *
 * Shows every live tunnel registered with the Tauri state —
 * including ones opened implicitly by DB / Log panels — and
 * lets the user open a new forward against any saved SSH
 * connection. Remote forwards (`ssh -R`) are explicitly NOT
 * supported here, per PRODUCT-SPEC §6.3: russh's
 * `tcpip_forward` is a separate implementation we haven't
 * built yet.
 */
export default function PortForwardDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const connections = useConnectionStore((s) => s.connections);
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  const [tunnels, setTunnels] = useState<TunnelInfoView[]>([]);
  const [loading, setLoading] = useState(false);
  const [connIndex, setConnIndex] = useState<number | null>(null);
  const [remoteHost, setRemoteHost] = useState("127.0.0.1");
  const [remotePort, setRemotePort] = useState<number>(5432);
  const [localPort, setLocalPort] = useState<number>(0);
  const [busy, setBusy] = useState(false);

  const selectedConn = useMemo(
    () => connections.find((c) => c.index === connIndex) ?? null,
    [connections, connIndex],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await cmd.sshTunnelList();
      list.sort((a, b) => a.localPort - b.localPort);
      setTunnels(list);
    } catch (e) {
      toast.error(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) void refresh();
  }, [open, refresh]);

  useEffect(() => {
    if (open && connIndex === null && connections.length > 0) {
      setConnIndex(connections[0].index);
    }
  }, [open, connIndex, connections]);

  const handleOpenTunnel = useCallback(async () => {
    if (!selectedConn) return;
    if (!remoteHost.trim() || remotePort <= 0) return;
    setBusy(true);
    try {
      const info = await cmd.sshTunnelOpen({
        host: selectedConn.host,
        port: selectedConn.port,
        user: selectedConn.user,
        authMode: selectedConn.authKind as "password" | "agent" | "key",
        password: "",
        keyPath: selectedConn.keyPath ?? "",
        remoteHost: remoteHost.trim(),
        remotePort,
        localPort: localPort > 0 ? localPort : null,
        savedConnectionIndex: selectedConn.index,
      });
      toast.success(
        t("Tunnel ready on {host}:{port}.", {
          host: info.localHost || "127.0.0.1",
          port: info.localPort,
        }),
      );
      await refresh();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  }, [selectedConn, remoteHost, remotePort, localPort, refresh, t]);

  const handleClose = useCallback(
    async (id: string) => {
      try {
        await cmd.sshTunnelClose(id);
        toast.info(t("Tunnel closed."));
        await refresh();
      } catch (e) {
        toast.error(String(e));
      }
    },
    [refresh, t],
  );

  if (!open) return null;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg" style={{ ...dialogStyle, minWidth: 560, maxWidth: 720 }} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Plug size={13} />
            {t("Port forwarding")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={() => void refresh()} title={t("Refresh")}>
            <RefreshCw size={12} className={loading ? "ftp-spin" : ""} />
          </IconButton>
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dlg-body" style={{ display: "block", padding: "var(--sp-3)" }}>
          <div
            style={{
              fontSize: "var(--ui-fs-sm)",
              color: "var(--muted)",
              marginBottom: "var(--sp-3)",
            }}
          >
            {t("Local forwards only (ssh -L). Opens a local listener that proxies into the SSH session.")}
          </div>

          <div className="settings__section-title" style={{ marginBottom: 4 }}>
            {t("Active tunnels")}
            <span className="settings__badge">{tunnels.length}</span>
          </div>
          {tunnels.length === 0 ? (
            <div className="empty-note">
              {loading ? t("Loading...") : t("No active tunnels.")}
            </div>
          ) : (
            <div className="settings__conn-list">
              {tunnels.map((tunnel) => (
                <div key={tunnel.tunnelId} className="settings__conn-card">
                  <div className="settings__conn-header">
                    <strong style={{ fontFamily: "var(--mono)" }}>
                      {tunnel.localHost || "127.0.0.1"}:{tunnel.localPort}
                    </strong>
                    <span className="settings__conn-auth">
                      {tunnel.alive ? t("alive") : t("dead")}
                    </span>
                  </div>
                  <div className="settings__conn-meta" style={{ fontFamily: "var(--mono)" }}>
                    → {tunnel.remoteHost}:{tunnel.remotePort}
                  </div>
                  <div className="settings__conn-actions">
                    <button
                      className="mini-button mini-button--destructive"
                      onClick={() => void handleClose(tunnel.tunnelId)}
                      type="button"
                    >
                      <Trash2 size={11} />
                      {t("Close Tunnel")}
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}

          <div className="settings__section-title" style={{ marginTop: "var(--sp-4)", marginBottom: 4 }}>
            {t("Open new tunnel")}
          </div>
          {connections.length === 0 ? (
            <div className="empty-note">
              {t("No saved connections yet. Add one from the Servers sidebar.")}
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
              <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <span className="settings__row-name">{t("Via SSH connection")}</span>
                <select
                  className="settings__select"
                  value={connIndex ?? ""}
                  onChange={(e) => setConnIndex(Number.parseInt(e.currentTarget.value, 10))}
                >
                  {connections.map((c) => (
                    <option key={c.index} value={c.index}>
                      {c.name} ({c.user}@{c.host}:{c.port})
                    </option>
                  ))}
                </select>
              </label>
              <div style={{ display: "grid", gridTemplateColumns: "2fr 1fr 1fr", gap: "var(--sp-2)" }}>
                <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <span className="settings__row-name">{t("Remote host")}</span>
                  <input
                    className="settings__select"
                    value={remoteHost}
                    onChange={(e) => setRemoteHost(e.currentTarget.value)}
                    placeholder="127.0.0.1"
                    style={{ fontFamily: "var(--mono)" }}
                  />
                </label>
                <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <span className="settings__row-name">{t("Remote port")}</span>
                  <input
                    className="settings__select"
                    type="number"
                    value={remotePort}
                    onChange={(e) => setRemotePort(Number.parseInt(e.currentTarget.value, 10) || 0)}
                    min={1}
                    max={65535}
                    style={{ fontFamily: "var(--mono)" }}
                  />
                </label>
                <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  <span className="settings__row-name">{t("Local port")}</span>
                  <input
                    className="settings__select"
                    type="number"
                    value={localPort}
                    onChange={(e) => setLocalPort(Number.parseInt(e.currentTarget.value, 10) || 0)}
                    min={0}
                    max={65535}
                    placeholder="0"
                    style={{ fontFamily: "var(--mono)" }}
                  />
                </label>
              </div>
              <div style={{ fontSize: "var(--ui-fs-sm)", color: "var(--muted)" }}>
                {t("Local port 0 lets the OS pick a free port.")}
              </div>
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button
                  className="mini-button"
                  onClick={() => void handleOpenTunnel()}
                  disabled={busy || !selectedConn || !remoteHost.trim() || remotePort <= 0}
                  type="button"
                >
                  <Plus size={11} />
                  {busy ? t("Opening...") : t("Open Tunnel")}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
