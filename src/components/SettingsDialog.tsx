import { Fragment, useCallback, useEffect, useState } from "react";
import {
  Check,
  Copy,
  FileText,
  Keyboard,
  Server,
  Settings as SettingsIcon,
  Sun,
  Terminal as TerminalIcon,
  Trash2,
  X,
} from "lucide-react";
import * as cmd from "../lib/commands";
import { writeClipboardText } from "../lib/clipboard";
import { toast } from "../stores/useToastStore";
import {
  useTerminalProfilesStore,
  type TerminalProfile,
} from "../stores/useTerminalProfilesStore";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  clearLogFile,
  getLogFilePath,
  getLogVerbose,
  readLogTail,
  setLogVerbose,
} from "../lib/logger";
import type { ComponentType, SVGProps } from "react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import {
  useThemeStore,
  TERMINAL_THEMES,
  type AccentName,
  type Density,
} from "../stores/useThemeStore";
import { useConnectionStore } from "../stores/useConnectionStore";
import {
  useSettingsStore,
  UI_FONT_OPTIONS,
  MONO_FONT_OPTIONS,
} from "../stores/useSettingsStore";
import type { Locale } from "../stores/useSettingsStore";

type Props = {
  open: boolean;
  onClose: () => void;
  onCheckForUpdates?: () => void;
};

type Page = "Appearance" | "Typography" | "Terminal" | "Connections" | "Profiles" | "Diagnostics" | "General";

type NavEntry = {
  key: Page;
  icon: ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;
};
type NavGroup = { label: string; items: NavEntry[] };

const NAV_GROUPS: NavGroup[] = [
  {
    label: "General",
    items: [
      { key: "Appearance", icon: Sun },
      { key: "Typography", icon: FileText },
      { key: "Terminal", icon: TerminalIcon },
    ],
  },
  {
    label: "Integrations",
    items: [
      { key: "Connections", icon: Server },
      { key: "Profiles", icon: TerminalIcon },
    ],
  },
  {
    label: "System",
    items: [
      { key: "Diagnostics", icon: FileText },
      { key: "General", icon: Keyboard },
    ],
  },
];

function authKindLabel(authKind: string, t: (key: string) => string) {
  switch (authKind) {
    case "key":
      return t("Key file");
    case "agent":
      return t("Agent");
    default:
      return t("Password");
  }
}

// ── Reusable sub-components ─────────────────────────────────────

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="settings__section-title">{children}</div>;
}

function SettingRow({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="settings__row">
      <div className="settings__row-label">
        <span className="settings__row-name">{label}</span>
        {description && <span className="settings__row-desc">{description}</span>}
      </div>
      <div className="settings__row-control">{children}</div>
    </div>
  );
}

