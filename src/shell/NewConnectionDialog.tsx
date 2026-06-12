import { Key, Server, Shield, ShieldCheck, X } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import ComboInput from "../components/ComboInput";
import IconButton from "../components/IconButton";
import Select from "../components/Select";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import type { SavedSshConnection } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useEgressStore } from "../stores/useEgressStore";
import { useSudoStore } from "../stores/useSudoStore";
import EgressProfileForm, {
  type EgressDraft,
  egressProfileToDraft,
  emptyEgressDraft,
  persistEgressDraft,
  validateEgressDraft,
} from "./EgressProfileForm";

const EgressProfilesDialog = lazy(() => import("./EgressProfilesDialog"));

type ConnectionDraft = {
  index?: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  keyPath: string;
  group: string;
  envTag: string;
  egressId: string;
  autoElevate: boolean;
};

type Props = {
  open: boolean;
  onClose: () => void;
  onConnect: (params: {
    name: string;
    host: string;
    port: number;
    user: string;
    authKind: string;
    password: string;
    keyPath: string;
  }) => void;
  /** Connect using a saved connection index — backend resolves credentials. */
  onConnectSaved?: (index: number) => void;
  /** Fired after a successful save/edit of a saved connection. Lets the
   *  caller propagate the freshly-typed password into open tabs that
   *  reference this saved index, so a stalled terminal session (e.g.
   *  one that failed because the keychain entry was missing) can
   *  retry without the user having to manually restart it. */
  onSaved?: (savedIndex: number, password: string, authKind: string) => void;
  initialConnection?: SavedSshConnection | null;
};

function toDraft(connection?: SavedSshConnection | null): ConnectionDraft {
  return {
    index: connection?.index,
    name: connection?.name ?? "",
    host: connection?.host ?? "",
    port: connection?.port ?? 22,
    user: connection?.user ?? "",
    authKind: connection?.authKind ?? "password",
    keyPath: connection?.keyPath ?? "",
    group: connection?.group ?? "",
    envTag: connection?.envTag ?? "",
    egressId: connection?.egressId ?? "",
    autoElevate: connection?.autoElevate ?? false,
  };
}

