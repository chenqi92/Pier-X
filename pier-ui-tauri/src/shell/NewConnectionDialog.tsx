import { X } from "lucide-react";
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
    <div className="palette-backdrop" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog__header">
          <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--sp-2)" }}>
            <div style={{ flex: 1 }}>
              <h2 className="dialog__title">
                {t(isEditing ? "Edit SSH connection" : "New SSH connection")}
              </h2>
              <span className="dialog__subtitle">
                {t("Saved under Servers sidebar.")}
              </span>
            </div>
            <IconButton variant="mini" onClick={onClose} title={t("Close")}>
              <X size={12} />
            </IconButton>
          </div>
        </div>
        <div className="dialog__body">
          <div className="form-stack">
            <label className="field-stack">
              <span className="field-label">{t("Name")}</span>
              <input className="field-input" onChange={(e) => setName(e.currentTarget.value)} placeholder="prod-api / staging" value={name} />
            </label>
            <div className="field-grid">
              <label className="field-stack">
                <span className="field-label">{t("Host")}</span>
                <input className="field-input" onChange={(e) => setHost(e.currentTarget.value)} placeholder="server.example.com" value={host} />
              </label>
              <label className="field-stack">
                <span className="field-label">{t("Port")}</span>
                <input className="field-input field-input--narrow" onChange={(e) => setPort(e.currentTarget.value)} value={port} />
              </label>
            </div>
            <label className="field-stack">
              <span className="field-label">{t("User")}</span>
              <input className="field-input" onChange={(e) => setUser(e.currentTarget.value)} placeholder="root" value={user} />
            </label>
            <label className="field-stack">
              <span className="field-label">{t("Auth mode")}</span>
              <select className="field-input field-select" onChange={(e) => setAuthMode(e.currentTarget.value as "password" | "agent" | "key")} value={authMode}>
                <option value="password">{t("Password")}</option>
                <option value="agent">{t("SSH Agent")}</option>
                <option value="key">{t("Key File")}</option>
              </select>
            </label>
            {authMode === "password" && (
              <label className="field-stack">
                <span className="field-label">{t("Password")}</span>
                <input className="field-input" type="password" onChange={(e) => setPassword(e.currentTarget.value)} placeholder={isEditing ? t("Leave blank to keep current password") : ""} value={password} />
              </label>
            )}
            {authMode === "key" && (
              <label className="field-stack">
                <span className="field-label">{t("Key File")}</span>
                <input className="field-input" onChange={(e) => setKeyPath(e.currentTarget.value)} placeholder="~/.ssh/id_ed25519" value={keyPath} />
              </label>
            )}
            {authMode === "agent" && (
              <div className="inline-note">{t("Agent auth uses the system SSH agent.")}</div>
            )}
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </div>
        <div className="dialog__footer">
          <button className="btn is-ghost" onClick={onClose} type="button">{t("Cancel")}</button>
          <button className="btn" disabled={!canDirectConnect} onClick={handleConnect} type="button">{t("Connect")}</button>
          <button className="btn" disabled={!canSave} onClick={() => void handleSave()} type="button">
            {t(isEditing ? "Save changes" : "Save")}
          </button>
          <button className="btn is-primary" disabled={!canSaveAndConnect} onClick={() => void handleSaveAndConnect()} type="button">
            {isEditing ? t("Save changes & Connect") : `${t("Save")} & ${t("Connect")}`}
          </button>
        </div>
      </div>
    </div>
  );
}
