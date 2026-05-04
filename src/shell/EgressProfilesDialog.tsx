import { ClipboardPaste, Plus, Shield, Trash2, X, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import IconButton from "../components/IconButton";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import { readClipboardText } from "../lib/clipboard";
import {
  egressProfileTest,
  egressWgConfSave,
  type EgressProbeResult,
} from "../lib/commands";
import { matchJumpConnection, parseEgressClipboard } from "../lib/egressImport";
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
  /** wireguard only: absolute path to a wg-quick .conf file. Empty
   *  string means the app-managed default slot. */
  wgConfPath: string;
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
    wgConfPath: "",
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
    wgConfPath: "",
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
    base.wgConfPath = profile.confPath;
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
    return {
      id: draft.id,
      name: draft.name.trim() || draft.id,
      kind: "wireguard",
      confPath: draft.wgConfPath.trim(),
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
  const {
    profiles,
    vpnStatus,
    refresh,
    save,
    remove,
    setBasicAuth,
    clearCredential,
    vpnStart,
    vpnStop,
    refreshVpnStatus,
  } = useEgressStore();
  const { connections, refresh: refreshConnections } = useConnectionStore();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft>(() => emptyDraft());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [probe, setProbe] = useState<EgressProbeResult | null>(null);
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    if (open) {
      void refresh();
      void refreshConnections();
      void refreshVpnStatus();
    }
  }, [open, refresh, refreshConnections, refreshVpnStatus]);

  // Clear stale probe results whenever the user switches profiles
  // — a green check from one profile is misleading for another.
  useEffect(() => {
    setProbe(null);
  }, [selectedId, draft.kind]);

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
    // WireGuard `conf_path` may be empty — backend then falls back to
    // ~/.config/pier-x/egress/<id>.conf. No client-side check needed;
    // a missing file surfaces as a typed error on Start VPN.
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

  /// Probe the *saved* version of the selected profile (so the
  /// caller can verify "the thing on disk works" rather than
  /// "the half-edited form would work"). Pre-saved profiles can
  /// still be probed without selecting one — id = null = direct.
  async function handleTest() {
    if (probing) return;
    setProbing(true);
    setProbe(null);
    try {
      const result = await egressProfileTest(selectedId, undefined, undefined);
      setProbe(result);
    } catch (e) {
      setProbe({
        ok: false,
        latencyMs: null,
        error: formatError(e),
        target: "1.1.1.1:443",
      });
    } finally {
      setProbing(false);
    }
  }

  /// Read the clipboard, sniff a wg-quick conf / SOCKS or HTTP proxy
  /// URL / OpenSSH ProxyJump, and seed `draft` with the parsed
  /// fields. WireGuard configs are written to the app-managed slot
  /// (`<data_dir>/egress/<id>.conf`) right away because that's the
  /// path `vpn_subprocess::plan_for` falls back to when `confPath`
  /// is empty — saving the profile separately would otherwise
  /// dead-end at `start VPN` with "config missing".
  async function handleImportFromClipboard() {
    if (busy) return;
    setError("");
    const raw = await readClipboardText();
    if (!raw.trim()) {
      setError(t("Clipboard is empty."));
      return;
    }
    const parsed = parseEgressClipboard(raw);
    if (!parsed) {
      setError(
        t(
          "Couldn't recognise a SOCKS5 / HTTP proxy URL, OpenSSH ProxyJump line, or wg-quick .conf in the clipboard.",
        ),
      );
      return;
    }
    const baseDraft: Draft = { ...emptyDraft(), name: draft.name || "" };
    if (parsed.kind === "socks5" || parsed.kind === "http") {
      const next: Draft = {
        ...baseDraft,
        kind: parsed.kind,
        host: parsed.patch.host,
        port: String(parsed.patch.port),
        useAuth: parsed.patch.useAuth,
        auth: {
          user: parsed.patch.authUser,
          password: parsed.patch.authPassword,
          dirty: parsed.patch.useAuth,
        },
        name: baseDraft.name || `${parsed.kind}-${parsed.patch.host}`,
      };
      setSelectedId(null);
      setDraft(next);
      setError("");
      return;
    }
    if (parsed.kind === "ssh_jump") {
      const matched = matchJumpConnection(parsed.jumpHint, connections);
      const next: Draft = {
        ...baseDraft,
        kind: "ssh_jump",
        viaConnection: matched ?? "",
        name:
          baseDraft.name ||
          `jump-${parsed.jumpHint.user ? parsed.jumpHint.user + "-" : ""}${parsed.jumpHint.host}`,
      };
      setSelectedId(null);
      setDraft(next);
      setError(
        matched
          ? ""
          : t(
              "No saved SSH connection matches {hint} — pick one from the dropdown or save it first, then re-import.",
              {
                hint: `${parsed.jumpHint.user ? parsed.jumpHint.user + "@" : ""}${parsed.jumpHint.host}:${parsed.jumpHint.port}`,
              },
            ),
      );
      return;
    }
    if (parsed.kind === "wireguard") {
      const next: Draft = {
        ...baseDraft,
        kind: "wireguard",
        wgConfPath: "",
        name: baseDraft.name || `wg-${baseDraft.id.slice(-6)}`,
      };
      setBusy(true);
      try {
        await egressWgConfSave(next.id, parsed.wgConf);
        setSelectedId(null);
        setDraft(next);
        setError(
          t(
            "Imported wg-quick conf into the managed slot. Save the profile, then click Start VPN.",
          ),
        );
      } catch (e) {
        setError(formatError(e));
      } finally {
        setBusy(false);
      }
      return;
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
            {sortedProfiles.map((p) => {
              const isVpn = p.kind === "wireguard" || p.kind === "external_vpn";
              const status = isVpn ? vpnStatus[p.id] : undefined;
              const dotColor =
                status === true
                  ? "var(--pos)"
                  : status === false
                  ? "var(--neg)"
                  : "var(--dim)";
              const dotTitle =
                status === true
                  ? t("VPN running")
                  : status === false
                  ? t("VPN started this session but exited")
                  : t("VPN never started in this session");
              return (
                <button
                  key={p.id}
                  type="button"
                  className={"dlg-opt" + (selectedId === p.id ? " active" : "")}
                  onClick={() => selectProfile(p.id)}
                  style={{ justifyContent: "flex-start", textAlign: "left" }}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", flex: 1, minWidth: 0 }}>
                    {isVpn && (
                      <span
                        title={dotTitle}
                        style={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          backgroundColor: dotColor,
                          flexShrink: 0,
                        }}
                      />
                    )}
                    <span style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 2, minWidth: 0 }}>
                      <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{p.name || p.id}</span>
                      <span className="dlg-row-hint" style={{ fontSize: "var(--size-micro)" }}>
                        {p.kind}
                        {(p.kind === "socks5" || p.kind === "http") && ` ${p.host}:${p.port}`}
                        {p.kind === "ssh_jump" && ` via ${p.viaConnection}`}
                        {p.kind === "wireguard" && ` ${p.confPath || "default"}`}
                        {p.kind === "external_vpn" && ` ${p.engine}`}
                      </span>
                    </span>
                  </span>
                </button>
              );
            })}
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
                  <label className="dlg-row-label">{t("Conf path")}</label>
                  <input
                    className="dlg-input mono"
                    value={draft.wgConfPath}
                    onChange={(e) => setDraft({ ...draft, wgConfPath: e.currentTarget.value })}
                    placeholder="/etc/wireguard/wg0.conf"
                  />
                </div>
                <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
                  {t("Path to a wg-quick compatible .conf file (Interface + Peer sections, including PrivateKey). Leave empty to use ~/.config/pier-x/egress/<id>.conf. Pier-X never reads or stores the private key — wg-quick does.")}
                </div>
                <div className="status-note status-note--warn">
                  {t("WireGuard runs as a system VPN via wg-quick. Requires admin/root and the AllowedIPs in the conf will install routes on the host (may collide with your local LAN — narrow AllowedIPs to the subnets you actually need). Click \"Start VPN\" after Save.")}
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

            {probe && (
              <div
                className={
                  "status-note " + (probe.ok ? "status-note--ok" : "status-note--error")
                }
              >
                {probe.ok
                  ? t("✓ Reached %s in %dms").replace("%s", probe.target).replace("%d", String(probe.latencyMs ?? 0))
                  : t("✗ %s failed (%dms): %s")
                      .replace("%s", probe.target)
                      .replace("%d", String(probe.latencyMs ?? 0))
                      .replace("%s", probe.error)}
              </div>
            )}
          </div>
        </div>
        <div className="dlg-foot">
          <button
            type="button"
            className="gb-btn"
            disabled={busy}
            onClick={() => void handleImportFromClipboard()}
            title={t(
              "Detect a SOCKS5/HTTP proxy URL, OpenSSH ProxyJump string, or wg-quick .conf in the clipboard and pre-fill the form.",
            )}
          >
            <ClipboardPaste size={12} />
            {t("Import from clipboard")}
          </button>
          <button
            type="button"
            className="gb-btn"
            disabled={!selectedId || busy}
            onClick={() => void handleDelete()}
          >
            <Trash2 size={12} />
            {t("Delete")}
          </button>
          <button
            type="button"
            className="gb-btn"
            disabled={probing || busy}
            onClick={() => void handleTest()}
            title={t("Probe TCP reachability through the saved profile (target: 1.1.1.1:443, 5s timeout)")}
          >
            <Zap size={12} />
            {probing ? t("Testing…") : t("Test")}
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