function SegmentedControl({
  options,
  value,
  onChange,
}: {
  options: { label: string; value: string | number }[];
  value: string | number;
  onChange: (v: string | number) => void;
}) {
  return (
    <div className="settings__segmented">
      {options.map((opt) => (
        <button
          key={String(opt.value)}
          className={value === opt.value ? "settings__seg-btn settings__seg-btn--active" : "settings__seg-btn"}
          onClick={() => onChange(opt.value)}
          type="button"
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      className={checked ? "settings__toggle settings__toggle--on" : "settings__toggle"}
      onClick={() => onChange(!checked)}
      type="button"
    >
      <span className="settings__toggle-thumb" />
    </button>
  );
}

const ACCENT_OPTIONS: { name: AccentName; label: string; cls: string }[] = [
  { name: "blue", label: "Blue", cls: "swatch-blue" },
  { name: "green", label: "Green", cls: "swatch-green" },
  { name: "amber", label: "Amber", cls: "swatch-amber" },
  { name: "violet", label: "Violet", cls: "swatch-violet" },
  { name: "coral", label: "Coral", cls: "swatch-coral" },
];

function AccentSwatches({
  value,
  onChange,
}: {
  value: AccentName;
  onChange: (accent: AccentName) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="swatches">
      {ACCENT_OPTIONS.map((opt) => (
        <button
          key={opt.name}
          type="button"
          title={t(opt.label)}
          className={`${opt.cls}${value === opt.name ? " is-active" : ""}`}
          onClick={() => onChange(opt.name)}
        />
      ))}
    </div>
  );
}

function KnownHostsList() {
  const { t } = useI18n();
  const [entries, setEntries] = useState<cmd.KnownHostEntry[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedLine, setCopiedLine] = useState<number | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await cmd.sshKnownHostsList();
      setEntries(result.entries);
      setPath(result.path);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleRemove = useCallback(
    async (line: number) => {
      try {
        await cmd.sshKnownHostsRemove(line);
        toast.success(t("Removed host key"));
        await load();
      } catch (e) {
        toast.error(String(e));
        setError(String(e));
      }
    },
    [load, t],
  );

  const handleCopyFingerprint = useCallback(async (entry: cmd.KnownHostEntry) => {
    if (!entry.fingerprint) return;
    await writeClipboardText(entry.fingerprint);
    setCopiedLine(entry.line);
    window.setTimeout(() => setCopiedLine((c) => (c === entry.line ? null : c)), 1500);
  }, []);

  return (
    <>
      <SectionTitle>
        {t("Known hosts")}
        <span className="settings__badge">{entries.length}</span>
      </SectionTitle>
      {path && (
        <div className="settings__row-desc" style={{ marginBottom: 8, fontFamily: "var(--mono)" }}>
          {path}
        </div>
      )}
      {error && <div className="empty-note" style={{ color: "var(--neg)" }}>{error}</div>}
      {!error && loading && entries.length === 0 && (
        <div className="empty-note">{t("Loading...")}</div>
      )}
      {!error && !loading && entries.length === 0 && (
        <div className="empty-note">{t("No pinned host keys yet.")}</div>
      )}
      {entries.length > 0 && (
        <div className="settings__conn-list">
          {entries.map((entry) => (
            <div key={entry.line} className="settings__conn-card">
              <div className="settings__conn-header">
                <strong style={{ fontFamily: "var(--mono)" }}>
                  {entry.hashed ? t("(hashed)") : entry.host}
                </strong>
                <span className="settings__conn-auth">{entry.keyType}</span>
              </div>
              <div className="settings__conn-meta" style={{ fontFamily: "var(--mono)" }}>
                {entry.fingerprint || t("(unparseable)")}
              </div>
              <div className="settings__conn-actions">
                <button
                  className="mini-button"
                  disabled={!entry.fingerprint}
                  onClick={() => void handleCopyFingerprint(entry)}
                  type="button"
                >
                  <Copy size={11} />
                  {copiedLine === entry.line ? t("Copied") : t("Copy fingerprint")}
                </button>
                <button
                  className="mini-button mini-button--destructive"
                  onClick={() => void handleRemove(entry.line)}
                  type="button"
                >
                  <Trash2 size={11} />
                  {t("Remove")}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

function TerminalProfilesManager() {
  const { t } = useI18n();
  const profiles = useTerminalProfilesStore((s) => s.profiles);
  const addProfile = useTerminalProfilesStore((s) => s.add);
  const updateProfile = useTerminalProfilesStore((s) => s.update);
  const removeProfile = useTerminalProfilesStore((s) => s.remove);

  // `null` = no editor open. `"new"` = add form. Any other string
  // is the id of an existing profile being edited in place.
  const [editing, setEditing] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftCwd, setDraftCwd] = useState("");
  const [draftCommand, setDraftCommand] = useState("");

  const startAdd = useCallback(() => {
    setEditing("new");
    setDraftName("");
    setDraftCwd("");
    setDraftCommand("");
  }, []);

  const startEdit = useCallback((profile: TerminalProfile) => {
    setEditing(profile.id);
    setDraftName(profile.name);
    setDraftCwd(profile.cwd ?? "");
    setDraftCommand(profile.startupCommand ?? "");
  }, []);

  const cancelEdit = useCallback(() => {
    setEditing(null);
  }, []);

  const commit = useCallback(() => {
    const name = draftName.trim();
    if (!name) return;
    const payload = {
      name,
      cwd: draftCwd.trim() || undefined,
      startupCommand: draftCommand.trim() || undefined,
    };
    if (editing === "new") {
      addProfile(payload);
      toast.success(t("Added profile: {name}", { name }));
    } else if (editing) {
      updateProfile(editing, payload);
      toast.success(t("Updated profile: {name}", { name }));
    }
    setEditing(null);
  }, [draftName, draftCwd, draftCommand, editing, addProfile, updateProfile, t]);

  const handleRemove = useCallback(
    (profile: TerminalProfile) => {
      removeProfile(profile.id);
      toast.info(t("Removed profile: {name}", { name: profile.name }));
      if (editing === profile.id) setEditing(null);
    },
    [removeProfile, editing, t],
  );

  return (
    <>
      <SectionTitle>
        {t("Terminal profiles")}
        <span className="settings__badge">{profiles.length}</span>
      </SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: 8 }}>
        {t("Presets for new local terminals: working directory plus optional startup command.")}
      </div>

      {profiles.length === 0 && editing !== "new" ? (
        <div className="empty-note">{t("No profiles yet.")}</div>
      ) : (
        <div className="settings__conn-list">
          {profiles.map((p) =>
            editing === p.id ? (
              <div key={p.id} className="settings__conn-card">
                <ProfileEditor
                  name={draftName}
                  cwd={draftCwd}
                  command={draftCommand}
                  onNameChange={setDraftName}
                  onCwdChange={setDraftCwd}
                  onCommandChange={setDraftCommand}
                  onCancel={cancelEdit}
                  onCommit={commit}
                />
              </div>
            ) : (
              <div key={p.id} className="settings__conn-card">
                <div className="settings__conn-header">
                  <strong>{p.name}</strong>
                </div>
                <div className="settings__conn-meta" style={{ fontFamily: "var(--mono)" }}>
                  {p.cwd || t("(no cwd)")}
                  {p.startupCommand ? ` && ${p.startupCommand}` : ""}
                </div>
                <div className="settings__conn-actions">
                  <button className="mini-button" onClick={() => startEdit(p)} type="button">
                    {t("Edit")}
                  </button>
                  <button
                    className="mini-button mini-button--destructive"
                    onClick={() => handleRemove(p)}
                    type="button"
                  >
                    <Trash2 size={11} />
                    {t("Delete")}
                  </button>
                </div>
              </div>
            ),
          )}
        </div>
      )}

      {editing === "new" ? (
        <div className="settings__conn-card" style={{ marginTop: 8 }}>
          <ProfileEditor
            name={draftName}
            cwd={draftCwd}
            command={draftCommand}
            onNameChange={setDraftName}
            onCwdChange={setDraftCwd}
            onCommandChange={setDraftCommand}
            onCancel={cancelEdit}
            onCommit={commit}
          />
        </div>
      ) : (
        <button
          className="mini-button"
          style={{ marginTop: 12 }}
          onClick={startAdd}
          type="button"
        >
          {t("Add profile")}
        </button>
      )}
    </>
  );
}

function ProfileEditor({
  name,
  cwd,
  command,
  onNameChange,
  onCwdChange,
  onCommandChange,
  onCancel,
  onCommit,
}: {
  name: string;
  cwd: string;
  command: string;
  onNameChange: (v: string) => void;
  onCwdChange: (v: string) => void;
  onCommandChange: (v: string) => void;
  onCancel: () => void;
  onCommit: () => void;
}) {
  const { t } = useI18n();
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
      <label className="settings__row-label" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <span className="settings__row-name">{t("Name")}</span>
        <input
          className="settings__select"
          value={name}
          onChange={(e) => onNameChange(e.currentTarget.value)}
          placeholder={t("e.g. Backend repo")}
          autoFocus
        />
      </label>
      <label className="settings__row-label" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <span className="settings__row-name">{t("Working directory")}</span>
        <input
          className="settings__select"
          value={cwd}
          onChange={(e) => onCwdChange(e.currentTarget.value)}
          placeholder="/Users/you/projects/app"
          style={{ fontFamily: "var(--mono)" }}
        />
      </label>
      <label className="settings__row-label" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <span className="settings__row-name">{t("Startup command")}</span>
        <input
          className="settings__select"
          value={command}
          onChange={(e) => onCommandChange(e.currentTarget.value)}
          placeholder="npm run dev"
          style={{ fontFamily: "var(--mono)" }}
        />
      </label>
      <div style={{ display: "flex", gap: "var(--sp-2)", justifyContent: "flex-end", marginTop: 4 }}>
        <button className="mini-button" onClick={onCancel} type="button">
          {t("Cancel")}
        </button>
        <button
          className="mini-button"
          onClick={onCommit}
          type="button"
          disabled={!name.trim()}
        >
          {t("Save")}
        </button>
      </div>
    </div>
  );
}

function DiagnosticsPanel() {
  const { t } = useI18n();
  const [logPath, setLogPath] = useState<string>("");
  const [verbose, setVerbose] = useState<boolean>(false);
  const [tail, setTail] = useState<string>("");
  const [tailLoading, setTailLoading] = useState<boolean>(false);

  const loadPath = useCallback(async () => {
    const p = await getLogFilePath();
    setLogPath(p);
  }, []);

  const loadVerbose = useCallback(async () => {
    setVerbose(await getLogVerbose());
  }, []);

  const loadTail = useCallback(async () => {
    setTailLoading(true);
    // Cap at 32 KiB for preview; the full log can still be opened
    // externally. This keeps the settings dialog responsive even
    // when the log is multi-megabyte after a long session.
    const text = await readLogTail(32 * 1024);
    setTail(text);
    setTailLoading(false);
  }, []);

  useEffect(() => {
    void loadPath();
    void loadVerbose();
    void loadTail();
  }, [loadPath, loadVerbose, loadTail]);

  const handleToggleVerbose = useCallback(
    async (next: boolean) => {
      setVerbose(next);
      await setLogVerbose(next);
      toast.info(
        next ? t("Verbose logging enabled") : t("Verbose logging disabled"),
      );
    },
    [t],
  );

  const handleCopyPath = useCallback(async () => {
    if (!logPath) return;
    await writeClipboardText(logPath);
    toast.success(t("Copied log path"));
  }, [logPath, t]);

  const handleOpen = useCallback(async () => {
    if (!logPath) return;
    try {
      await openPath(logPath);
    } catch (e) {
      toast.error(String(e));
    }
  }, [logPath]);

  const handleReveal = useCallback(async () => {
    if (!logPath) return;
    try {
      await revealItemInDir(logPath);
    } catch (e) {
      toast.error(String(e));
    }
  }, [logPath]);

  const handleClear = useCallback(async () => {
    if (!logPath) return;
    if (!window.confirm(t("Truncate the log file to zero bytes?"))) return;
    try {
      await clearLogFile();
      toast.success(t("Log cleared"));
      await loadTail();
    } catch (e) {
      toast.error(String(e));
    }
  }, [logPath, loadTail, t]);

  return (
    <>
      <SectionTitle>{t("Diagnostics")}</SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: 8 }}>
        {t("Runtime logs for Pier-X itself. Paths, panel errors, and SSH session traces land here.")}
      </div>

      <SectionTitle>{t("Log file")}</SectionTitle>
      <div className="settings__conn-card">
        <div
          className="settings__conn-meta"
          style={{ fontFamily: "var(--mono)", wordBreak: "break-all" }}
        >
          {logPath || t("(unavailable)")}
        </div>
        <div className="settings__conn-actions">
          <button className="mini-button" onClick={handleCopyPath} disabled={!logPath} type="button">
            <Copy size={11} />
            {t("Copy path")}
          </button>
          <button className="mini-button" onClick={handleOpen} disabled={!logPath} type="button">
            {t("Open")}
          </button>
          <button className="mini-button" onClick={handleReveal} disabled={!logPath} type="button">
            {t("Show in folder")}
          </button>
          <button
            className="mini-button mini-button--destructive"
            onClick={handleClear}
            disabled={!logPath}
            type="button"
          >
            <Trash2 size={11} />
            {t("Clear log")}
          </button>
        </div>
      </div>

      <SectionTitle>{t("Verbosity")}</SectionTitle>
      <SettingRow
        label={t("Verbose logging")}
        description={t("Include debug-level events. Turn off to keep the log small.")}
      >
        <Toggle checked={verbose} onChange={(v) => void handleToggleVerbose(v)} />
      </SettingRow>

      <SectionTitle>
        {t("Recent entries")}
        <button
          className="mini-button"
          style={{ marginLeft: 8 }}
          onClick={() => void loadTail()}
          disabled={tailLoading}
          type="button"
        >
          {tailLoading ? t("Loading...") : t("Refresh")}
        </button>
      </SectionTitle>
      <pre
        style={{
          maxHeight: 260,
          overflow: "auto",
          padding: "var(--sp-2) var(--sp-3)",
          background: "var(--surface)",
          border: "1px solid var(--line)",
          borderRadius: "var(--radius-sm)",
          fontFamily: "var(--mono)",
          fontSize: "var(--ui-fs-sm)",
          color: "var(--ink-2)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {tail || (tailLoading ? "" : t("(log is empty)"))}
      </pre>
    </>
  );
}

// ── Main dialog ─────────────────────────────────────────────────

export default function SettingsDialog({ open, onClose, onCheckForUpdates }: Props) {
  const { t } = useI18n();
  const [page, setPage] = useState<Page>("Appearance");
  const theme = useThemeStore();
  const settings = useSettingsStore();
  const { connections, remove } = useConnectionStore();
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  if (!open) return null;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div
        className="dlg dlg--settings"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <SettingsIcon size={13} />
            {t("Settings")}
          </span>
          <div style={{ flex: 1 }} />
          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dlg-body">
          <nav className="dlg-nav">
            {NAV_GROUPS.map((group) => (
              <Fragment key={group.label}>
                <div className="dlg-nav-group">{t(group.label)}</div>
                {group.items.map(({ key, icon: Icon }) => (
                  <button
                    key={key}
                    className={"dlg-nav-btn" + (page === key ? " active" : "")}
                    onClick={() => setPage(key)}
                    type="button"
                  >
                    <Icon size={13} />
                    <span>{t(key)}</span>
                  </button>
                ))}
              </Fragment>
            ))}
          </nav>

          <div className="dlg-pane">
            {/* ── Appearance ───────────────────────────────── */}
            {page === "Appearance" && (
              <div className="settings__page">
                <SectionTitle>{t("Theme")}</SectionTitle>
                <SettingRow
                  label={t("Color scheme")}
                  description={t("Dark is the native medium; light is a faithful mirror.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Dark"), value: "dark" },
                      { label: t("Light"), value: "light" },
                      { label: t("System"), value: "system" },
                    ]}
                    value={theme.mode}
                    onChange={(v) => theme.setMode(v as "dark" | "light" | "system")}
                  />
                </SettingRow>

                <SettingRow
                  label={t("Accent")}
                  description={t("One chromatic accent — applies everywhere.")}
                >
                  <AccentSwatches value={theme.accent} onChange={theme.setAccent} />
                </SettingRow>

                <SettingRow
                  label={t("Density")}
                  description={t("Compact is the IDE default; Comfortable adds 2–4px of air.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Compact"), value: "compact" },
                      { label: t("Comfortable"), value: "comfortable" },
                    ]}
                    value={theme.density}
                    onChange={(v) => theme.setDensity(v as Density)}
                  />
                </SettingRow>
              </div>
            )}

            {/* ── Typography ───────────────────────────────── */}
            {page === "Typography" && (
              <div className="settings__page">
                <SectionTitle>{t("Typography")}</SectionTitle>
                <SettingRow label={t("UI font")} description={t("Primary font for interface elements.")}>
                  <select
                    className="settings__select"
                    value={settings.uiFontFamily}
                    onChange={(e) => settings.setUiFontFamily(e.currentTarget.value)}
                  >
                    {UI_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                </SettingRow>

                <SettingRow
                  label={t("Interface text scale")}
                  description={t("{scale}% — affects all UI text.", {
                    scale: (settings.uiScale * 100).toFixed(0),
                  })}
                >
                  <input
                    className="settings__slider"
                    type="range"
                    min={0.9}
                    max={1.2}
                    step={0.05}
                    value={settings.uiScale}
                    onChange={(e) => settings.setUiScale(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SettingRow label={t("Code / mono font")} description={t("Used in terminal, code blocks, and tables.")}>
                  <select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={(e) => settings.setMonoFontFamily(e.currentTarget.value)}
                  >
                    {MONO_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                </SettingRow>

                <SectionTitle>{t("Preview")}</SectionTitle>
                <div className="settings__preview-card">
                  <p style={{ fontFamily: `"${settings.uiFontFamily}", var(--sans)`, fontSize: `${13 * settings.uiScale}px` }}>
                    {t("The quick brown fox jumps over the lazy dog — Bold text")}
                  </p>
                  <p
                    className="mono text-muted"
                    style={{ fontFamily: `"${settings.monoFontFamily}", var(--mono)`, fontSize: "13px" }}
                  >
                    {'const result = await query("SELECT * FROM users");'}
                  </p>
                </div>
              </div>
            )}

            {/* ── Terminal ─────────────────────────────────── */}
            {page === "Terminal" && (
              <div className="settings__page">
                <SectionTitle>{t("Terminal Theme")}</SectionTitle>
                <div className="settings__theme-grid">
                  {TERMINAL_THEMES.map((th, i) => (
                    <button
                      key={th.name}
                      className={
                        theme.terminalThemeIndex === i
                          ? "settings__theme-card settings__theme-card--selected"
                          : "settings__theme-card"
                      }
                      onClick={() => theme.setTerminalTheme(i)}
                      type="button"
                    >
                      <div className="settings__theme-preview" style={{ background: th.bg, color: th.fg }}>
                        <span style={{ color: th.ansi[2] }}>~</span>
                        <span style={{ color: th.ansi[4] }}> $ </span>
                        <span style={{ color: th.fg }}>echo </span>
                        <span style={{ color: th.ansi[3] }}>"{t("hello")}"</span>
                      </div>
                      <span className="settings__theme-name">{t(th.name)}</span>
                    </button>
                  ))}
                </div>

                <SectionTitle>{t("Font")}</SectionTitle>
                <SettingRow label={t("Font family")} description={t("Monospace font used in the terminal.")}>
                  <select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={(e) => settings.setMonoFontFamily(e.currentTarget.value)}
                  >
                    {MONO_FONT_OPTIONS.map((f) => (
                      <option key={f} value={f}>
                        {f}
                      </option>
                    ))}
                  </select>
                </SettingRow>

                <SettingRow label={t("Font size")} description={t("{size}px", { size: settings.terminalFontSize })}>
                  <input
                    className="settings__slider"
                    type="range"
                    min={9}
                    max={24}
                    step={1}
                    value={settings.terminalFontSize}
                    onChange={(e) => settings.setTerminalFontSize(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SectionTitle>{t("Cursor")}</SectionTitle>
                <SettingRow label={t("Cursor style")}>
                  <SegmentedControl
                    options={[
                      { label: t("Block"), value: 0 },
                      { label: t("Beam"), value: 1 },
                      { label: t("Underline"), value: 2 },
                    ]}
                    value={settings.cursorStyle}
                    onChange={(v) => settings.setCursorStyle(v as 0 | 1 | 2)}
                  />
                </SettingRow>
                <SettingRow label={t("Cursor blink")} description={t("Animate the cursor to attract attention.")}>
                  <Toggle checked={settings.cursorBlink} onChange={settings.setCursorBlink} />
                </SettingRow>

                <SectionTitle>{t("Scrollback")}</SectionTitle>
                <SettingRow
                  label={t("Buffer lines")}
                  description={t("{lines} lines of history kept in memory.", {
                    lines: settings.scrollbackLines.toLocaleString(),
                  })}
                >
                  <input
                    className="settings__number-input"
                    type="number"
                    min={1000}
                    max={100000}
                    step={1000}
                    value={settings.scrollbackLines}
                    onChange={(e) => settings.setScrollbackLines(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SectionTitle>{t("Bell")}</SectionTitle>
                <SettingRow label={t("Visual bell")} description={t("Flash the terminal border on bell character.")}>
                  <Toggle checked={settings.visualBell} onChange={settings.setVisualBell} />
                </SettingRow>
                <SettingRow label={t("Audio bell")} description={t("Play a system sound on bell character.")}>
                  <Toggle checked={settings.audioBell} onChange={settings.setAudioBell} />
                </SettingRow>

                <SectionTitle>{t("Display")}</SectionTitle>
                <SettingRow
                  label={t("Row separators")}
                  description={t("Draw a 1px divider between terminal rows — off by default.")}
                >
                  <Toggle
                    checked={settings.terminalRowSeparators}
                    onChange={settings.setTerminalRowSeparators}
                  />
                </SettingRow>
              </div>
            )}

            {/* ── Connections ──────────────────────────────── */}
            {page === "Connections" && (
              <div className="settings__page">
                <SectionTitle>
                  {t("Saved SSH connections")}
                  <span className="settings__badge">{connections.length}</span>
                </SectionTitle>
                {connections.length === 0 ? (
                  <div className="empty-note">
                    {t("No saved connections yet. Add one from the Servers sidebar.")}
                  </div>
                ) : (
                  <div className="settings__conn-list">
                    {connections.map((conn) => (
                      <div key={`${conn.index}-${conn.name}`} className="settings__conn-card">
                        <div className="settings__conn-header">
                          <strong>{conn.name}</strong>
                          <span className="settings__conn-auth">{authKindLabel(conn.authKind, t)}</span>
                        </div>
                        <div className="settings__conn-meta">
                          {conn.user}@{conn.host}:{conn.port}
                        </div>
                        <div className="settings__conn-actions">
                          <button
                            className="mini-button mini-button--destructive"
                            onClick={() => void remove(conn.index).catch(() => {})}
                            type="button"
                          >
                            {t("Remove")}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
                <KnownHostsList />
              </div>
            )}

            {/* ── Profiles ────────────────────────────────── */}
            {page === "Profiles" && (
              <div className="settings__page">
                <TerminalProfilesManager />
              </div>
            )}

            {/* ── Diagnostics ────────────────────────────── */}
            {page === "Diagnostics" && (
              <div className="settings__page">
                <DiagnosticsPanel />
              </div>
            )}

            {/* ── General ─────────────────────────────────── */}
            {page === "General" && (
              <div className="settings__page">
                <SectionTitle>{t("Language")}</SectionTitle>
                <SettingRow label={t("Interface language")} description={t("Changes apply immediately to all UI text.")}>
                  <SegmentedControl
                    options={[
                      { label: t("English"), value: "en" },
                      { label: t("Simplified Chinese"), value: "zh" },
                    ]}
                    value={settings.locale}
                    onChange={(v) => settings.setLocale(v as Locale)}
                  />
                </SettingRow>

                <SectionTitle>{t("Git")}</SectionTitle>
                <SettingRow
                  label={t("Sign commits")}
                  description={t("Pass -S to git commit. Key selection follows your git config (user.signingkey, gpg.format).")}
                >
                  <Toggle
                    checked={settings.gitCommitSigning}
                    onChange={settings.setGitCommitSigning}
                  />
                </SettingRow>

                <SectionTitle>{t("Updates")}</SectionTitle>
                <SettingRow
                  label={t("Check for updates on startup")}
                  description={t("Pier-X is offline by default. When on, the app makes a single HTTPS call to GitHub Releases at launch to see if a newer version exists. Never auto-downloads.")}
                >
                  <Toggle
                    checked={settings.updateCheckOnStartup}
                    onChange={settings.setUpdateCheckOnStartup}
                  />
                </SettingRow>
                {onCheckForUpdates ? (
                  <SettingRow
                    label={t("Check now")}
                    description={t("Check GitHub Releases this one time.")}
                  >
                    <button className="mini-button" onClick={onCheckForUpdates} type="button">
                      {t("Check for updates")}
                    </button>
                  </SettingRow>
                ) : null}

                <SectionTitle>{t("Developer")}</SectionTitle>
                <SettingRow
                  label={t("Performance overlay")}
                  description={t("Show FPS and memory usage in the status bar.")}
                >
                  <Toggle
                    checked={settings.performanceOverlay}
                    onChange={settings.setPerformanceOverlay}
                  />
                </SettingRow>
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="dlg-foot">
          <span className="dlg-foot-hint">
            <Check size={11} />
            {t("Changes save automatically")}
          </span>
          <div style={{ flex: 1 }} />
          <button className="gb-btn primary" onClick={onClose} type="button">
            {t("Done")}
          </button>
        </div>
      </div>
    </div>
  );
}
