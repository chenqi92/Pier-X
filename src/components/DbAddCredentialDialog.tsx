import { Database, Star, X, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import type { DbCredential, DbKind, DetectedDbInstance, TabState } from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useConnectionStore } from "../stores/useConnectionStore";

type Props = {
  open: boolean;
  onClose: () => void;
  /** Panel kind. Controls default port + which fields show. */
  kind: Extract<DbKind, "mysql" | "postgres" | "redis">;
  /** SSH profile index to attach the credential to. `null` blocks
   *  save (manual / unsaved SSH connections have nowhere to put
   *  the credential). */
  savedConnectionIndex: number | null;
  /** Optional detection row being adopted — pre-fills host/port
   *  and stamps `source: detected`. */
  adopting?: DetectedDbInstance | null;
  /** Tab whose SSH context powers `docker_inspect_db_env` when
   *  adopting a docker container. Optional — without it we just
   *  skip the env pre-fill. */
  tab?: TabState;
  /** Called after a successful save, with the new credential.
   *  Parent typically activates it in the tab immediately. */
  onSaved: (cred: DbCredential) => void;
};

const DEFAULT_PORT: Record<Props["kind"], number> = {
  mysql: 3306,
  postgres: 5432,
  redis: 6379,
};

const DEFAULT_USER: Record<Props["kind"], string> = {
  mysql: "root",
  postgres: "postgres",
  redis: "",
};

const KIND_LABEL: Record<Props["kind"], string> = {
  mysql: "MySQL",
  postgres: "PostgreSQL",
  redis: "Redis",
};

