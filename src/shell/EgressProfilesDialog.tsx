import { Plus, Shield, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import type { EgressProfile } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useEgressStore } from "../stores/useEgressStore";

type Props = {
  open: boolean;
  onClose: () => void;
};

type EditableKind = "none" | "socks5" | "http" | "ssh_jump" | "wireguard" | "external_vpn";

type DraftAuth = { user: string; password: string; dirty: boolean };

type Draft = {
  id: string;
  name: string;
  kind: EditableKind;
  host: string;
  port: string;
  useAuth: boolean;
  auth: DraftAuth;
  /** ssh_jump only: name of a saved SSH connection to jump through. */
  viaConnection: string;
  /** wireguard only: AllowedIPs / endpoint / address (informational —
   *  the actual config lives in `~/.config/pier-x/egress/<id>.conf`). */
  wgEndpoint: string;
  wgAddress: string;
  wgAllowedIps: string;
  wgPeerKey: string;
  /** external_vpn: engine choice + config path. */
  vpnEngine: "open_vpn" | "open_connect";
  vpnConfig: string;
  dns: "auto" | "tunnel" | "passthrough" | "custom";
  /** Only meaningful when dns === "custom". */
  dnsCustomServer: string;
  isNew: boolean;
};

function newId() {
  return `egress-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

function emptyDraft(): Draft {
  return {
    id: newId(),
    name: "",
    kind: "socks5",
    host: "",
    port: "1080",
    useAuth: false,
    auth: { user: "", password: "", dirty: false },
    viaConnection: "",
    wgEndpoint: "",
    wgAddress: "",
    wgAllowedIps: "",
    wgPeerKey: "",
    vpnEngine: "open_vpn",
    vpnConfig: "",
    dns: "auto",
    dnsCustomServer: "",
    isNew: true,
  };
}

function toDraft(profile: EgressProfile): Draft {
  const dnsRaw = profile.dns?.mode ?? "auto";
  const base: Draft = {
    id: profile.id,
    name: profile.name,
    kind: "none",
    host: "",
    port: "",
    useAuth: false,
    auth: { user: "", password: "", dirty: false },
    viaConnection: "",
    wgEndpoint: "",
    wgAddress: "",
    wgAllowedIps: "",
    wgPeerKey: "",
    vpnEngine: "open_vpn",
    vpnConfig: "",
    dns:
      dnsRaw === "passthrough"
        ? "passthrough"
        : dnsRaw === "tunnel"
        ? "tunnel"
        : dnsRaw === "custom"
        ? "custom"
        : "auto",
    dnsCustomServer:
      profile.dns && profile.dns.mode === "custom" ? profile.dns.server : "",
    isNew: false,
  };
  if (profile.kind === "socks5" || profile.kind === "http") {
    base.kind = profile.kind;
    base.host = profile.host;
    base.port = String(profile.port);
    base.useAuth = !!profile.auth;
  } else if (profile.kind === "ssh_jump") {
    base.kind = "ssh_jump";
    base.viaConnection = profile.viaConnection;
  } else if (profile.kind === "wireguard") {
    base.kind = "wireguard";
    base.wgEndpoint = profile.endpoint;
    base.wgAddress = profile.address;
    base.wgAllowedIps = profile.allowedIps.join(", ");
    base.wgPeerKey = profile.peerPublicKey;
  } else if (profile.kind === "external_vpn") {
    base.kind = "external_vpn";
    base.vpnEngine = profile.engine;
    base.vpnConfig = profile.config;
  }
  return base;
}

function buildProfile(draft: Draft): EgressProfile {
  const dns =
    draft.dns === "tunnel"
      ? { mode: "tunnel" as const }
      : draft.dns === "passthrough"
      ? { mode: "passthrough" as const }
      : draft.dns === "custom"
      ? { mode: "custom" as const, server: draft.dnsCustomServer.trim() }
      : null;
  if (draft.kind === "none") {
    return { id: draft.id, name: draft.name.trim() || draft.id, kind: "none", dns };
  }
  if (draft.kind === "ssh_jump") {
    return {
      id: draft.id,
      name: draft.name.trim() || draft.id,
      kind: "ssh_jump",
      viaConnection: draft.viaConnection.trim(),
      dns,
    };
  }
  if (draft.kind === "wireguard") {
    const credentialId = `pier-x.egress.${draft.id}`;
    return {
      id: draft.id,
      name: draft.name.trim() || draft.id,
      kind: "wireguard",
      endpoint: draft.wgEndpoint.trim(),
      address: draft.wgAddress.trim(),
      allowedIps: draft.wgAllowedIps
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      privateKey: { credentialId },
      peerPublicKey: draft.wgPeerKey.trim(),
      dns,
    };
  }
  if (draft.kind === "external_vpn") {
    return {
      id: draft.id,
      name: draft.name.trim() || draft.id,
      kind: "external_vpn",
      engine: draft.vpnEngine,
      config: draft.vpnConfig.trim(),
      dns,
    };
  }
  const credentialId = `pier-x.egress.${draft.id}`;
  const port = Number.parseInt(draft.port, 10);
  return {
    id: draft.id,
    name: draft.name.trim() || draft.id,
    kind: draft.kind,
    host: draft.host.trim(),
    port: Number.isFinite(port) && port > 0 ? port : 1080,
    auth: draft.useAuth ? { credentialId } : null,
    dns,
  };
}

export default function EgressProfilesDialog({ open, onClose }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const { profiles, refresh, save, remove, setBasicAuth, clearCredential, vpnStart, vpnStop } =
    useEgressStore();
  const { connections, refresh: refreshConnections } = useConnectionStore();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft>(() => emptyDraft());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      void refresh();
      void refreshConnections();
    }
  }, [open, refresh, refreshConnections]);

  // Reset selection when dialog opens, prefer first profile if any.
  useEffect(() => {
    if (!open) return;
    if (profiles.length > 0) {
      const first = profiles[0];
      setSelectedId(first.id);
      setDraft(toDraft(first));
    } else {
      setSelectedId(null);
      setDraft(emptyDraft());
    }
    setError("");
  }, [open, profiles.length]);

  // Esc closes.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const sortedProfiles = useMemo(
    () => [...profiles].sort((a, b) => a.name.localeCompare(b.name)),
    [profiles],
  );

  if (!open) return null;

  function selectProfile(id: string) {
    const p = profiles.find((x) => x.id === id);
    if (!p) return;
    setSelectedId(id);
    setDraft(toDraft(p));
    setError("");
  }

  function startNew() {
    const next = emptyDraft();
    setSelectedId(null);
    setDraft(next);
    setError("");
  }

  async function handleSave() {
    if (busy) return;
    if ((draft.kind === "socks5" || draft.kind === "http") && !draft.host.trim()) {
      setError(t("Host must not be empty."));
      return;
    }
    if (draft.kind === "ssh_jump" && !draft.viaConnection.trim()) {
      setError(t("Choose a saved SSH connection to jump through."));
      return;
    }
    if (draft.kind === "wireguard" && !draft.wgEndpoint.trim()) {
      setError(t("WireGuard endpoint (host:port) must not be empty."));
      return;
    }
    if (draft.kind === "external_vpn" && !draft.vpnConfig.trim()) {
      setError(t("VPN config path / hostname must not be empty."));
      return;
    }
    if (draft.dns === "custom" && !draft.dnsCustomServer.trim()) {
      setError(t("Custom DNS server must not be empty."));
      return;
    }
    setBusy(true);
    try {
      const usesBasicAuth = draft.kind === "socks5" || draft.kind === "http";
      // Save the credential first when auth is enabled and dirty;
      // the backend resolves it on the next connect.
      if (usesBasicAuth && draft.useAuth && draft.auth.dirty) {
        await setBasicAuth(`pier-x.egress.${draft.id}`, draft.auth.user, draft.auth.password);
      }
      // Clear any stale credential when auth was just disabled.
      if (usesBasicAuth && !draft.useAuth) {
        await clearCredential(`pier-x.egress.${draft.id}`).catch(() => undefined);
      }
      const profile = buildProfile(draft);
      await save(profile);
      setError("");
      setDraft({ ...draft, isNew: false, auth: { ...draft.auth, dirty: false } });
      setSelectedId(profile.id);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleVpnStart() {
    if (!selectedId || busy) return;
    setBusy(true);
    try {
      await vpnStart(selectedId);
      setError("");
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleVpnStop() {
    if (!selectedId || busy) return;
    setBusy(true);
    try {
      await vpnStop(selectedId);
      setError("");
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete() {
    if (!selectedId || busy) return;
    if (!window.confirm(t("Delete this egress profile? Connections that referenced it will fall back to direct."))) {
      return;
    }
    setBusy(true);
    try {
      await remove(selectedId);
      setError("");
      startNew();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  }

  return createPortal(
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--egress" style={{ ...dialogStyle, minWidth: 640 }} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Shield size={13} />
            {t("Egress profiles")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body" style={{ display: "grid", gridTemplateColumns: "200px 1fr", gap: "var(--sp-3)", padding: "var(--sp-3)" }}>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-1)", borderRight: "1px solid var(--line)", paddingRight: "var(--sp-3)" }}>
            {sortedProfiles.length === 0 && (
              <div className="dlg-row-hint">{t("No egress profiles yet.")}</div>
            )}
            {sortedProfiles.map((p) => (
              <button
                key={p.id}
                type="button"
                className={"dlg-opt" + (selectedId === p.id ? " active" : "")}
                onClick={() => selectProfile(p.id)}
                style={{ justifyContent: "flex-start", textAlign: "left" }}
              >
                <span style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 2 }}>
                  <span>{p.name || p.id}</span>
                  <span className="dlg-row-hint" style={{ fontSize: "var(--size-micro)" }}>
                    {p.kind}
                    {(p.kind === "socks5" || p.kind === "http") && ` ${p.host}:${p.port}`}
                    {p.kind === "ssh_jump" && ` via ${p.viaConnection}`}
                    {p.kind === "wireguard" && ` ${p.endpoint}`}
                    {p.kind === "external_vpn" && ` ${p.engine}`}
                  </span>
                </span>
              </button>
            ))}
            <button type="button" className="gb-btn" onClick={startNew} style={{ marginTop: "var(--sp-2)", justifyContent: "center" }}>
              <Plus size={12} />
              {t("New profile")}
            </button>
          </div>

          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Name")}</label>
              <input
                className="dlg-input"
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.currentTarget.value })}
                placeholder={t("Office SOCKS")}
              />
            </div>

            <div className="dlg-row">
              <label className="dlg-row-label">{t("Kind")}</label>
              <div className="dlg-opts" role="radiogroup">
                {(["none", "socks5", "http", "ssh_jump", "wireguard", "external_vpn"] as const).map(
                  (k) => (
                    <button
                      key={k}
                      type="button"
                      role="radio"
                      aria-checked={draft.kind === k}
                      className={"dlg-opt" + (draft.kind === k ? " active" : "")}
                      onClick={() => setDraft({ ...draft, kind: k })}
                    >
                      {k === "none"
                        ? t("Direct")
                        : k === "socks5"
                        ? "SOCKS5"
                        : k === "http"
                        ? "HTTP CONNECT"
                        : k === "ssh_jump"
                        ? t("SSH jump")
                        : k === "wireguard"
                        ? "WireGuard"
                        : t("External VPN")}
                    </button>
                  ),
                )}
              </div>
            </div>

            {draft.kind === "ssh_jump" && (
              <>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Jump host")}</label>
                  <select
                    className="dlg-input"
                    value={draft.viaConnection}
                    onChange={(e) => setDraft({ ...draft, viaConnection: e.currentTarget.value })}
                  >
                    <option value="">{t("Select a saved SSH connection…")}</option>
                    {connections.map((c) => (
                      <option key={c.index} value={c.name}>
                        {c.name} ({c.user}@{c.host}:{c.port})
                      </option>
                    ))}
                    {draft.viaConnection
                      && !connections.some((c) => c.name === draft.viaConnection) && (
                        <option value={draft.viaConnection}>
                          {t("(missing)")}: {draft.viaConnection}
                        </option>
                      )}
                  </select>
                </div>
                <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
                  {t("Multi-hop: the jump host honours its own egress (depth ≤ 8, cycle-checked).")}
                </div>
              </>
            )}

            {draft.kind === "wireguard" && (
              <>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Endpoint")}</label>
                  <input
                    className="dlg-input mono"
                    value={draft.wgEndpoint}
                    onChange={(e) => setDraft({ ...draft, wgEndpoint: e.currentTarget.value })}
                    placeholder="vpn.example.com:51820"
                  />
                </div>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Address (CIDR)")}</label>
                  <input
                    className="dlg-input mono"
                    value={draft.wgAddress}
                    onChange={(e) => setDraft({ ...draft, wgAddress: e.currentTarget.value })}
                    placeholder="10.0.0.2/32"
                  />
                </div>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("AllowedIPs")}</label>
                  <input
                    className="dlg-input mono"
                    value={draft.wgAllowedIps}
                    onChange={(e) => setDraft({ ...draft, wgAllowedIps: e.currentTarget.value })}
                    placeholder="10.0.0.0/24, 192.168.1.0/24"
                  />
                </div>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Peer public key")}</label>
                  <input
                    className="dlg-input mono"
                    value={draft.wgPeerKey}
                    onChange={(e) => setDraft({ ...draft, wgPeerKey: e.currentTarget.value })}
                    placeholder="base64-encoded public key"
                  />
                </div>
                <div className="status-note status-note--warn">
                  {t("WireGuard runs as a system VPN via wg-quick. Requires admin/root and writes a config file under ~/.config/pier-x/egress/<id>.conf. Click \"Start VPN\" below after Save.")}
                </div>
              </>
            )}

            {draft.kind === "external_vpn" && (
              <>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Engine")}</label>
                  <div className="dlg-opts" role="radiogroup">
                    {(["open_vpn", "open_connect"] as const).map((eng) => (
                      <button
                        key={eng}
                        type="button"
                        role="radio"
                        aria-checked={draft.vpnEngine === eng}
                        className={"dlg-opt" + (draft.vpnEngine === eng ? " active" : "")}
                        onClick={() => setDraft({ ...draft, vpnEngine: eng })}
                      >
                        {eng === "open_vpn" ? "OpenVPN" : "OpenConnect"}
                      </button>
                    ))}
                  </div>
                </div>
                <div className="dlg-row">
                  <label className="dlg-row-label">
                    {draft.vpnEngine === "open_vpn" ? t("Config (.ovpn)") : t("Hostname / config")}
                  </label>
                  <input
                    className="dlg-input mono"
                    value={draft.vpnConfig}
                    onChange={(e) => setDraft({ ...draft, vpnConfig: e.currentTarget.value })}
                    placeholder={
                      draft.vpnEngine === "open_vpn"
                        ? "/path/to/profile.ovpn"
                        : "vpn.corp.example.com"
                    }
                  />
                </div>
                <div className="status-note status-note--warn">
                  {t("External VPN runs as a system VPN. Requires admin/root and credentials are typed inline by the VPN client itself (Pier-X does not capture them). Click \"Start VPN\" after Save.")}
                </div>
              </>
            )}

            {(draft.kind === "socks5" || draft.kind === "http") && (
              <>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Proxy host")}</label>
                  <div style={{ display: "grid", gridTemplateColumns: "1fr 88px", gap: "var(--sp-2)" }}>
                    <input
                      className="dlg-input mono"
                      value={draft.host}
                      onChange={(e) => setDraft({ ...draft, host: e.currentTarget.value })}
                      placeholder="proxy.example.com"
                    />
                    <input
                      className="dlg-input"
                      value={draft.port}
                      onChange={(e) => setDraft({ ...draft, port: e.currentTarget.value })}
                      placeholder={t("Port")}
                    />
                  </div>
                </div>

                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Auth")}</label>
                  <div className="dlg-opts" role="radiogroup">
                    <button
                      type="button"
                      role="radio"
                      aria-checked={!draft.useAuth}
                      className={"dlg-opt" + (!draft.useAuth ? " active" : "")}
                      onClick={() => setDraft({ ...draft, useAuth: false })}
                    >
                      {t("None")}
                    </button>
                    <button
                      type="button"
                      role="radio"
                      aria-checked={draft.useAuth}
                      className={"dlg-opt" + (draft.useAuth ? " active" : "")}
                      onClick={() => setDraft({ ...draft, useAuth: true })}
                    >
                      {t("Basic (user / password)")}
                    </button>
                  </div>
                </div>

                {draft.useAuth && (
                  <>
                    <div className="dlg-row">
                      <label className="dlg-row-label">{t("Username")}</label>
                      <input
                        className="dlg-input"
                        value={draft.auth.user}
                        onChange={(e) =>
                          setDraft({ ...draft, auth: { ...draft.auth, user: e.currentTarget.value, dirty: true } })
                        }
                      />
                    </div>
                    <div className="dlg-row">
                      <label className="dlg-row-label">{t("Password")}</label>
                      <input
                        className="dlg-input"
                        type="password"
                        value={draft.auth.password}
                        placeholder={draft.isNew ? "" : t("Leave blank to keep current password")}
                        onChange={(e) =>
                          setDraft({ ...draft, auth: { ...draft.auth, password: e.currentTarget.value, dirty: true } })
                        }
                      />
                    </div>
                  </>
                )}
              </>
            )}

            <div className="dlg-row">
              <label className="dlg-row-label">{t("DNS")}</label>
              <div className="dlg-opts" role="radiogroup">
                {(["auto", "passthrough", "tunnel", "custom"] as const).map((d) => (
                  <button
                    key={d}
                    type="button"
                    role="radio"
                    aria-checked={draft.dns === d}
                    className={"dlg-opt" + (draft.dns === d ? " active" : "")}
                    onClick={() => setDraft({ ...draft, dns: d })}
                  >
                    {d === "auto"
                      ? t("Auto")
                      : d === "passthrough"
                      ? t("Local resolve")
                      : d === "tunnel"
                      ? t("Resolve via tunnel")
                      : t("Custom DNS server")}
                  </button>
                ))}
              </div>
            </div>

            {draft.dns === "custom" && (
              <div className="dlg-row">
                <label className="dlg-row-label">{t("DNS server")}</label>
                <input
                  className="dlg-input mono"
                  value={draft.dnsCustomServer}
                  onChange={(e) => setDraft({ ...draft, dnsCustomServer: e.currentTarget.value })}
                  placeholder="8.8.8.8 / 1.1.1.1:5353"
                />
              </div>
            )}

            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="gb-btn"
            disabled={!selectedId || busy}
            onClick={() => void handleDelete()}
          >
            <Trash2 size={12} />
            {t("Delete")}
          </button>
          {(draft.kind === "wireguard" || draft.kind === "external_vpn") && (
            <>
              <button
                type="button"
                className="gb-btn"
                disabled={!selectedId || busy}
                onClick={() => void handleVpnStart()}
                title={t("Spawn the system VPN client; may prompt for admin")}
              >
                {t("Start VPN")}
              </button>
              <button
                type="button"
                className="gb-btn"
                disabled={!selectedId || busy}
                onClick={() => void handleVpnStop()}
              >
                {t("Stop VPN")}
              </button>
            </>
          )}
          <div style={{ flex: 1 }} />
          <button type="button" className="gb-btn" onClick={onClose}>{t("Close")}</button>
          <button
            type="button"
            className="gb-btn primary"
            disabled={busy}
            onClick={() => void handleSave()}
          >
            {t("Save")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
