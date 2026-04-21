import { Key, Server, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import IconButton from "../components/IconButton";
import { useI18n } from "../i18n/useI18n";
import type { SavedSshConnection } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";

type ConnectionDraft = {
  index?: number;
  name: string;
  host: string;
  port: number;
  user: string;
  authKind: string;
  keyPath: string;
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
  };
}

export default function NewConnectionDialog({ open, onClose, onConnect, onConnectSaved, initialConnection }: Props) {
  const { t } = useI18n();
  const { save, update } = useConnectionStore();
  const isEditing = !!initialConnection;
  const initialDraft = useMemo(() => toDraft(initialConnection), [initialConnection]);
  const [name, setName] = useState(initialDraft.name);
  const [host, setHost] = useState(initialDraft.host);
  const [port, setPort] = useState(String(initialDraft.port));
  const [user, setUser] = useState(initialDraft.user);
  const [authMode, setAuthMode] = useState<"password" | "agent" | "key">(initialDraft.authKind as "password" | "agent" | "key");
  const [password, setPassword] = useState("");
  const [keyPath, setKeyPath] = useState(initialDraft.keyPath);
  const [error, setError] = useState("");

  useEffect(() => {
    const next = toDraft(initialConnection);
    setName(next.name);
    setHost(next.host);
    setPort(String(next.port));
    setUser(next.user);
    setAuthMode(next.authKind as "password" | "agent" | "key");
    setPassword("");
    setKeyPath(next.keyPath);
    setError("");
  }, [initialConnection, open]);

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
    const params = {
      name: connectionName,
      host: host.trim(),
      port: p,
      user: user.trim(),
      authKind: authMode,
      password: authMode === "password" ? password : "",
      keyPath: authMode === "key" ? keyPath.trim() : "",
    };

    if (isEditing && typeof initialDraft.index === "number") {
      await update({
        index: initialDraft.index,
        ...params,
      });
    } else {
      await save(params);
    }
  }

  async function handleSave() {
    if (!canSave) return;
    setError("");
    try {
      await persistConnection();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleSaveAndConnect() {
    if (!canSaveAndConnect) return;
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
      setError(String(e));
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
      <div className="dlg dlg--newconn" onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head">
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
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Name")}</label>
              <input className="dlg-input" onChange={(e) => setName(e.currentTarget.value)} placeholder="prod-api / staging" value={name} />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Host")}</label>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 88px", gap: "var(--sp-2)" }}>
                <input className="dlg-input" onChange={(e) => setHost(e.currentTarget.value)} placeholder="server.example.com" value={host} />
                <input className="dlg-input" onChange={(e) => setPort(e.currentTarget.value)} value={port} placeholder={t("Port")} />
              </div>
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("User")}</label>
              <input className="dlg-input" onChange={(e) => setUser(e.currentTarget.value)} placeholder="root" value={user} />
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
                  <input className="dlg-input mono" onChange={(e) => setKeyPath(e.currentTarget.value)} placeholder="~/.ssh/id_ed25519" value={keyPath} />
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
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onClose} type="button">{t("Cancel")}</button>
          <button className="gb-btn" disabled={!canDirectConnect} onClick={handleConnect} type="button">{t("Connect")}</button>
          <button className="gb-btn" disabled={!canSave} onClick={() => void handleSave()} type="button">
            {t(isEditing ? "Save changes" : "Save")}
          </button>
          <button className="gb-btn primary" disabled={!canSaveAndConnect} onClick={() => void handleSaveAndConnect()} type="button">
            {isEditing ? t("Save changes & Connect") : `${t("Save")} & ${t("Connect")}`}
          </button>
        </div>
      </div>
    </div>
  );
}
