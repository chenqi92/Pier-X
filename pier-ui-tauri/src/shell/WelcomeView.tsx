import {
  Command,
  FolderTree,
  GitBranch,
  Keyboard,
  Moon,
  Plus,
  PlugZap,
  Settings,
  SquareTerminal,
  Sun,
} from "lucide-react";
import { useI18n } from "../i18n/useI18n";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useThemeStore } from "../stores/useThemeStore";

type Props = {
  onOpenLocalTerminal: (path?: string) => void;
  onNewSsh: () => void;
  onConnectSaved: (index: number) => void;
  onSettings: () => void;
  onCommandPalette: () => void;
  version?: string;
  workspaceRoot?: string;
};

export default function WelcomeView({
  onOpenLocalTerminal,
  onNewSsh,
  onConnectSaved,
  onSettings,
  onCommandPalette,
  version,
  workspaceRoot,
}: Props) {
  const { t } = useI18n();
  const { resolvedDark, setMode } = useThemeStore();
  const { connections } = useConnectionStore();

  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "\u2318" : "Ctrl+";

  // Group connections by auth type for visual separation
  const agentConns = connections.filter((c) => c.authKind === "agent");
  const keyConns = connections.filter((c) => c.authKind === "key");
  const pwConns = connections.filter((c) => c.authKind === "password");

  return (
    <div className="welcome">
      <div className="welcome__container">
        {/* ── Hero ──────────────────────────────────────── */}
        <div className="welcome__hero">
          <div className="welcome__hero-left">
            <div className="welcome__icon">
              <span className="welcome__dot" />
            </div>
            <div>
              <h1 className="welcome__title">Pier-X</h1>
              <p className="welcome__subtitle">{t("Cross-platform terminal workspace")}</p>
            </div>
          </div>
          <div className="welcome__pills">
            {version && <span className="welcome__pill">v{version}</span>}
            <button className="welcome__pill welcome__pill--btn" onClick={() => setMode(resolvedDark ? "light" : "dark")} type="button">
              {resolvedDark ? <Moon size={11} /> : <Sun size={11} />}
              {resolvedDark ? t("Dark") : t("Light")}
            </button>
          </div>
        </div>

        {/* ── Quick Actions ─────────────────────────────── */}
        <section className="welcome__section">
          <h3 className="welcome__section-title">{t("Start")}</h3>
          <div className="welcome__action-grid">
            <button className="welcome__action-card" onClick={() => onOpenLocalTerminal()} type="button">
              <SquareTerminal size={20} />
              <div>
                <strong>{t("Open local terminal")}</strong>
                <span>{workspaceRoot ? workspaceRoot.split("/").pop() : t("Default shell")}</span>
              </div>
              <kbd>{mod}T</kbd>
            </button>
            <button className="welcome__action-card" onClick={onNewSsh} type="button">
              <PlugZap size={20} />
              <div>
                <strong>{t("New SSH connection")}</strong>
                <span>{t("Connect to remote server")}</span>
              </div>
              <kbd>{mod}N</kbd>
            </button>
            {workspaceRoot && (
              <button className="welcome__action-card" onClick={() => onOpenLocalTerminal(workspaceRoot)} type="button">
                <FolderTree size={20} />
                <div>
                  <strong>{t("Open workspace")}</strong>
                  <span>{workspaceRoot}</span>
                </div>
              </button>
            )}
            <button className="welcome__action-card" onClick={onCommandPalette} type="button">
              <Command size={20} />
              <div>
                <strong>{t("Command Palette")}</strong>
                <span>{t("Search actions and commands")}</span>
              </div>
              <kbd>{mod}K</kbd>
            </button>
          </div>
        </section>

        {/* ── Saved Connections ──────────────────────────── */}
        {connections.length > 0 && (
          <section className="welcome__section">
            <h3 className="welcome__section-title">
              {t("Servers")}
              <span className="welcome__count">{connections.length}</span>
            </h3>

            {agentConns.length > 0 && (
              <div className="welcome__conn-group">
                <span className="welcome__conn-group-label">{t("SSH Agent")}</span>
                <div className="welcome__conn-grid">
                  {agentConns.map((c) => (
                    <button key={c.index} className="welcome__conn-card" onClick={() => onConnectSaved(c.index)} type="button">
                      <div className="welcome__conn-indicator welcome__conn-indicator--agent" />
                      <div className="welcome__conn-body">
                        <strong>{c.name}</strong>
                        <span>{c.user}@{c.host}:{c.port}</span>
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {keyConns.length > 0 && (
              <div className="welcome__conn-group">
                <span className="welcome__conn-group-label">{t("Key File")}</span>
                <div className="welcome__conn-grid">
                  {keyConns.map((c) => (
                    <button key={c.index} className="welcome__conn-card" onClick={() => onConnectSaved(c.index)} type="button">
                      <div className="welcome__conn-indicator welcome__conn-indicator--key" />
                      <div className="welcome__conn-body">
                        <strong>{c.name}</strong>
                        <span>{c.user}@{c.host}:{c.port}</span>
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {pwConns.length > 0 && (
              <div className="welcome__conn-group">
                <span className="welcome__conn-group-label">{t("Password")}</span>
                <div className="welcome__conn-grid">
                  {pwConns.map((c) => (
                    <button key={c.index} className="welcome__conn-card" onClick={() => onConnectSaved(c.index)} type="button">
                      <div className="welcome__conn-indicator welcome__conn-indicator--password" />
                      <div className="welcome__conn-body">
                        <strong>{c.name}</strong>
                        <span>{c.user}@{c.host}:{c.port}</span>
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            <button className="welcome__add-conn" onClick={onNewSsh} type="button">
              <Plus size={14} />
              <span>{t("New SSH connection")}</span>
            </button>
          </section>
        )}

        {/* ── Empty state when no connections ───────────── */}
        {connections.length === 0 && (
          <section className="welcome__section">
            <div className="welcome__empty-servers">
              <PlugZap size={28} />
              <h3>{t("No saved servers")}</h3>
              <p>{t("Add your first SSH connection to get started with remote tools.")}</p>
              <button className="welcome__btn welcome__btn--primary" onClick={onNewSsh} type="button">
                <Plus size={14} />
                {t("New SSH connection")}
              </button>
            </div>
          </section>
        )}

        {/* ── Shortcuts ─────────────────────────────────── */}
        <section className="welcome__section">
          <h3 className="welcome__section-title">
            <Keyboard size={14} />
            {t("Keyboard Shortcuts")}
          </h3>
          <div className="welcome__shortcuts">
            <div className="welcome__shortcut"><kbd>{mod}T</kbd><span>{t("New terminal")}</span></div>
            <div className="welcome__shortcut"><kbd>{mod}N</kbd><span>{t("New SSH")}</span></div>
            <div className="welcome__shortcut"><kbd>{mod}W</kbd><span>{t("Close tab")}</span></div>
            <div className="welcome__shortcut"><kbd>{mod}K</kbd><span>{t("Command palette")}</span></div>
            <div className="welcome__shortcut"><kbd>{mod},</kbd><span>{t("Settings")}</span></div>
            <div className="welcome__shortcut"><kbd>{mod}{isMac ? "\u21e7" : "Shift+"}G</kbd><span>{t("Git panel")}</span></div>
          </div>
        </section>

        {/* ── Footer ────────────────────────────────────── */}
        <footer className="welcome__footer">
          <button className="welcome__footer-link" onClick={onSettings} type="button">
            <Settings size={12} />
            {t("Settings")}
          </button>
          <span className="welcome__footer-sep">·</span>
          <span className="welcome__footer-text">{version ? `v${version}` : ""}</span>
          <span className="welcome__footer-sep">·</span>
          <span className="welcome__footer-text">
            <GitBranch size={11} />
            {workspaceRoot?.split("/").pop() ?? "—"}
          </span>
        </footer>
      </div>
    </div>
  );
}