export default function DbAddCredentialDialog({
  open,
  onClose,
  kind,
  savedConnectionIndex,
  adopting,
  tab,
  onSaved,
}: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const refreshConnections = useConnectionStore((s) => s.refresh);

  const seed = useMemo(() => buildSeed(kind, adopting), [kind, adopting]);

  const [label, setLabel] = useState(seed.label);
  const [host, setHost] = useState(seed.host);
  const [port, setPort] = useState(String(seed.port));
  const [user, setUser] = useState(seed.user);
  const [password, setPassword] = useState("");
  const [database, setDatabase] = useState(seed.database);
  const [favorite, setFavorite] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  // Reseed when the dialog (re)opens with a different adopting row.
  useEffect(() => {
    if (!open) return;
    setLabel(seed.label);
    setHost(seed.host);
    setPort(String(seed.port));
    setUser(seed.user);
    setPassword("");
    setDatabase(seed.database);
    setFavorite(false);
    setError("");
  }, [open, seed]);

  // When adopting a docker container, best-effort fetch the
  // container's env vars so `MYSQL_DATABASE` / `POSTGRES_USER`
  // pre-fill the form. Failures are silent — we fall back to
  // whatever the detection row already gave us.
  useEffect(() => {
    if (!open) return;
    if (!adopting || adopting.source !== "docker" || !adopting.containerId) return;
    if (!tab) return;
    const sshTarget = effectiveSshTarget(tab);
    if (!sshTarget) return;

    let cancelled = false;
    const containerId = adopting.containerId;
    cmd
      .dockerInspectDbEnv({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        containerId,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      })
      .then((env) => {
        if (cancelled) return;
        if (kind === "mysql") {
          if (env.mysqlDatabase) setDatabase(env.mysqlDatabase);
          if (env.mysqlUser) setUser(env.mysqlUser);
        } else if (kind === "postgres") {
          if (env.postgresDb) setDatabase(env.postgresDb);
          if (env.postgresUser) setUser(env.postgresUser);
        }
      })
      .catch(() => {
        /* silent — detection row values remain */
      });
    return () => {
      cancelled = true;
    };
  }, [open, adopting, kind, tab]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const parsedPort = Number.parseInt(port, 10);
  const canSave =
    savedConnectionIndex !== null &&
    label.trim().length > 0 &&
    host.trim().length > 0 &&
    Number.isFinite(parsedPort) &&
    parsedPort > 0 &&
    !saving;

  async function handleSave() {
    if (savedConnectionIndex === null) return;
    setSaving(true);
    setError("");
    try {
      const cred = await cmd.dbCredSave(
        savedConnectionIndex,
        {
          kind,
          label: label.trim(),
          host: host.trim(),
          port: parsedPort,
          user: user.trim(),
          database: database.trim() || null,
          sqlitePath: null,
          favorite,
          detectionSignature: adopting?.signature ?? null,
        },
        password.length > 0 ? password : null,
      );
      await refreshConnections();
      onSaved(cred);
      onClose();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  }

  const Icon = kind === "redis" ? Zap : Database;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="dlg dlg--newconn"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <Icon size={13} />
            {t("Save {kind} connection", { kind: KIND_LABEL[kind] })}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>
        <div className="dlg-body dlg-body--form">
          <div className="dlg-form">
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Label")}</label>
              <input
                className="dlg-input"
                onChange={(e) => setLabel(e.currentTarget.value)}
                placeholder={t("prod-main / legacy-5.7")}
                value={label}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Host")}</label>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 88px", gap: "var(--sp-2)" }}>
                <input
                  className="dlg-input"
                  onChange={(e) => setHost(e.currentTarget.value)}
                  placeholder="127.0.0.1"
                  value={host}
                />
                <input
                  className="dlg-input"
                  onChange={(e) => setPort(e.currentTarget.value)}
                  placeholder={t("Port")}
                  value={port}
                />
              </div>
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("User")}</label>
              <input
                className="dlg-input"
                onChange={(e) => setUser(e.currentTarget.value)}
                placeholder={
                  kind === "redis"
                    ? t("ACL user (optional)")
                    : DEFAULT_USER[kind]
                }
                value={user}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Password")}</label>
              <input
                className="dlg-input"
                type="password"
                onChange={(e) => setPassword(e.currentTarget.value)}
                placeholder={kind === "redis" ? t("AUTH secret (optional)") : ""}
                value={password}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">
                {kind === "redis" ? t("DB index") : t("Database")}
              </label>
              <input
                className="dlg-input"
                onChange={(e) => setDatabase(e.currentTarget.value)}
                placeholder={kind === "redis" ? "0" : t("(optional)")}
                value={database}
              />
            </div>
            <div className="dlg-row">
              <label className="dlg-row-label">{t("Favorite")}</label>
              <button
                type="button"
                className={
                  "dlg-opt" + (favorite ? " active" : "")
                }
                onClick={() => setFavorite((v) => !v)}
                style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-1)" }}
              >
                <Star size={11} fill={favorite ? "currentColor" : "none"} />
                {favorite ? t("Seed on open") : t("Don't seed")}
              </button>
            </div>
            {savedConnectionIndex === null && (
              <div className="dlg-note">
                {t("Credentials can only be saved for a saved SSH profile. Open the connection from the sidebar first.")}
              </div>
            )}
            {error && <div className="status-note status-note--error">{error}</div>}
          </div>
        </div>
        <div className="dlg-foot">
          <div style={{ flex: 1 }} />
          <button className="gb-btn" onClick={onClose} type="button">
            {t("Cancel")}
          </button>
          <button
            className="gb-btn"
            disabled={!canSave}
            onClick={() => void handleSave()}
            type="button"
          >
            {saving ? t("Saving...") : t("Save")}
          </button>
        </div>
      </div>
    </div>
  );
}

function buildSeed(
  kind: Props["kind"],
  adopting: DetectedDbInstance | null | undefined,
) {
  if (adopting) {
    return {
      label: adopting.label || `${kind}@${adopting.port}`,
      host: adopting.host === "0.0.0.0" ? "127.0.0.1" : adopting.host,
      port: adopting.port,
      user: DEFAULT_USER[kind],
      database: "",
    };
  }
  return {
    label: "",
    host: "127.0.0.1",
    port: DEFAULT_PORT[kind],
    user: DEFAULT_USER[kind],
    database: "",
  };
}
