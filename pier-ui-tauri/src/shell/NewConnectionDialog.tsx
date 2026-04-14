import { useState } from "react";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";

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
};

export default function NewConnectionDialog({ open, onClose, onConnect }: Props) {
  const { t } = useI18n();
  const { save } = useConnectionStore();
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [user, setUser] = useState("");
  const [authMode, setAuthMode] = useState<"password" | "agent" | "key">("password");
  const [password, setPassword] = useState("");
  const [keyPath, setKeyPath] = useState("");
  const [error, setError] = useState("");

  if (!open) return null;

  const p = Number.parseInt(port, 10);
  const canConnect =
    host.trim() && user.trim() && Number.isFinite(p) && p > 0 &&
    (authMode === "agent" || (authMode === "password" ? password.length > 0 : keyPath.trim().length > 0));

  async function handleSaveAndConnect() {
    if (!canConnect) return;
    setError("");
    const params = {
      name: name.trim() || `${user.trim()}@${host.trim()}`,
      host: host.trim(),
      port: p,
      user: user.trim(),
      authKind: authMode,
      password: authMode === "password" ? password : "",
      keyPath: authMode === "key" ? keyPath.trim() : "",
    };
    try {
      await save(params);
      onConnect(params);
      onClose();
      // Reset form
      setName(""); setHost(""); setPort("22"); setUser("");
      setAuthMode("password"); setPassword(""); setKeyPath("");
    } catch (e) {
      setError(String(e));
    }
  }

  function handleConnect() {
    if (!canConnect) return;
    onConnect({
      name: name.trim() || `${user.trim()}@${host.trim()}`,
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
          <h2 className="dialog__title">{t("New SSH connection")}</h2>
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
                <input className="field-input" type="password" onChange={(e) => setPassword(e.currentTarget.value)} value={password} />
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
          <button className="mini-button" onClick={onClose} type="button">{t("Cancel")}</button>
          <button className="mini-button" disabled={!canConnect} onClick={handleConnect} type="button">{t("Connect")}</button>
          <button className="welcome__btn welcome__btn--primary" disabled={!canConnect} onClick={() => void handleSaveAndConnect()} type="button">{t("Save")} &amp; {t("Connect")}</button>
        </div>
      </div>
    </div>
  );
}
