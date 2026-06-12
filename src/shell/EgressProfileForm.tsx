import Select from "../components/Select";
import { useI18n } from "../i18n/useI18n";
import type {
  EgressProfile,
  OpenConnectProtocol,
  SavedSshConnection,
} from "../lib/types";
import { useEgressStore } from "../stores/useEgressStore";

/** Editable kinds in the profile form. `none` is the explicit
 *  "Direct" entry — kept distinct from "no profile selected". */
export type EditableEgressKind =
  | "none"
  | "socks5"
  | "http"
  | "ssh_jump"
  | "wireguard"
  | "external_vpn";

type DraftAuth = { user: string; password: string; dirty: boolean };

/** Form-local working copy of an `EgressProfile`. All fields are
 *  strings so inputs bind directly; `buildEgressProfile` converts
 *  back to the wire shape. */
export type EgressDraft = {
  id: string;
  name: string;
  kind: EditableEgressKind;
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
  /** external_vpn + open_connect only: WebVPN dialect. */
  vpnProtocol: OpenConnectProtocol;
  dns: "auto" | "tunnel" | "passthrough" | "custom";
  /** Only meaningful when dns === "custom". */
  dnsCustomServer: string;
  isNew: boolean;
};

export function newEgressId() {
  return `egress-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

export function emptyEgressDraft(): EgressDraft {
  return {
    id: newEgressId(),
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
    vpnProtocol: "anyconnect",
    dns: "auto",
    dnsCustomServer: "",
    isNew: true,
  };
}

export function egressProfileToDraft(profile: EgressProfile): EgressDraft {
  const dnsRaw = profile.dns?.mode ?? "auto";
  const base: EgressDraft = {
    ...emptyEgressDraft(),
    id: profile.id,
    name: profile.name,
    kind: "none",
    port: "",
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
    base.vpnProtocol = profile.protocol ?? "anyconnect";
  }
  return base;
}

export function buildEgressProfile(draft: EgressDraft): EgressProfile {
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
      protocol: draft.vpnEngine === "open_connect" ? draft.vpnProtocol : null,
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

/** Returns the untranslated message key for the first invalid field,
 *  or null when the draft is saveable. Callers wrap with t(). */
export function validateEgressDraft(draft: EgressDraft): string | null {
  if ((draft.kind === "socks5" || draft.kind === "http") && !draft.host.trim()) {
    return "Host must not be empty.";
  }
  if (draft.kind === "ssh_jump" && !draft.viaConnection.trim()) {
    return "Choose a saved SSH connection to jump through.";
  }
  // WireGuard `conf_path` may be empty — backend then falls back to
  // the app-managed slot. A missing file surfaces on Start VPN.
  if (draft.kind === "external_vpn" && !draft.vpnConfig.trim()) {
    return "VPN config path / hostname must not be empty.";
  }
  if (draft.dns === "custom" && !draft.dnsCustomServer.trim()) {
    return "Custom DNS server must not be empty.";
  }
  return null;
}

/** Persist the draft: store the basic-auth credential first when it
 *  was (re)typed, clear it when auth was just disabled, then upsert
 *  the profile. Throws on IPC failure — callers surface the error. */
export async function persistEgressDraft(draft: EgressDraft): Promise<EgressProfile> {
  const { setBasicAuth, clearCredential, save } = useEgressStore.getState();
  const usesBasicAuth = draft.kind === "socks5" || draft.kind === "http";
  if (usesBasicAuth && draft.useAuth && draft.auth.dirty) {
    await setBasicAuth(`pier-x.egress.${draft.id}`, draft.auth.user, draft.auth.password);
  }
  if (usesBasicAuth && !draft.useAuth) {
    await clearCredential(`pier-x.egress.${draft.id}`).catch(() => undefined);
  }
  const profile = buildEgressProfile(draft);
  await save(profile);
  return profile;
}

const OPENCONNECT_PROTOCOLS: { value: OpenConnectProtocol; label: string }[] = [
  { value: "anyconnect", label: "AnyConnect (Cisco / ocserv)" },
  { value: "nc", label: "Juniper Network Connect" },
  { value: "gp", label: "GlobalProtect (Palo Alto)" },
  { value: "pulse", label: "Pulse / Ivanti Connect Secure" },
  { value: "f5", label: "F5 BIG-IP" },
  { value: "fortinet", label: "Fortinet FortiGate" },
  { value: "array", label: "Array Networks AG" },
];

type Props = {
  draft: EgressDraft;
  onChange: (next: EgressDraft) => void;
  /** Saved SSH connections for the ssh_jump picker. */
  connections: SavedSshConnection[];
};

/** Field rows of the egress profile editor — shared between the
 *  standalone management dialog and the inline pane in
 *  NewConnectionDialog. Validation errors and footer actions stay
 *  with the host component. */
export default function EgressProfileForm({ draft, onChange, connections }: Props) {
  const { t } = useI18n();

  return (
    <>
      <div className="dlg-row">
        <label className="dlg-row-label">{t("Name")}</label>
        <input
          className="dlg-input"
          value={draft.name}
          onChange={(e) => onChange({ ...draft, name: e.currentTarget.value })}
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
                onClick={() => onChange({ ...draft, kind: k })}
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
                  : "OpenVPN / OpenConnect"}
              </button>
            ),
          )}
        </div>
      </div>

      {draft.kind === "ssh_jump" && (
        <>
          <div className="dlg-row">
            <label className="dlg-row-label">{t("Jump host")}</label>
            <Select
              className="dlg-input"
              value={draft.viaConnection}
              onChange={(val) => onChange({ ...draft, viaConnection: val })}
              items={[
                { value: "", label: t("Select a saved SSH connection…") },
                ...connections.map((c) => ({
                  value: c.name,
                  label: `${c.name} (${c.user}@${c.host}:${c.port})`,
                })),
                ...(draft.viaConnection
                && !connections.some((c) => c.name === draft.viaConnection)
                  ? [
                      {
                        value: draft.viaConnection,
                        label: `${t("(missing)")}: ${draft.viaConnection}`,
                      },
                    ]
                  : []),
              ]}
            />
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
              onChange={(e) => onChange({ ...draft, wgConfPath: e.currentTarget.value })}
              placeholder="/etc/wireguard/wg0.conf"
            />
          </div>
          <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
            {t("Path to a wg-quick compatible .conf file (Interface + Peer sections, including PrivateKey). Leave empty to use ~/.config/pier-x/egress/<id>.conf. Pier-X never reads or stores the private key — wg-quick does.")}
          </div>
          <div className="status-note status-note--warn">
            {t("WireGuard runs as a system VPN (wg-quick on macOS/Linux, the WireGuard tunnel service on Windows). Requires admin/root and the AllowedIPs in the conf will install routes on the host (may collide with your local LAN — narrow AllowedIPs to the subnets you actually need). Click \"Start VPN\" after Save.")}
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
                  onClick={() => onChange({ ...draft, vpnEngine: eng })}
                >
                  {eng === "open_vpn" ? "OpenVPN" : "OpenConnect"}
                </button>
              ))}
            </div>
          </div>
          {draft.vpnEngine === "open_connect" && (
            <>
              <div className="dlg-row">
                <label className="dlg-row-label">{t("Protocol")}</label>
                <Select
                  className="dlg-input"
                  value={draft.vpnProtocol}
                  onChange={(val) =>
                    onChange({ ...draft, vpnProtocol: val as OpenConnectProtocol })
                  }
                  items={OPENCONNECT_PROTOCOLS}
                />
              </div>
              <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
                {t("WebVPN gateways speaking one of these dialects work through openconnect --protocol. Proprietary clients (Sangfor EasyConnect, …) are not supported.")}
              </div>
            </>
          )}
          <div className="dlg-row">
            <label className="dlg-row-label">
              {draft.vpnEngine === "open_vpn" ? t("Config (.ovpn)") : t("Hostname / config")}
            </label>
            <input
              className="dlg-input mono"
              value={draft.vpnConfig}
              onChange={(e) => onChange({ ...draft, vpnConfig: e.currentTarget.value })}
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
                onChange={(e) => onChange({ ...draft, host: e.currentTarget.value })}
                placeholder="proxy.example.com"
              />
              <input
                className="dlg-input"
                value={draft.port}
                onChange={(e) => onChange({ ...draft, port: e.currentTarget.value })}
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
                onClick={() => onChange({ ...draft, useAuth: false })}
              >
                {t("None")}
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={draft.useAuth}
                className={"dlg-opt" + (draft.useAuth ? " active" : "")}
                onClick={() => onChange({ ...draft, useAuth: true })}
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
                    onChange({ ...draft, auth: { ...draft.auth, user: e.currentTarget.value, dirty: true } })
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
                    onChange({ ...draft, auth: { ...draft.auth, password: e.currentTarget.value, dirty: true } })
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
              onClick={() => onChange({ ...draft, dns: d })}
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
            onChange={(e) => onChange({ ...draft, dnsCustomServer: e.currentTarget.value })}
            placeholder="8.8.8.8 / 1.1.1.1:5353"
          />
        </div>
      )}
    </>
  );
}