export default function NewConnectionDialog({ open, onClose, onConnect, onConnectSaved, onSaved, initialConnection }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const save = useConnectionStore((s) => s.save);
  const update = useConnectionStore((s) => s.update);
  const connections = useConnectionStore((s) => s.connections);
  const { profiles: egressProfiles, refresh: refreshEgress } = useEgressStore();
  const isEditing = !!initialConnection;
  const initialDraft = useMemo(() => toDraft(initialConnection), [initialConnection]);
  const [name, setName] = useState(initialDraft.name);
  const [host, setHost] = useState(initialDraft.host);
  const [port, setPort] = useState(String(initialDraft.port));
  const [user, setUser] = useState(initialDraft.user);
  const [authMode, setAuthMode] = useState<"password" | "agent" | "key">(initialDraft.authKind as "password" | "agent" | "key");
  const [password, setPassword] = useState("");
  const [keyPath, setKeyPath] = useState(initialDraft.keyPath);
  // Optional sudo / privilege-escalation password. Stored in the OS
  // keychain under `pier-x.elev.<user>@<host>:<port>` only when set
  // via this field; the host's interactive panels (Docker / firewall
  // / nginx / software) prompt on demand otherwise. Editing an
  // existing connection that already has one shows a "saved" badge
  // but does NOT pre-fill the input — same disclosure model as the
  // SSH password above.
  const [sudoPassword, setSudoPasswordInput] = useState("");
  const [hasStoredSudoPassword, setHasStoredSudoPassword] = useState(false);
  const [autoElevate, setAutoElevate] = useState(initialDraft.autoElevate);
  const [group, setGroup] = useState(initialDraft.group);
  const [envTag, setEnvTag] = useState(initialDraft.envTag);
  const [egressId, setEgressId] = useState(initialDraft.egressId);
  const [egressDialogOpen, setEgressDialogOpen] = useState(false);
  // Inline egress editor (right pane). Opened from the "Configure…"
  // button next to the egress picker; edits the selected profile or
  // drafts a new one without a second stacked dialog.
  const [egressPaneOpen, setEgressPaneOpen] = useState(false);
  const [egressDraft, setEgressDraft] = useState<EgressDraft>(() => emptyEgressDraft());
  const [egressError, setEgressError] = useState("");
  const [egressSaving, setEgressSaving] = useState(false);
  const [error, setError] = useState("");
  // Guards double-submit: `persistConnection` is async and the buttons
  // previously stayed enabled while the IPC was in flight, so a quick
  // second click would insert a duplicate saved connection.
  const [saving, setSaving] = useState(false);
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  // Close on Esc so keyboard users aren't trapped in the dialog.
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Refresh the egress profile list whenever the dialog opens so the
  // dropdown reflects anything the user may have added in another window.
  useEffect(() => {
    if (open) void refreshEgress();
  }, [open, refreshEgress]);

  // Unique sorted list of existing group labels, for the datalist autocomplete.
  const knownGroups = useMemo(() => {
    const seen = new Set<string>();
    for (const c of connections) {
      const g = (c.group ?? "").trim();
      if (g) seen.add(g);
    }
    return Array.from(seen).sort((a, b) => a.localeCompare(b));
  }, [connections]);

  useEffect(() => {
    const next = toDraft(initialConnection);
    setName(next.name);
    setHost(next.host);
    setPort(String(next.port));
    setUser(next.user);
    setAuthMode(next.authKind as "password" | "agent" | "key");
    setPassword("");
    setKeyPath(next.keyPath);
    setSudoPasswordInput("");
    setHasStoredSudoPassword(false);
    setAutoElevate(next.autoElevate);
    setGroup(next.group);
    setEnvTag(next.envTag);
    setEgressId(next.egressId);
    setEgressPaneOpen(false);
    setEgressError("");
    setError("");
  }, [initialConnection, open]);

  // When editing, probe the keychain for an existing elevation
  // password so the UI can show a "Saved" badge without leaking the
  // actual value. Skipped for new connections (nothing to probe).
  useEffect(() => {
    if (!open || !initialConnection) {
      setHasStoredSudoPassword(false);
      return;
    }
    let cancelled = false;
    void cmd
      .getElevationPassword(
        initialConnection.user,
        initialConnection.host,
        initialConnection.port,
      )
      .then((stored) => {
        if (cancelled) return;
        setHasStoredSudoPassword(Boolean(stored && stored.length > 0));
      })
      .catch(() => {
        if (!cancelled) setHasStoredSudoPassword(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, initialConnection]);

  if (!open) return null;

  const p = Number.parseInt(port, 10);
  const isEditingKept = isEditing && initialConnection?.authKind === authMode;
  const canSave =
    host.trim() &&
    user.trim() &&
    Number.isFinite(p) &&
    p > 0 &&
    (authMode === "agent"
      || (authMode === "password"
        ? (password.length > 0 || isEditingKept)
        : (keyPath.trim().length > 0 || isEditingKept)));
  const canDirectConnect =
    host.trim() &&
    user.trim() &&
    Number.isFinite(p) &&
    p > 0 &&
    (authMode === "agent"
      || (authMode === "password"
        ? (password.length > 0 || isEditingKept)
        : (keyPath.trim().length > 0 || isEditingKept)));
  const canSaveAndConnect = canSave && canDirectConnect;

  const connectionName = name.trim() || `${user.trim()}@${host.trim()}`;

  async function persistConnection() {
    const trimmedGroup = group.trim();
    const trimmedEnvTag = envTag.trim();
    const trimmedEgressId = egressId.trim();
    const params = {
      name: connectionName,
      host: host.trim(),
      port: p,
      user: user.trim(),
      authKind: authMode,
      password: authMode === "password" ? password : "",
      keyPath: authMode === "key" ? keyPath.trim() : "",
      group: trimmedGroup ? trimmedGroup : null,
      envTag: trimmedEnvTag ? trimmedEnvTag : null,
      egressId: trimmedEgressId ? trimmedEgressId : "",
      autoElevate,
    };

    if (isEditing && typeof initialDraft.index === "number") {
      await update({
        index: initialDraft.index,
        ...params,
      });
    } else {
      await save(params);
    }

    // Persist the sudo / elevation password if the user typed one.
    // Empty input is a no-op (existing keychain entry stays). To
    // clear an existing entry the user goes to Settings → Security
    // and clicks Forget. Mirrors the SSH password "leave blank to
    // keep" semantics so editing the form never accidentally drops
    // a stored password.
    if (sudoPassword.trim().length > 0) {
      try {
        const trimmedHost = host.trim();
        const trimmedUser = user.trim();
        await cmd.setElevationPassword(
          trimmedUser,
          trimmedHost,
          p,
          sudoPassword,
        );
        // Mirror into the in-memory L1 cache so panels active in this
        // session pick it up without a hydrate round-trip.
        useSudoStore.getState().set(
          {
            host: trimmedHost,
            port: p,
            user: trimmedUser,
            authMode,
            password: "",
            keyPath: "",
            savedConnectionIndex: null,
          },
          sudoPassword,
        );
      } catch (e) {
        console.warn("setElevationPassword failed", e);
      }
    }
  }

  async function handleSave() {
    if (!canSave || saving) return;
    setSaving(true);
    setError("");
    try {
      await persistConnection();
      // After an edit, hand the freshly-typed password back to the
      // caller so it can populate any open tabs that reference this
      // saved-connection index. Skipped for "new" saves (no existing
      // tabs to update) and when nothing relevant was retyped.
      if (
        isEditing
        && onSaved
        && typeof initialDraft.index === "number"
        && (authMode !== "password" || password.length > 0)
      ) {
        onSaved(
          initialDraft.index,
          authMode === "password" ? password : "",
          authMode,
        );
      }
      onClose();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleSaveAndConnect() {
    if (!canSaveAndConnect || saving) return;
    setSaving(true);
    setError("");
    const params = {
      name: connectionName,
      host: host.trim(),
      port: p,
      user: user.trim(),
      authKind: authMode,
      password: authMode === "password" ? password : "",
      keyPath: authMode === "key" ? keyPath.trim() : "",
    };
    try {
      await persistConnection();
      // When editing an existing connection, use the saved-index path so the
      // backend resolves the password from the keychain (avoids sending empty
      // string when the user didn't retype).
      if (isEditing && typeof initialDraft.index === "number" && onConnectSaved) {
        onConnectSaved(initialDraft.index);
      } else {
        onConnect(params);
      }
      onClose();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  }

  /** Open the inline egress editor pre-filled with the selected
   *  profile, or a fresh draft when "Direct" is selected. Clicking
   *  again collapses the pane. */
  function toggleEgressPane() {
    if (egressPaneOpen) {
      setEgressPaneOpen(false);
      return;
    }
    const selected = egressProfiles.find((p) => p.id === egressId);
    setEgressDraft(selected ? egressProfileToDraft(selected) : emptyEgressDraft());
    setEgressError("");
    setEgressPaneOpen(true);
  }

  async function handleEgressSave() {
    if (egressSaving) return;
    const invalid = validateEgressDraft(egressDraft);
    if (invalid) {
      setEgressError(t(invalid));
      return;
    }
    setEgressSaving(true);
    try {
      const profile = await persistEgressDraft(egressDraft);
      // Bind the freshly saved profile to this connection right away —
      // that's the whole point of editing it from here.
      setEgressId(profile.id);
      setEgressError("");
      setEgressPaneOpen(false);
    } catch (e) {
      setEgressError(formatError(e));
    } finally {
      setEgressSaving(false);
    }
  }

  function handleConnect() {
    if (!canDirectConnect) return;
    // Editing an existing connection — prefer the saved-index connect path
    // so the backend resolves secrets from the keychain.
    if (isEditing && typeof initialDraft.index === "number" && onConnectSaved) {
      onConnectSaved(initialDraft.index);
      onClose();
      return;
    }
    onConnect({
      name: connectionName,
      host: host.trim(),
      port: p,
      user: user.trim(),
      authKind: authMode,
      password: authMode === "password" ? password : "",
      keyPath: authMode === "key" ? keyPath.trim() : "",
    });
    onClose();
  }

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className={"dlg dlg--newconn" + (egressPaneOpen ? " dlg--newconn-wide" : "")}
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Server size={13} />
            {t(isEditing ? "Edit SSH connection" : "New SSH connection")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className={"dlg-split" + (egressPaneOpen ? " dlg-split--two" : "")}>
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Name")}</label>
              <input className="dlg-input" onChange={(e) => setName(e.currentTarget.value)} placeholder={t("prod-api / staging")} value={name} />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Group")}</label>
              <ComboInput
                className="dlg-input"
                onChange={(v) => setGroup(v)}
                placeholder={t("Default")}
                value={group}
                suggestions={knownGroups}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Env tag")}</label>
              <ComboInput
                className="dlg-input"
                onChange={(v) => setEnvTag(v)}
                placeholder={t("prod / staging / dev / local")}
                value={envTag}
                suggestions={["prod", "staging", "dev", "local"]}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Egress")}</label>
              <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: "var(--sp-2)" }}>
                <Select
                  className="dlg-input"
                  value={egressId}
                  onChange={(val) => {
                    setEgressId(val);
                    // Keep the open inline editor in sync with the
                    // selection — it always edits "the selected
                    // profile" (unsaved pane edits are discarded).
                    if (egressPaneOpen) {
                      const sel = egressProfiles.find((p) => p.id === val);
                      setEgressDraft(sel ? egressProfileToDraft(sel) : emptyEgressDraft());
                      setEgressError("");
                    }
                  }}
                  items={[
                    { value: "", label: t("Direct (no tunnel)") },
                    ...egressProfiles.map((p) => {
                      const isSystemVpn = p.kind === "wireguard" || p.kind === "external_vpn";
                      const prefix = isSystemVpn ? "⚠ " : "";
                      return { value: p.id, label: `${prefix}${p.name || p.id}` };
                    }),
                    ...(egressId && !egressProfiles.some((p) => p.id === egressId)
                      ? [{ value: egressId, label: `${t("(missing)")}: ${egressId}` }]
                      : []),
                  ]}
                />
                <button
                  type="button"
                  className={"gb-btn" + (egressPaneOpen ? " active" : "")}
                  onClick={toggleEgressPane}
                  title={t("Edit the selected egress profile, or create a new one, in a side pane")}
                >
                  <Shield size={12} />
                  {t("Configure…")}
                </button>
              </div>
            </div>
            {(() => {
              const selected = egressProfiles.find((p) => p.id === egressId);
              if (!selected) return null;
              const isSystemVpn =
                selected.kind === "wireguard" || selected.kind === "external_vpn";
              if (isSystemVpn) {
                return (
                  <div className="dlg-row-hint" style={{ marginLeft: 110, color: "var(--warn)" }}>
                    {t("⚠ System-level VPN. wg-quick / openvpn installs OS routes when started; if its AllowedIPs / pushed routes overlap your local LAN subnet you will lose access to those LAN hosts. Narrow AllowedIPs in the conf to just the subnets you need.")}
                  </div>
                );
              }
              if (selected.kind === "ssh_jump") {
                return (
                  <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
                    {t("Per-connection: this SSH session tunnels through the saved \"%s\" jump host (multi-hop allowed, depth ≤ 8).").replace("%s", selected.viaConnection)}
                  </div>
                );
              }
              if (selected.kind === "socks5" || selected.kind === "http") {
                return (
                  <div className="dlg-row-hint" style={{ marginLeft: 110 }}>
                    {t("Per-connection: only this SSH session goes through the proxy. Host routing untouched.")}
                  </div>
                );
              }
              return null;
            })()}
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Host")}</label>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 88px", gap: "var(--sp-2)" }}>
                <input className="dlg-input" onChange={(e) => setHost(e.currentTarget.value)} placeholder={t("server.example.com")} value={host} />
                <input className="dlg-input" onChange={(e) => setPort(e.currentTarget.value)} value={port} placeholder={t("Port")} />
              </div>
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("User")}</label>
              <input className="dlg-input" onChange={(e) => setUser(e.currentTarget.value)} placeholder={t("root")} value={user} />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Authentication")}</label>
              <div className="dlg-opts" role="radiogroup" aria-label={t("Authentication")}>
                <button
                  type="button"
                  role="radio"
                  aria-checked={authMode === "password"}
                  className={"dlg-opt" + (authMode === "password" ? " active" : "")}
                  onClick={() => setAuthMode("password")}
                >
                  {t("Password")}
                </button>
                <button
                  type="button"
                  role="radio"
                  aria-checked={authMode === "key"}
                  className={"dlg-opt" + (authMode === "key" ? " active" : "")}
                  onClick={() => setAuthMode("key")}
                >
                  {t("Key file")}
                </button>
                <button
                  type="button"
                  role="radio"
                  aria-checked={authMode === "agent"}
                  className={"dlg-opt" + (authMode === "agent" ? " active" : "")}
                  onClick={() => setAuthMode("agent")}
                >
                  {t("Agent")}
                </button>
              </div>
            </div>
            {authMode === "password" && (
              <div className="dlg-row">
                <label className="dlg-row-label">{t("Password")}</label>
                <input className="dlg-input" type="password" onChange={(e) => setPassword(e.currentTarget.value)} placeholder={isEditing ? t("Leave blank to keep current password") : ""} value={password} />
              </div>
            )}
            {authMode === "key" && (
              <>
                <div className="dlg-row">
                  <label className="dlg-row-label">{t("Private key")}</label>
                  <input className="dlg-input mono" onChange={(e) => setKeyPath(e.currentTarget.value)} placeholder={t("~/.ssh/id_ed25519")} value={keyPath} />
                </div>
                <div className="dlg-note">
                  <Key size={11} />
                  <span>
                    {t("Passphrase will be stored in the system keychain")}
                    {connectionName ? (
                      <>
                        {" "}(<span className="mono">{`pier-x.ssh.${connectionName}`}</span>)
                      </>
                    ) : null}
                    .
                  </span>
                </div>
              </>
            )}
            {authMode === "agent" && (
              <div className="dlg-note">{t("Agent auth uses the system SSH agent.")}</div>
            )}
            <div className="dlg-row">
              <label
                className="dlg-row-label"
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "var(--sp-1)",
                }}
              >
                <ShieldCheck size={11} />
                {t("Sudo password")}
              </label>
              <input
                className="dlg-input"
                type="password"
                onChange={(e) => setSudoPasswordInput(e.currentTarget.value)}
                placeholder={
                  hasStoredSudoPassword
                    ? t("Leave blank to keep saved password")
                    : t("Optional — saved to keychain for Docker / firewall / nginx panels")
                }
                value={sudoPassword}
                autoComplete="off"
                spellCheck={false}
              />
            </div>
            <div className="dlg-note">
              <ShieldCheck size={11} />
              <span>
                {hasStoredSudoPassword
                  ? t(
                      "A sudo password is already saved for this host. Pier-X panels (Docker, firewall, nginx, software) use it transparently. Manage / forget from Settings → Security.",
                    )
                  : t(
                      "Optional. When set, panels that need root (Docker, firewall, nginx, software) wrap their commands in sudo -S and pipe this password from the OS keychain.",
                    )}
              </span>
            </div>
            <div className="dlg-row">
              <label
                className="dlg-row-label"
                style={{ alignSelf: "center" }}
              >
                {t("Auto-elevate to root")}
              </label>
              <label
                style={{
                  display: "inline-flex",
                  alignItems: "center",
                  gap: "var(--sp-2)",
                  fontSize: "var(--ui-fs-sm)",
                }}
              >
                <input
                  type="checkbox"
                  checked={autoElevate}
                  onChange={(e) => setAutoElevate(e.currentTarget.checked)}
                />
                <span>
                  {t(
                    "Run sudo -i automatically after the SSH terminal opens",
                  )}
                </span>
              </label>
            </div>
            {autoElevate ? (
              <div className="dlg-note">
                <ShieldCheck size={11} />
                <span>
                  {hasStoredSudoPassword || sudoPassword.trim().length > 0
                    ? t(
                        "On connect, Pier-X will pipe the saved sudo password into `sudo -S -p '' -i bash` so the terminal lands directly in a root login shell.",
                      )
                    : t(
                        "Auto-elevate is enabled but no sudo password is saved for this host. Set one in the Sudo password field above first.",
                      )}
                </span>
              </div>
            ) : null}
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
          {egressPaneOpen && (
            <div className="dlg-split-pane">
              <div className="dlg-split-pane-title">
                <Shield size={12} />
                {t(egressDraft.isNew ? "New egress profile" : "Edit egress profile")}
              </div>
              <EgressProfileForm
                draft={egressDraft}
                onChange={setEgressDraft}
                connections={connections}
              />
              {egressError && (
                <div className="status-note status-note--error">{egressError}</div>
              )}
              <div className="dlg-split-pane-foot">
                <button
                  type="button"
                  className="gb-btn"
                  onClick={() => setEgressDialogOpen(true)}
                  title={t("Manage egress profiles")}
                >
                  {t("Manage…")}
                </button>
                <div style={{ flex: 1 }} />
                <button type="button" className="gb-btn" onClick={() => setEgressPaneOpen(false)}>
                  {t("Cancel")}
                </button>
                <button
                  type="button"
                  className="gb-btn primary"
                  disabled={egressSaving}
                  onClick={() => void handleEgressSave()}
                >
                  {t("Save profile")}
                </button>
              </div>
            </div>
          )}
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onClose} type="button">{t("Cancel")}</button>
          <button className="gb-btn" disabled={!canDirectConnect || saving} onClick={handleConnect} type="button">{t("Connect")}</button>
          <button className="gb-btn" disabled={!canSave || saving} onClick={() => void handleSave()} type="button">
            {t(isEditing ? "Save changes" : "Save")}
          </button>
          <button className="gb-btn primary" disabled={!canSaveAndConnect || saving} onClick={() => void handleSaveAndConnect()} type="button">
            {isEditing ? t("Save changes & Connect") : `${t("Save")} & ${t("Connect")}`}
          </button>
        </div>
      </div>
      {egressDialogOpen && (
        <Suspense fallback={null}>
          <EgressProfilesDialog open onClose={() => setEgressDialogOpen(false)} />
        </Suspense>
      )}
    </div>
  );
}
