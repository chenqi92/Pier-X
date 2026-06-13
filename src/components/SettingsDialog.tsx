import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import {
  Check,
  Copy,
  ExternalLink,
  FileText,
  GitBranch,
  Info,
  Key,
  Keyboard,
  Lock,
  Monitor,
  Moon,
  RefreshCw,
  Search,
  Server,
  Settings as SettingsIcon,
  ShieldCheck,
  Sparkles,
  Sun,
  Terminal as TerminalIcon,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { KEYBINDINGS, chordFor, chordTokens, type KeybindingScope } from "../lib/keybindings";
import type { CoreInfo } from "../lib/types";
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
import Select from "./Select";
import ComboInput from "./ComboInput";
import SudoPasswordDialog from "./SudoPasswordDialog";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import * as aiCmd from "../lib/ai";
import { AI_VENDOR_GROUP_LABELS, aiVendorById, aiVendorsByGroup } from "../lib/aiVendors";
import {
  useThemeStore,
  TERMINAL_THEMES,
  type AccentName,
  type Density,
} from "../stores/useThemeStore";
import {
  useSettingsStore,
  UI_FONT_OPTIONS,
  MONO_FONT_OPTIONS,
} from "../stores/useSettingsStore";
import type { Locale } from "../stores/useSettingsStore";
import { useConnectionStore } from "../stores/useConnectionStore";
import { useSudoStore } from "../stores/useSudoStore";
import { confirm } from "../stores/useConfirmStore";
import "../styles/settings-dialog.css";

type Props = {
  open: boolean;
  onClose: () => void;
  onCheckForUpdates?: () => void;
  /** Build / runtime info shown in the About page. */
  coreInfo?: CoreInfo | null;
  /** Page to land on when the dialog opens. Resets to the previously
   *  active page on close so the next open isn't sticky. */
  initialPage?: Page;
};

type Page =
  | "Appearance"
  | "Typography"
  | "Terminal"
  | "Editor"
  | "Keymap"
  | "Ai"
  | "Connections"
  | "Profiles"
  | "Git"
  | "SshKeys"
  | "Diagnostics"
  | "Privacy"
  | "Security"
  | "General"
  | "About";

type NavEntry = {
  key: Page;
  icon: ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;
};
type NavGroup = { label: string; items: NavEntry[] };

/** Page-key → user-facing label. Defaults to the key itself when
 *  the key is already a sensible word ("Appearance" / "Terminal"). */
const PAGE_LABEL: Record<Page, string> = {
  Appearance: "Appearance",
  Typography: "Typography",
  Terminal: "Terminal",
  Editor: "Editor",
  Keymap: "Keymap",
  Ai: "AI",
  Connections: "Connections",
  Profiles: "Profiles",
  Git: "Git",
  SshKeys: "SSH keys",
  Diagnostics: "Diagnostics",
  Privacy: "Privacy",
  Security: "Security",
  General: "General",
  About: "About",
};

const NAV_GROUPS: NavGroup[] = [
  {
    label: "General",
    items: [
      { key: "Appearance", icon: Sun },
      { key: "Typography", icon: FileText },
      { key: "Terminal", icon: TerminalIcon },
      { key: "Editor", icon: FileText },
      { key: "Keymap", icon: Keyboard },
    ],
  },
  {
    label: "Integrations",
    items: [
      { key: "Ai", icon: Sparkles },
      { key: "Connections", icon: Server },
      { key: "Profiles", icon: TerminalIcon },
      { key: "Git", icon: GitBranch },
      { key: "SshKeys", icon: Key },
    ],
  },
  {
    label: "System",
    items: [
      { key: "Diagnostics", icon: FileText },
      { key: "Privacy", icon: Lock },
      { key: "Security", icon: ShieldCheck },
      { key: "General", icon: SettingsIcon },
      { key: "About", icon: Info },
    ],
  },
];

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

/** Three-card picker for the color scheme (Dark / Light / System).
 *  Each card shows a tinted preview strip mimicking the resulting
 *  chrome — more skimmable than a plain segmented control because
 *  the user sees the outcome before committing. */
function ColorSchemeCards({
  value,
  onChange,
  t,
}: {
  value: "dark" | "light" | "system";
  onChange: (next: "dark" | "light" | "system") => void;
  t: (s: string) => string;
}) {
  const opts: Array<{
    key: "dark" | "light" | "system";
    label: string;
    Icon: ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;
  }> = [
    { key: "dark", label: t("Dark"), Icon: Moon },
    { key: "light", label: t("Light"), Icon: Sun },
    { key: "system", label: t("System"), Icon: Monitor },
  ];
  return (
    <div className="theme-cards">
      {opts.map(({ key, label, Icon }) => (
        <button
          key={key}
          type="button"
          className={"theme-card theme-card--" + key + (value === key ? " is-active" : "")}
          onClick={() => onChange(key)}
        >
          <span className="theme-card-thumb">
            <span className="theme-card-bar theme-card-bar--top" />
            <span className="theme-card-bar theme-card-bar--mid" />
            <span className="theme-card-bar theme-card-bar--bot" />
          </span>
          <span className="theme-card-label">
            <Icon size={11} /> {label}
          </span>
        </button>
      ))}
    </div>
  );
}

/** Single segmented control for the terminal bell mode. Maps to the
 *  existing two booleans (visualBell + audioBell) without a store
 *  schema change — keeps Off/Visual/Audio/Both expressible via the
 *  same primitives. */
type BellMode = "off" | "visual" | "audio" | "both";
function bellModeFrom(visual: boolean, audio: boolean): BellMode {
  if (visual && audio) return "both";
  if (visual) return "visual";
  if (audio) return "audio";
  return "off";
}
function bellModeToFlags(mode: BellMode): { visual: boolean; audio: boolean } {
  return {
    visual: mode === "visual" || mode === "both",
    audio: mode === "audio" || mode === "both",
  };
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
    if (!(await confirm({ message: t("Truncate the log file to zero bytes?"), tone: "destructive" }))) return;
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
          overscrollBehavior: "contain",
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

// ── Git panel ───────────────────────────────────────────────────
// Reads + writes the user's `~/.gitconfig` global config via the
// `git_global_config_get/set` Tauri commands. Scope is intentionally
// narrow — Identity, Signing, Hooks toggles. Anything beyond this
// (URL rewriting, diff tools, custom aliases) stays in the user's
// preferred git config editor.
//
// The page is a buffered editor: form state stays local, "Save"
// pushes the whole config in one shot. Discard/reload re-reads from
// disk so the form mirrors the actual file.
// ── Editor panel ─────────────────────────────────────────────────
// Settings → Editor. Today this drives the SFTP file viewer/editor
// dialog (`SftpEditorDialog`) — wrap default, line-numbers default,
// tab width, and on-save transforms (trim trailing / final newline).
// The dialog reads these from `useSettingsStore` directly; Settings
// just renders the form.
function EditorPanel() {
  const { t } = useI18n();
  const settings = useSettingsStore();
  return (
    <>
      <SectionTitle>{t("SFTP file editor")}</SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: "var(--sp-1)" }}>
        {t("Defaults for the in-app file viewer/editor opened from SFTP.")}
      </div>
      <SettingRow
        label={t("Wrap long lines by default")}
        description={t("Toggleable per-session in the dialog toolbar.")}
      >
        <Toggle
          checked={settings.editorWrapDefault}
          onChange={settings.setEditorWrapDefault}
        />
      </SettingRow>
      <SettingRow
        label={t("Show line numbers by default")}
        description={t("Toggleable per-session in the dialog toolbar.")}
      >
        <Toggle
          checked={settings.editorLineNumbersDefault}
          onChange={settings.setEditorLineNumbersDefault}
        />
      </SettingRow>
      <SettingRow
        label={t("Tab width")}
        description={t("Number of spaces a Tab character renders as.")}
      >
        <SegmentedControl
          options={[
            { label: "2", value: 2 },
            { label: "4", value: 4 },
            { label: "8", value: 8 },
          ]}
          value={settings.editorTabSize}
          onChange={(v) => settings.setEditorTabSize(Number(v))}
        />
      </SettingRow>

      <SectionTitle>{t("On save")}</SectionTitle>
      <SettingRow
        label={t("Trim trailing whitespace")}
        description={t("Strips spaces and tabs at the end of every line before writing.")}
      >
        <Toggle
          checked={settings.editorTrimTrailingOnSave}
          onChange={settings.setEditorTrimTrailingOnSave}
        />
      </SettingRow>
      <SettingRow
        label={t("Ensure final newline")}
        description={t("Guarantees the file ends with exactly one trailing \\n.")}
      >
        <Toggle
          checked={settings.editorEnsureFinalNewlineOnSave}
          onChange={settings.setEditorEnsureFinalNewlineOnSave}
        />
      </SettingRow>
    </>
  );
}

// ── Privacy panel ────────────────────────────────────────────────
// Settings → Privacy. Two sections:
//   1. Local storage stats — disk usage of pier-x's own logs/cache.
//      Reading the directory size is straightforward; Tauri's
//      filesystem plugin would be cleaner but adds a dependency, so
//      we leave the actual numbers blank for now and just surface
//      the paths + a "Show in folder" / "Open log" link.
//   2. Secret-scan patterns — user-defined regex patterns to flag.
//      Storage-only for now (no enforcement) — wired so the future
//      pre-commit / paste guard can read them straight away.
/**
 * Settings → Terminal → Command library.
 *
 * Renders the loaded packs (bundled + user) and exposes
 * Reload / Remove actions. Online update (Phase E) lands later
 * — for now the loader picks up files dropped into the user
 * pack dir on disk; "Reload" forces a re-scan.
 *
 * The list is read-only when bundled; user packs get a Trash
 * button per row. Source pill colors: bundled = accent, auto-
 * imported = muted, user = positive.
 */
function CommandLibraryPanel() {
  const { t } = useI18n();
  const [snapshot, setSnapshot] = useState<{
    entries: Array<{
      command: string;
      toolVersion: string;
      source: string;
      importMethod: string;
      importDate: string;
      subcommandCount: number;
      optionCount: number;
      locales: string[];
    }>;
    userDir: string;
  } | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const next = await import("../lib/terminalSmart").then((m) =>
        m.completionLibraryList(),
      );
      setSnapshot(next);
    } catch (e) {
      toast.error(`${t("Library list failed")}: ${String(e)}`);
    }
  }, [t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function reload() {
    setBusy(true);
    try {
      const m = await import("../lib/terminalSmart");
      const next = await m.completionLibraryReload();
      setSnapshot(next);
      toast.success(t("Library reloaded"));
    } catch (e) {
      toast.error(`${t("Library reload failed")}: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function remove(command: string) {
    setBusy(true);
    try {
      const m = await import("../lib/terminalSmart");
      const next = await m.completionLibraryRemovePack(command);
      setSnapshot(next);
    } catch (e) {
      toast.error(`${t("Remove failed")}: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function importPack() {
    setBusy(true);
    try {
      const dialog = await import("@tauri-apps/plugin-dialog");
      const picked = await dialog.open({
        multiple: false,
        directory: false,
        filters: [{ name: "Pier-X command pack", extensions: ["json"] }],
        title: t("Import command pack"),
      });
      if (typeof picked !== "string" || !picked) return;
      const m = await import("../lib/terminalSmart");
      const next = await m.completionLibraryInstallPackFromPath(picked);
      setSnapshot(next);
      toast.success(t("Pack imported"));
    } catch (e) {
      toast.error(`${t("Import failed")}: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  const entries = snapshot?.entries ?? [];
  const totalSubs = entries.reduce((acc, e) => acc + e.subcommandCount, 0);
  const zhCovered = entries.filter((e) =>
    e.locales.some((l) => l === "zh-CN" || l.startsWith("zh-")),
  ).length;

  return (
    <>
      <SettingRow
        label={t("Installed packs")}
        description={t(
          "Bundled + user-supplied command packs feed the Tab completion popover. Use Import to add a pack JSON, drop importer output into the directory below and click Reload, or remove user packs from the table.",
        )}
      >
        <div style={{ display: "flex", gap: "var(--sp-2)" }}>
          <button
            type="button"
            className="btn is-compact"
            disabled={busy}
            onClick={() => void importPack()}
          >
            <Upload size={10} /> {t("Import…")}
          </button>
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={busy}
            onClick={() => void reload()}
          >
            <RefreshCw size={10} /> {busy ? t("Reloading...") : t("Reload")}
          </button>
        </div>
      </SettingRow>

      <div
        className="settings__row-desc mono"
        style={{ marginBottom: "var(--sp-2)", fontSize: "var(--size-small)" }}
      >
        {t("{count} packs · {subs} subcommands · zh coverage {zh}/{total}", {
          count: entries.length,
          subs: totalSubs,
          zh: zhCovered,
          total: entries.length,
        })}
      </div>

      <div
        style={{
          border: "1px solid var(--line)",
          borderRadius: "var(--radius-sm)",
          background: "var(--surface-2)",
          overflow: "hidden",
        }}
      >
        <table className="dk-table" style={{ margin: 0, width: "100%" }}>
          <thead>
            <tr>
              <th style={{ width: 140 }}>{t("Command")}</th>
              <th style={{ width: 90 }}>{t("Version")}</th>
              <th style={{ width: 110 }}>{t("Source")}</th>
              <th style={{ width: 90, textAlign: "right" }}>{t("Subcmds")}</th>
              <th style={{ width: 70, textAlign: "right" }}>{t("Flags")}</th>
              <th>{t("Locales")}</th>
              <th style={{ width: 60 }} />
            </tr>
          </thead>
          <tbody>
            {entries.length === 0 && (
              <tr>
                <td
                  colSpan={7}
                  className="text-muted"
                  style={{ padding: "var(--sp-3)", textAlign: "center" }}
                >
                  {t("No packs loaded.")}
                </td>
              </tr>
            )}
            {entries.map((e) => {
              const isUser = e.source === "user" || e.source === "auto-imported";
              const sourceColor =
                e.source === "bundled-seed"
                  ? "var(--accent)"
                  : e.source === "user"
                    ? "var(--pos)"
                    : "var(--muted)";
              return (
                <tr key={e.command}>
                  <td className="mono">
                    <strong>{e.command}</strong>
                  </td>
                  <td className="mono text-muted">{e.toolVersion || "—"}</td>
                  <td className="mono" style={{ color: sourceColor }}>
                    {e.source}
                  </td>
                  <td className="mono" style={{ textAlign: "right" }}>
                    {e.subcommandCount}
                  </td>
                  <td className="mono" style={{ textAlign: "right" }}>
                    {e.optionCount}
                  </td>
                  <td className="mono text-muted">{e.locales.join(", ") || "—"}</td>
                  <td style={{ textAlign: "right" }}>
                    {isUser && (
                      <button
                        type="button"
                        className="mini-btn"
                        title={t("Remove pack")}
                        disabled={busy}
                        onClick={() => void remove(e.command)}
                      >
                        <Trash2 size={11} />
                      </button>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {snapshot?.userDir && (
        <div
          className="settings__row-desc mono"
          style={{ marginTop: "var(--sp-2)", fontSize: "var(--size-small)" }}
        >
          {t("User pack directory")}: {snapshot.userDir}
        </div>
      )}
    </>
  );
}

function PrivacyPanel() {
  const { t } = useI18n();
  const settings = useSettingsStore();
  const [logPath, setLogPath] = useState("");

  useEffect(() => {
    void getLogFilePath().then(setLogPath).catch(() => {});
  }, []);

  return (
    <>
      <SectionTitle>{t("Local storage")}</SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: "var(--sp-1)" }}>
        {t("Pier-X is offline-first — nothing in this section leaves your device.")}
      </div>
      <div className="privacy-storage">
        <div className="privacy-storage-row">
          <span className="privacy-storage-key">{t("Log file")}</span>
          <span className="privacy-storage-val mono">{logPath || "—"}</span>
          <span className="privacy-storage-actions">
            <button
              type="button"
              className="mini-button mini-button--ghost"
              onClick={() => {
                if (!logPath) return;
                void openPath(logPath).catch((e) => toast.error(String(e)));
              }}
              disabled={!logPath}
            >
              {t("Open")}
            </button>
            <button
              type="button"
              className="mini-button mini-button--ghost"
              onClick={() => {
                if (!logPath) return;
                void revealItemInDir(logPath).catch((e) => toast.error(String(e)));
              }}
              disabled={!logPath}
            >
              {t("Show in folder")}
            </button>
          </span>
        </div>
        <div className="settings__row-desc" style={{ padding: "var(--sp-2) var(--sp-3) 0" }}>
          {t("See the Diagnostics page for tail / clear actions on the log.")}
        </div>
      </div>

      <SectionTitle>{t("Secret-scan patterns")}</SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: "var(--sp-1)" }}>
        {t("One regex per line. Stored locally — flagging is a planned feature; nothing acts on these patterns yet.")}
      </div>
      <textarea
        className="dlg-ta mono"
        rows={6}
        value={settings.secretScanPatterns}
        onChange={(e) => settings.setSecretScanPatterns(e.currentTarget.value)}
        placeholder={"pier_api_[A-Za-z0-9]{32}\\b\nxoxb-[0-9A-Za-z-]+"}
        spellCheck={false}
      />
    </>
  );
}

// ── Security panel ───────────────────────────────────────────────
// Settings → Security. Inventories every saved SSH connection and
// shows whether a privilege-escalation password is currently armed
// for it (in-memory L1 cache OR persisted in the OS keychain).
// "Forget" purges both layers for that host. The panel does NOT
// expose passwords themselves — only their presence — so even a
// shoulder-surfing screenshot can't leak credentials.

type SecurityHostRow = {
  /** `user@host:port`, the same key `useSudoStore` indexes by. */
  storeKey: string;
  /** UI display name (saved-connection label, falls back to the
   *  raw `user@host`). */
  label: string;
  /** Saved-connection display name on its own, or `""` when the host
   *  has none — rendered as the row's primary line above the address. */
  name: string;
  /** Stable identity tuple — used to drive the keychain lookup
   *  and to call `forgetElevationPassword`. */
  user: string;
  host: string;
  port: number;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex: number | null;
  /** True when the L1 (in-memory) cache holds a password for this
   *  host this session. */
  inMemory: boolean;
  /** Set to `"yes"` / `"no"` once the keychain probe completes;
   *  `"unknown"` while the per-row probe is in flight. */
  inKeychain: "yes" | "no" | "unknown";
};

function SecurityPanel() {
  const { t } = useI18n();
  const connections = useConnectionStore((s) => s.connections);
  const refreshConnections = useConnectionStore((s) => s.refresh);
  const sudoMemory = useSudoStore((s) => s.passwords);

  const [keychainState, setKeychainState] = useState<
    Record<string, "yes" | "no" | "unknown">
  >({});
  const [busyHost, setBusyHost] = useState<string>("");
  // Inline "set sudo password" prompt for hosts that don't yet have
  // one. The user picks a row whose state is "not set", clicks Set,
  // types the password, and we persist via setElevationPassword.
  // Avoids forcing them to either edit the saved connection or wait
  // for an EACCES from a panel just to arm sudo.
  const [setPromptHost, setSetPromptHost] = useState<SecurityHostRow | null>(
    null,
  );

  // Pull the connection list once when the page opens (it's cached
  // in the store, but the user may have edited connections without
  // forcing a refresh).
  useEffect(() => {
    void refreshConnections();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Probe the keychain for each connection. Runs once on mount and
   *  whenever the connection list changes; per-row results stream
   *  in so the table renders progressively. */
  useEffect(() => {
    let cancelled = false;
    const probe = async () => {
      const next: Record<string, "yes" | "no" | "unknown"> = {};
      for (const c of connections) {
        const key = `${c.user}@${c.host}:${c.port}`;
        next[key] = "unknown";
      }
      setKeychainState(next);
      for (const c of connections) {
        if (cancelled) return;
        const key = `${c.user}@${c.host}:${c.port}`;
        try {
          const stored = await cmd.getElevationPassword(c.user, c.host, c.port);
          if (cancelled) return;
          setKeychainState((prev) => ({
            ...prev,
            [key]: stored && stored.length > 0 ? "yes" : "no",
          }));
        } catch {
          if (cancelled) return;
          setKeychainState((prev) => ({ ...prev, [key]: "no" }));
        }
      }
    };
    void probe();
    return () => {
      cancelled = true;
    };
  }, [connections]);

  const rows: SecurityHostRow[] = useMemo(() => {
    return connections.map((c) => {
      const storeKey = `${c.user}@${c.host}:${c.port}`;
      return {
        storeKey,
        label: c.name?.trim()
          ? `${c.name} — ${c.user}@${c.host}`
          : `${c.user}@${c.host}`,
        name: c.name?.trim() ?? "",
        user: c.user,
        host: c.host,
        port: c.port,
        authMode: "",
        password: "",
        keyPath: "",
        savedConnectionIndex: c.index,
        inMemory: Boolean(sudoMemory[storeKey]),
        inKeychain: keychainState[storeKey] ?? "unknown",
      };
    });
  }, [connections, sudoMemory, keychainState]);

  const armedCount = rows.filter(
    (r) => r.inMemory || r.inKeychain === "yes",
  ).length;

  async function forget(row: SecurityHostRow) {
    setBusyHost(row.storeKey);
    try {
      await useSudoStore.getState().clear(
        {
          host: row.host,
          port: row.port,
          user: row.user,
          authMode: row.authMode,
          password: row.password,
          keyPath: row.keyPath,
          savedConnectionIndex: row.savedConnectionIndex,
        },
        true,
      );
      // Refresh keychain probe just for this row so the badge flips
      // from "yes" to "no" without a full list scan.
      setKeychainState((prev) => ({ ...prev, [row.storeKey]: "no" }));
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusyHost("");
    }
  }

  async function forgetAll() {
    if (
      !(await confirm({
        message: t(
          "Forget every saved sudo password? This clears the in-memory cache and deletes keychain entries for all listed hosts.",
        ),
        tone: "destructive",
      }))
    ) {
      return;
    }
    setBusyHost("__all__");
    try {
      // Clear memory in one shot.
      useSudoStore.getState().clearAll();
      // Walk the keychain layer per-host. We deliberately only
      // delete entries whose hosts we currently see — there's no
      // enumerate API on the keyring crate, so an entry for a
      // host the user has since deleted from connections survives
      // until the user re-adds and forgets it from this page.
      for (const row of rows) {
        if (row.inKeychain === "yes") {
          try {
            await cmd.forgetElevationPassword(row.user, row.host, row.port);
          } catch (e) {
            console.warn("forget elevation password failed", e);
          }
        }
      }
      const next: Record<string, "yes" | "no" | "unknown"> = {};
      for (const row of rows) next[row.storeKey] = "no";
      setKeychainState(next);
    } finally {
      setBusyHost("");
    }
  }

  return (
    <>
      <SectionTitle>{t("Saved sudo passwords")}</SectionTitle>
      <div
        className="settings__row-desc"
        style={{ marginBottom: "var(--sp-1)" }}
      >
        {t(
          "Pier-X stores per-host elevation passwords in your OS keychain only when you opt in via the \"Remember\" checkbox in the sudo prompt. Use this page to review what's saved or forget a host's password.",
        )}
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-2)",
          marginBottom: "var(--sp-2)",
        }}
      >
        <span className="settings__row-desc">
          {t("{n} of {total} hosts armed", {
            n: armedCount,
            total: rows.length,
          })}
        </span>
        <div style={{ flex: 1 }} />
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={forgetAll}
          disabled={armedCount === 0 || busyHost === "__all__"}
        >
          {busyHost === "__all__" ? t("Forgetting…") : t("Forget all")}
        </button>
      </div>

      {rows.length === 0 ? (
        <div className="settings__row-desc">
          {t("No saved SSH connections yet — open New Connection to add one.")}
        </div>
      ) : (
        <div className="privacy-storage">
          {rows.map((row) => {
            const armed = row.inMemory || row.inKeychain === "yes";
            return (
              <div
                className="privacy-storage-row privacy-storage-row--host"
                key={row.storeKey}
              >
                <span className="privacy-storage-key">
                  {row.name ? (
                    <span className="privacy-storage-host-name">{row.name}</span>
                  ) : null}
                  <span className="privacy-storage-host-addr">
                    {`${row.user}@${row.host}`}
                  </span>
                </span>
                <span
                  className="privacy-storage-val"
                  style={{
                    display: "inline-flex",
                    gap: "var(--sp-2)",
                    alignItems: "center",
                  }}
                >
                  {row.inMemory ? (
                    <span className="badge" title={t("Cached in memory for this session.")}>
                      {t("memory")}
                    </span>
                  ) : null}
                  {row.inKeychain === "yes" ? (
                    <span
                      className="badge"
                      title={t("Persisted in the OS keychain.")}
                    >
                      {t("keychain")}
                    </span>
                  ) : null}
                  {!armed ? (
                    <span className="settings__row-desc">{t("not set")}</span>
                  ) : null}
                  {row.inKeychain === "unknown" && !row.inMemory ? (
                    <span className="settings__row-desc">{t("checking…")}</span>
                  ) : null}
                </span>
                <span className="privacy-storage-actions">
                  {!armed ? (
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => setSetPromptHost(row)}
                      disabled={busyHost === row.storeKey}
                    >
                      {t("Set")}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="mini-button mini-button--ghost"
                      onClick={() => void forget(row)}
                      disabled={busyHost === row.storeKey}
                    >
                      {busyHost === row.storeKey ? t("Forgetting…") : t("Forget")}
                    </button>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      )}
      <SudoPasswordDialog
        open={setPromptHost !== null}
        hostLabel={setPromptHost ? `${setPromptHost.user}@${setPromptHost.host}` : ""}
        onSubmit={(password, remember) => {
          const row = setPromptHost;
          setSetPromptHost(null);
          if (!row) return;
          void useSudoStore
            .getState()
            .setPersistent(
              {
                host: row.host,
                port: row.port,
                user: row.user,
                authMode: row.authMode,
                password: row.password,
                keyPath: row.keyPath,
                savedConnectionIndex: row.savedConnectionIndex,
              },
              password,
              remember,
            )
            .then(() => {
              // Optimistically reflect the new state without waiting
              // for the full keychain re-probe; the next prop drill
              // will overwrite this if reality disagrees.
              setKeychainState((prev) => ({
                ...prev,
                [row.storeKey]: remember ? "yes" : prev[row.storeKey] ?? "no",
              }));
            });
        }}
        onCancel={() => setSetPromptHost(null)}
      />
    </>
  );
}

// ── SSH keys panel ───────────────────────────────────────────────
// Settings → SSH keys. Read-only inventory of `~/.ssh/id_*` files —
// surfaces the file path, file type (from .pub first line), and
// permissions octal. Generation / agent-load require platform-
// specific work (ssh-keygen flags, ssh-add wiring) and are deferred.
function SshKeysPanel() {
  const { t } = useI18n();
  const [keys, setKeys] = useState<cmd.SshKeyInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const reload = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const list = await cmd.sshKeysList();
      setKeys(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <>
      <SectionTitle>
        {t("SSH identities")}
        <span className="settings__badge">{keys.length}</span>
      </SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: "var(--sp-2)" }}>
        {t("Read-only list of ~/.ssh/id_* files. Generation and ssh-agent integration are tracked in a follow-up.")}
      </div>
      {error && (
        <div className="status-note status-note--error" style={{ marginBottom: "var(--sp-2)" }}>
          {error}
        </div>
      )}
      {loading && keys.length === 0 ? (
        <div className="empty-note">{t("Loading...")}</div>
      ) : keys.length === 0 ? (
        <div className="empty-note">{t("No identities found in ~/.ssh.")}</div>
      ) : (
        <div className="ssh-keys-list">
          {keys.map((k) => (
            <div key={k.path} className="ssh-key-row">
              <div className="ssh-key-icon"><Key size={14} /></div>
              <div className="ssh-key-meta">
                <div className="ssh-key-path mono">{k.path}</div>
                <div className="ssh-key-sub mono">
                  <span>{k.kind || t("unknown")}</span>
                  {k.comment && (
                    <>
                      <span className="sep"> · </span>
                      <span>{k.comment}</span>
                    </>
                  )}
                  <span className="sep"> · </span>
                  <span>{k.mode || "—"}</span>
                  {k.hasPublic && (
                    <>
                      <span className="sep"> · </span>
                      <span className="ssh-key-pub">{t(".pub present")}</span>
                    </>
                  )}
                </div>
              </div>
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => void openPath(k.path).catch((e) => toast.error(String(e)))}
                title={t("Open private key in default editor")}
              >
                {t("Open")}
              </button>
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => void revealItemInDir(k.path).catch((e) => toast.error(String(e)))}
                title={t("Reveal in folder")}
              >
                {t("Show")}
              </button>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

function GitConfigPanel() {
  const { t } = useI18n();
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  const [userName, setUserName] = useState("");
  const [userEmail, setUserEmail] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("");
  const [signingMethod, setSigningMethod] = useState("");
  const [signingKey, setSigningKey] = useState("");
  const [signCommits, setSignCommits] = useState(false);
  const [signTags, setSignTags] = useState(false);

  // Snapshot of the loaded config — used by Discard to revert and to
  // compute the dirty flag without diffing every field manually.
  const [snapshot, setSnapshot] = useState<{
    userName: string;
    userEmail: string;
    defaultBranch: string;
    signingMethod: string;
    signingKey: string;
    signCommits: boolean;
    signTags: boolean;
  } | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const cfg = await cmd.gitGlobalConfigGet();
      setUserName(cfg.userName);
      setUserEmail(cfg.userEmail);
      setDefaultBranch(cfg.defaultBranch);
      setSigningMethod(cfg.signingMethod);
      setSigningKey(cfg.signingKey);
      setSignCommits(cfg.signCommits);
      setSignTags(cfg.signTags);
      setSnapshot({
        userName: cfg.userName,
        userEmail: cfg.userEmail,
        defaultBranch: cfg.defaultBranch,
        signingMethod: cfg.signingMethod,
        signingKey: cfg.signingKey,
        signCommits: cfg.signCommits,
        signTags: cfg.signTags,
      });
      setLoaded(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const dirty =
    snapshot !== null &&
    (snapshot.userName !== userName ||
      snapshot.userEmail !== userEmail ||
      snapshot.defaultBranch !== defaultBranch ||
      snapshot.signingMethod !== signingMethod ||
      snapshot.signingKey !== signingKey ||
      snapshot.signCommits !== signCommits ||
      snapshot.signTags !== signTags);

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      await cmd.gitGlobalConfigSet({
        userName,
        userEmail,
        defaultBranch,
        signingMethod,
        signingKey,
        signCommits,
        signTags,
      });
      toast.success(t("Git config saved."));
      // Reload to confirm persistence (and pick up any normalization).
      await reload();
    } catch (e) {
      setError(String(e));
      toast.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const discard = () => {
    if (!snapshot) return;
    setUserName(snapshot.userName);
    setUserEmail(snapshot.userEmail);
    setDefaultBranch(snapshot.defaultBranch);
    setSigningMethod(snapshot.signingMethod);
    setSigningKey(snapshot.signingKey);
    setSignCommits(snapshot.signCommits);
    setSignTags(snapshot.signTags);
  };

  if (!loaded && loading) {
    return <div className="empty-note">{t("Loading...")}</div>;
  }

  return (
    <>
      {error && (
        <div className="status-note status-note--error" style={{ marginBottom: "var(--sp-2)" }}>
          {error}
        </div>
      )}

      <SectionTitle>{t("Identity")}</SectionTitle>
      <div className="settings__row-desc" style={{ marginBottom: "var(--sp-1)" }}>
        {t("Stored in ~/.gitconfig under [user] and [init]. Used by every git command Pier-X runs.")}
      </div>
      <SettingRow label={t("Name")} description={t("Appears as the commit author.")}>
        <input
          className="settings__select"
          type="text"
          value={userName}
          onChange={(e) => setUserName(e.currentTarget.value)}
          placeholder="Jane Doe"
        />
      </SettingRow>
      <SettingRow label={t("Email")} description={t("Appears as the commit author email.")}>
        <input
          className="settings__select"
          type="email"
          value={userEmail}
          onChange={(e) => setUserEmail(e.currentTarget.value)}
          placeholder="jane@example.com"
        />
      </SettingRow>
      <SettingRow
        label={t("Default branch")}
        description={t("Name of the initial branch when you `git init` a new repo.")}
      >
        <input
          className="settings__select"
          type="text"
          value={defaultBranch}
          onChange={(e) => setDefaultBranch(e.currentTarget.value)}
          placeholder="main"
        />
      </SettingRow>

      <SectionTitle>{t("Signing")}</SectionTitle>
      <SettingRow
        label={t("Method")}
        description={t("Sets gpg.format. SSH signing requires git 2.34+ and an allowed-signers file.")}
      >
        <SegmentedControl
          options={[
            { label: t("Off"), value: "" },
            { label: "GPG", value: "openpgp" },
            { label: "SSH", value: "ssh" },
            { label: "X.509", value: "x509" },
          ]}
          value={signingMethod}
          onChange={(v) => setSigningMethod(String(v))}
        />
      </SettingRow>
      <SettingRow
        label={t("Signing key")}
        description={t("user.signingkey — fingerprint for GPG, public-key path for SSH.")}
      >
        <input
          className="settings__select"
          type="text"
          value={signingKey}
          onChange={(e) => setSigningKey(e.currentTarget.value)}
          placeholder={signingMethod === "ssh" ? "~/.ssh/id_ed25519.pub" : "ABCD1234EFGH5678"}
        />
      </SettingRow>
      <SettingRow
        label={t("Sign all commits by default")}
        description={t("Sets commit.gpgsign — passes -S to every commit.")}
      >
        <Toggle checked={signCommits} onChange={setSignCommits} />
      </SettingRow>
      <SettingRow
        label={t("Sign all tags by default")}
        description={t("Sets tag.gpgsign — equivalent to git tag -s.")}
      >
        <Toggle checked={signTags} onChange={setSignTags} />
      </SettingRow>

      <div className="git-config-actions">
        <button
          type="button"
          className="mini-button mini-button--ghost"
          onClick={() => void reload()}
          disabled={loading || saving}
        >
          {t("Reload")}
        </button>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          className="mini-button"
          onClick={discard}
          disabled={!dirty || saving}
        >
          {t("Discard")}
        </button>
        <button
          type="button"
          className="btn is-primary is-compact"
          onClick={() => void save()}
          disabled={!dirty || saving}
        >
          {saving ? t("Saving...") : dirty ? t("Save changes") : t("Saved")}
        </button>
      </div>
    </>
  );
}

// ── Keymap panel ────────────────────────────────────────────────
// Read-only viewer for the global / panel / editor shortcuts the
// app actually handles. Filterable by command label or chord. Source
// of truth is `src/lib/keybindings.ts` — keep that file in sync when
// adding new key handlers.
function KeymapPanel() {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const isMac = navigator.platform.toLowerCase().includes("mac");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return KEYBINDINGS;
    return KEYBINDINGS.filter((b) => {
      const label = t(b.label).toLowerCase();
      const chord = chordFor(b, isMac).toLowerCase();
      return label.includes(q) || chord.includes(q);
    });
  }, [query, isMac, t]);

  // Group by scope for the segmented sub-headers — keeps a long
  // shortcut list scannable at a glance.
  const grouped = useMemo(() => {
    const order: KeybindingScope[] = ["global", "panel", "editor", "git", "terminal"];
    const buckets = new Map<KeybindingScope, typeof KEYBINDINGS>();
    for (const b of filtered) {
      const list = buckets.get(b.scope) ?? [];
      list.push(b);
      buckets.set(b.scope, list);
    }
    return order
      .filter((scope) => buckets.has(scope))
      .map((scope) => ({ scope, items: buckets.get(scope) ?? [] }));
  }, [filtered]);

  const scopeLabel = (s: KeybindingScope): string => {
    switch (s) {
      case "global": return t("Global");
      case "panel": return t("Panels");
      case "editor": return t("Editor & dialogs");
      case "git": return t("Git");
      case "terminal": return t("Terminal");
    }
  };

  return (
    <>
      <SectionTitle>{t("Keyboard shortcuts")}</SectionTitle>
      <div className="keymap-search">
        <Search size={11} />
        <input
          type="text"
          placeholder={t("Filter {n} shortcuts…", { n: KEYBINDINGS.length })}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
          spellCheck={false}
        />
        {query && (
          <button
            type="button"
            className="mini-button mini-button--ghost"
            onClick={() => setQuery("")}
            title={t("Clear")}
          >
            <X size={10} />
          </button>
        )}
      </div>

      {grouped.length === 0 && (
        <div className="empty-note">{t("No shortcuts match.")}</div>
      )}

      {grouped.map(({ scope, items }) => (
        <Fragment key={scope}>
          <div className="keymap-scope-head">{scopeLabel(scope)}</div>
          <div className="keymap-list">
            {items.map((b) => (
              <div key={b.id} className="keymap-row">
                <span className="keymap-label">{t(b.label)}</span>
                <span className="keymap-chord">
                  {chordTokens(chordFor(b, isMac)).map((tok, i) => (
                    <kbd key={i} className="keymap-kbd">{tok}</kbd>
                  ))}
                </span>
              </div>
            ))}
          </div>
        </Fragment>
      ))}

      <div className="settings__row-desc" style={{ marginTop: "var(--sp-3)" }}>
        {t("Rebinding is not supported yet — edit src/lib/keybindings.ts to update this list when adding new handlers.")}
      </div>
    </>
  );
}

// ── About panel ─────────────────────────────────────────────────
// Replaces the old window.alert popup with a structured panel.
// Pulls version + platform + profile from CoreInfo (loaded at app
// boot in App.tsx), and exposes external links via the Tauri opener.
function AboutPanel({ coreInfo, onCheckForUpdates }: { coreInfo?: CoreInfo | null; onCheckForUpdates?: () => void }) {
  const { t } = useI18n();
  const version = coreInfo?.version ?? "—";
  const profile = coreInfo?.profile ?? "—";
  const platform = coreInfo?.platform ?? "—";

  const [components, setComponents] = useState<cmd.ComponentInfo[]>([]);
  useEffect(() => {
    let alive = true;
    void cmd.coreComponentsInfo()
      .then((rows) => {
        if (alive) setComponents(rows);
      })
      .catch(() => {
        /* table is informational; stay empty on failure */
      });
    return () => {
      alive = false;
    };
  }, []);

  const open = (url: string) => {
    void openUrl(url).catch(() => {
      /* opener failures are silent — user can copy the URL from the chip */
    });
  };

  return (
    <>
      <div className="about-card">
        <div className="about-mark">
          <img src="/pier-icon.png" alt="" width={48} height={48} draggable={false} />
        </div>
        <div className="about-meta">
          <div className="about-name">Pier-X</div>
          <div className="about-ver">
            <span className="mono">{version}</span>
            <span className="sep">·</span>
            <span className="mono">{profile}</span>
            <span className="sep">·</span>
            <span className="mono">{platform}</span>
          </div>
          <div className="about-tag">
            {t("Cross-platform terminal / Git / SSH / database management tool.")}
          </div>
        </div>
      </div>

      <SectionTitle>{t("Updates")}</SectionTitle>
      <SettingRow
        label={t("Check now")}
        description={t("One-shot HTTPS call to GitHub Releases. No telemetry, no auto-install.")}
      >
        <button
          className="mini-button"
          onClick={onCheckForUpdates}
          disabled={!onCheckForUpdates}
          type="button"
        >
          {t("Check for updates")}
        </button>
      </SettingRow>

      <SectionTitle>{t("Links")}</SectionTitle>
      <div className="about-links">
        <button
          className="btn is-ghost is-compact"
          onClick={() => open("https://github.com/chenqi92/Pier-X")}
          type="button"
        >
          <ExternalLink size={11} /> {t("GitHub")}
        </button>
        <button
          className="btn is-ghost is-compact"
          onClick={() => open("https://github.com/chenqi92/Pier-X#readme")}
          type="button"
        >
          <ExternalLink size={11} /> {t("Documentation")}
        </button>
        <button
          className="btn is-ghost is-compact"
          onClick={() => open("https://github.com/chenqi92/Pier-X/releases")}
          type="button"
        >
          <ExternalLink size={11} /> {t("Changelog")}
        </button>
        <button
          className="btn is-ghost is-compact"
          onClick={() => open("https://github.com/chenqi92/Pier-X/issues/new")}
          type="button"
        >
          <ExternalLink size={11} /> {t("Report an issue")}
        </button>
      </div>

      {components.length > 0 && (
        <>
          <SectionTitle>{t("Components")}</SectionTitle>
          <div className="about-components">
            {components.map((c) => (
              <div key={c.name} className="about-component-row">
                <span className="about-component-name">{c.name}</span>
                <span className="about-component-role">{t(c.role)}</span>
                <span className="about-component-ver mono">{c.version}</span>
              </div>
            ))}
          </div>
        </>
      )}

      <div className="about-foot">
        © 2024–2026 Pier-X · MIT licensed
      </div>
    </>
  );
}

// ── Main dialog ─────────────────────────────────────────────────

// ── AI assistant settings (PRODUCT-SPEC §5.14 / §6.3) ───────────
// Non-secret config lives in useSettingsStore (localStorage like
// every other setting); the API key goes straight to the OS keyring
// via `ai_secret_set` and is never echoed back.

function AiSettingsPanel() {
  const { t } = useI18n();
  const settings = useSettingsStore();
  const vendorId = settings.aiVendorId;
  const vendor = aiVendorById(vendorId);

  const [keyDraft, setKeyDraft] = useState("");
  const [keySaved, setKeySaved] = useState<boolean | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [fetchState, setFetchState] = useState<string | null>(null);
  const [fetching, setFetching] = useState(false);
  const [testState, setTestState] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [whitelist, setWhitelist] = useState<aiCmd.AiWhitelistEntry[]>([]);

  // Per-vendor key slot: switching vendors re-reads ITS keyring
  // entry and drops the previous vendor's fetched model list.
  useEffect(() => {
    setKeyDraft("");
    setKeySaved(null);
    setModels([]);
    setFetchState(null);
    aiCmd
      .aiSecretStatus(vendorId)
      .then(setKeySaved)
      .catch(() => setKeySaved(null));
  }, [vendorId]);

  useEffect(() => {
    aiCmd
      .aiWhitelistList()
      .then(setWhitelist)
      .catch(() => setWhitelist([]));
  }, []);

  const applyVendor = (id: string) => {
    if (id === vendorId) return;
    const preset = aiVendorById(id);
    settings.setAiVendorId(id);
    settings.setAiProviderKind(preset.kind);
    settings.setAiBaseUrl(preset.baseUrl);
    settings.setAiModel("");
  };

  const providerPayload = (): aiCmd.AiProviderSettings => ({
    kind: settings.aiProviderKind,
    baseUrl: settings.aiBaseUrl,
    model: settings.aiModel,
    maxTokens: settings.aiMaxTokens > 0 ? settings.aiMaxTokens : null,
    secretId: vendorId,
  });

  const saveKey = () => {
    void aiCmd
      .aiSecretSet(vendorId, keyDraft)
      .then(() => {
        setKeySaved(keyDraft.trim().length > 0);
        setKeyDraft("");
      })
      .catch(() => {});
  };

  // Doubles as the connection test: a successful fetch proves
  // endpoint + key in one round-trip.
  const fetchModels = () => {
    setFetching(true);
    setFetchState(null);
    aiCmd
      .aiListModels(providerPayload())
      .then((list) => {
        setModels(list);
        setFetchState(
          list.length > 0
            ? `${list.length} ${t("models available — pick one or keep typing.")}`
            : t("Endpoint returned an empty list — type the model id manually."),
        );
      })
      .catch((err) => setFetchState(String(err)))
      .finally(() => setFetching(false));
  };

  // Unlike "fetch models" (GET /models) this exercises the REAL chat
  // path — a 1-token POST to chat/completions — so it catches the
  // "model list works but chat is unreachable / model id rejected"
  // class of failure.
  const testConnection = () => {
    setTesting(true);
    setTestState(null);
    aiCmd
      .aiTestConnection(providerPayload())
      .then((msg) => setTestState(`✓ ${msg}`))
      .catch((err) => setTestState(`✗ ${String(err)}`))
      .finally(() => setTesting(false));
  };

  return (
    <>
      <SectionTitle>{t("Model provider")}</SectionTitle>
      <SettingRow
        label={t("Provider")}
        description={t("Bring your own key. Nothing leaves this machine until a model is configured.")}
      >
        <Select
          className="settings__select"
          value={vendorId}
          onChange={applyVendor}
          items={aiVendorsByGroup().map(({ group, vendors }) => ({
            group: t(AI_VENDOR_GROUP_LABELS[group]),
            options: vendors.map((v) => ({ value: v.id, label: v.label })),
          }))}
        />
      </SettingRow>
      <SettingRow
        label={t("Base URL")}
        description={t("Preset default — always editable, so a vendor moving its endpoint never blocks you.")}
      >
        <input
          className="settings__input"
          value={settings.aiBaseUrl}
          onChange={(e) => settings.setAiBaseUrl(e.currentTarget.value)}
          placeholder={vendor.baseUrl || "https://…/v1"}
          style={{ fontFamily: "var(--mono)" }}
        />
      </SettingRow>
      <SettingRow
        label={t("API key")}
        description={
          keySaved
            ? t("Stored in the OS keyring. Enter a new value to replace, or save empty to remove.")
            : vendor.needsKey
              ? t("Stored in the OS keyring (never in config files). Each vendor keeps its own slot.")
              : t("This endpoint usually needs no key — leave empty.")
        }
      >
        <div style={{ display: "flex", gap: "var(--sp-2)" }}>
          <input
            className="settings__input"
            type="password"
            value={keyDraft}
            onChange={(e) => setKeyDraft(e.currentTarget.value)}
            placeholder={keySaved ? "••••••••" : (vendor.keyHint ?? t("(not set)"))}
            autoComplete="off"
          />
          <button type="button" className="btn is-compact" onClick={saveKey}>
            {t("Save")}
          </button>
        </div>
      </SettingRow>
      <SettingRow
        label={t("Model")}
        description={
          fetchState ?? t("Fetch the endpoint's model list and pick one — or type any model id, listed or not.")
        }
      >
        <div style={{ display: "flex", gap: "var(--sp-2)" }}>
          <ComboInput
            className="settings__input"
            mono
            value={settings.aiModel}
            onChange={settings.setAiModel}
            suggestions={models}
            placeholder={vendor.modelHint ?? "model-id"}
          />
          <button type="button" className="btn is-compact" onClick={fetchModels} disabled={fetching}>
            {fetching ? t("Fetching…") : t("Fetch models")}
          </button>
        </div>
      </SettingRow>
      <SettingRow
        label={t("Max tokens per turn")}
        description={t("0 = default (4096). Caps a single model response.")}
      >
        <input
          className="settings__input"
          type="number"
          min={0}
          max={64000}
          value={settings.aiMaxTokens}
          onChange={(e) => settings.setAiMaxTokens(Number(e.currentTarget.value))}
          style={{ width: 100, minWidth: 100 }}
        />
      </SettingRow>
      <SettingRow
        label={t("Connection test")}
        description={
          testState ??
          t("Sends a 1-token chat through the configured endpoint — verifies the URL, key and model id over the same path the panel uses.")
        }
      >
        <button
          type="button"
          className="btn is-compact"
          onClick={testConnection}
          disabled={testing || !settings.aiModel.trim()}
          title={!settings.aiModel.trim() ? t("Set a model id first") : undefined}
        >
          {testing ? t("Testing…") : t("Test connection")}
        </button>
      </SettingRow>

      <SectionTitle>{t("Saved configurations")}</SectionTitle>
      <SettingRow
        label={t("Configurations")}
        description={t("Keep several vendor/model combos side by side; activate one here or switch from the AI panel's dropdown. Edits above are saved into the active configuration; switching vendor starts a new draft.")}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)", alignItems: "flex-end", minWidth: 0 }}>
          <button
            type="button"
            className="btn is-compact"
            onClick={settings.saveCurrentAsAiProfile}
            disabled={!settings.aiModel.trim()}
          >
            {t("Save current as configuration")}
          </button>
          {settings.aiProfiles.length === 0 && (
            <span style={{ color: "var(--dim)" }}>{t("(none saved yet)")}</span>
          )}
          {settings.aiProfiles.map((p) => {
            const active = p.id === settings.aiActiveProfileId;
            return (
              <div key={p.id} style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}>
                <code
                  style={{
                    fontFamily: "var(--mono)",
                    color: active ? "var(--accent)" : "var(--ink-2)",
                    fontSize: "var(--ui-fs-sm)",
                  }}
                >
                  {p.name}
                </code>
                {active ? (
                  <span style={{ color: "var(--accent)", fontSize: "var(--size-micro)" }}>{t("Active")}</span>
                ) : (
                  <button type="button" className="btn is-compact" onClick={() => settings.activateAiProfile(p.id)}>
                    {t("Activate")}
                  </button>
                )}
                <button type="button" className="btn is-compact" onClick={() => settings.deleteAiProfile(p.id)}>
                  {t("Remove")}
                </button>
              </div>
            );
          })}
        </div>
      </SettingRow>

      <SectionTitle>{t("Context & privacy")}</SectionTitle>
      <SettingRow
        label={t("Send tab context")}
        description={t("Host, cwd, OS and detected services for the active tab ride along with each message.")}
      >
        <Toggle checked={settings.aiAutoContext} onChange={settings.setAiAutoContext} />
      </SettingRow>
      <SettingRow
        label={t("Redact secrets")}
        description={t("Mask private keys, tokens and password assignments before anything is sent to the model.")}
      >
        <Toggle checked={settings.aiRedact} onChange={settings.setAiRedact} />
      </SettingRow>
      <SettingRow
        label={t("Save history to disk")}
        description={t("Keep AI conversations (and their audit trail) across restarts. Turn off for memory-only history; existing transcripts stay until you clear the conversation.")}
      >
        <Toggle checked={settings.aiPersistHistory} onChange={settings.setAiPersistHistory} />
      </SettingRow>

      <SectionTitle>{t("Execution")}</SectionTitle>
      <SettingRow
        label={t("Ask for read-only actions too")}
        description={t("By default L0 read-only commands auto-run (visible in the chat). Turn on to approve every action.")}
      >
        <Toggle checked={settings.aiAskReadOnly} onChange={settings.setAiAskReadOnly} />
      </SettingRow>
      <SettingRow
        label={t("One-click high-risk confirm")}
        description={t("L2 high-risk commands normally make you type the command's first word before Execute unlocks. Turn on to run them with a single click. L3 red-line commands stay blocked either way.")}
      >
        <Toggle checked={settings.aiSkipL2Confirm} onChange={settings.setAiSkipL2Confirm} />
      </SettingRow>
      <SettingRow
        label={t("Allow list")}
        description={t("Commands granted “Always allow”. L2 high-risk and L3 red-line actions can never be listed here.")}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-1)", minWidth: 0 }}>
          {whitelist.length === 0 && <span style={{ color: "var(--dim)" }}>{t("(empty)")}</span>}
          {whitelist.map((entry) => (
            <div
              key={`${entry.host}:${entry.prefix}`}
              style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)" }}
            >
              <code style={{ fontFamily: "var(--mono)", color: "var(--ink-2)" }}>
                {entry.prefix}
              </code>
              <span style={{ color: "var(--dim)", fontSize: "var(--size-micro)" }}>@{entry.host}</span>
              <button
                type="button"
                className="btn is-compact"
                onClick={() => {
                  void aiCmd.aiWhitelistRemove(entry.host, entry.prefix).then(() =>
                    setWhitelist((prev) =>
                      prev.filter((e) => !(e.host === entry.host && e.prefix === entry.prefix)),
                    ),
                  );
                }}
              >
                {t("Remove")}
              </button>
            </div>
          ))}
        </div>
      </SettingRow>
    </>
  );
}

export default function SettingsDialog({
  open,
  onClose,
  onCheckForUpdates,
  coreInfo,
  initialPage,
}: Props) {
  const { t } = useI18n();
  const [page, setPage] = useState<Page>("Appearance");
  const theme = useThemeStore();
  const settings = useSettingsStore();
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  // Honor `initialPage` whenever the dialog re-opens. The titlebar's
  // "About" menu item, for example, lands directly on the About pane
  // instead of the last viewed section.
  useEffect(() => {
    if (open && initialPage) setPage(initialPage);
  }, [open, initialPage]);

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
                    <span>{t(PAGE_LABEL[key])}</span>
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
                  <ColorSchemeCards
                    value={theme.mode}
                    onChange={(v) => theme.setMode(v)}
                    t={t}
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
                  description={t("Compact is the IDE default; Spacious adds extra breathing room.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Compact"), value: "compact" },
                      { label: t("Comfortable"), value: "comfortable" },
                      { label: t("Spacious"), value: "spacious" },
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
                  <Select
                    className="settings__select"
                    value={settings.uiFontFamily}
                    onChange={settings.setUiFontFamily}
                    items={UI_FONT_OPTIONS.map((f) => ({ value: f, label: f }))}
                  />
                </SettingRow>

                <SettingRow
                  label={t("Interface scale")}
                  description={t("{scale}% — scales the whole interface: text, icons, and spacing.", {
                    scale: (settings.uiScale * 100).toFixed(0),
                  })}
                >
                  <input
                    className="settings__slider"
                    type="range"
                    min={0.8}
                    max={1.5}
                    step={0.05}
                    value={settings.uiScale}
                    onChange={(e) => settings.setUiScale(Number(e.currentTarget.value))}
                  />
                </SettingRow>

                <SettingRow label={t("Code / mono font")} description={t("Used in terminal, code blocks, and tables.")}>
                  <Select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={settings.setMonoFontFamily}
                    items={MONO_FONT_OPTIONS.map((f) => ({ value: f, label: f }))}
                  />
                </SettingRow>

                <SectionTitle>{t("Preview")}</SectionTitle>
                <div className="settings__preview-card">
                  {/* No ×uiScale here: webview zoom already scales rendered px. */}
                  <p style={{ fontFamily: `"${settings.uiFontFamily}", var(--sans)`, fontSize: "13px" }}>
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
                  <Select
                    className="settings__select"
                    value={settings.monoFontFamily}
                    onChange={settings.setMonoFontFamily}
                    items={MONO_FONT_OPTIONS.map((f) => ({ value: f, label: f }))}
                  />
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
                <SettingRow
                  label={t("Bell mode")}
                  description={t("Visual flashes the terminal border; Audio plays the system bell.")}
                >
                  <SegmentedControl
                    options={[
                      { label: t("Off"), value: "off" },
                      { label: t("Visual"), value: "visual" },
                      { label: t("Audio"), value: "audio" },
                      { label: t("Both"), value: "both" },
                    ]}
                    value={bellModeFrom(settings.visualBell, settings.audioBell)}
                    onChange={(v) => {
                      const flags = bellModeToFlags(v as BellMode);
                      settings.setVisualBell(flags.visual);
                      settings.setAudioBell(flags.audio);
                    }}
                  />
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

                <SectionTitle>{t("Selection")}</SectionTitle>
                <SettingRow
                  label={t("Copy on select")}
                  description={t("Auto-copies the highlighted text to the clipboard (iTerm-style). ⌘C still works regardless.")}
                >
                  <Toggle
                    checked={settings.terminalCopyOnSelect}
                    onChange={settings.setTerminalCopyOnSelect}
                  />
                </SettingRow>

                <SectionTitle>{t("Smart Mode")}</SectionTitle>
                <SettingRow
                  label={t("Enable Smart Mode")}
                  description={t("Adds fish-style autosuggest, syntax highlighting, Tab completion popover, and man-page assistant on top of bash/zsh. Reopens new terminals only — existing tabs keep their current mode. Auto-disabled inside SSH sessions and full-screen apps like vim/htop.")}
                >
                  <Toggle
                    checked={settings.terminalSmartMode}
                    onChange={settings.setTerminalSmartMode}
                  />
                </SettingRow>
                <SettingRow
                  label={t("Persist autosuggest history")}
                  description={t("Save the autosuggest history to terminal-history-<shell>.jsonl in the Pier-X data directory so it survives app restarts. Lines that look like they hold a token / password are filtered out before disk write; the in-memory ring still has them for the current session.")}
                >
                  <Toggle
                    checked={settings.terminalHistoryPersist}
                    onChange={settings.setTerminalHistoryPersist}
                  />
                </SettingRow>
                <SettingRow
                  label={t("Follow terminal elevation")}
                  description={t("When you elevate in the terminal (sudo or su) and type the password, reuse it for the right-side panels (SFTP, DB, Docker…) so they follow your elevation without a second prompt. Kept in memory for this session only — never written to the keychain. The captured value is your own password (sudo) or root's (su); the backend tries sudo then su.")}
                >
                  <Toggle
                    checked={settings.followTerminalSudo}
                    onChange={settings.setFollowTerminalSudo}
                  />
                </SettingRow>

                <SectionTitle>{t("Command library")}</SectionTitle>
                <CommandLibraryPanel />

                <div className="settings__row-desc" style={{ marginTop: "var(--sp-3)" }}>
                  {t("Per-shell args / working dir / env vars are configured per profile in Settings → Profiles.")}
                </div>
              </div>
            )}

            {/* ── Editor (SFTP file editor) ───────────────── */}
            {page === "Editor" && (
              <div className="settings__page">
                <EditorPanel />
              </div>
            )}

            {/* ── Keymap ──────────────────────────────────── */}
            {page === "Keymap" && (
              <div className="settings__page">
                <KeymapPanel />
              </div>
            )}

            {/* ── Connections ──────────────────────────────── */}
            {page === "Connections" && (
              <div className="settings__page">
                <KnownHostsList />
              </div>
            )}

            {/* ── Profiles ────────────────────────────────── */}
            {page === "Profiles" && (
              <div className="settings__page">
                <TerminalProfilesManager />
              </div>
            )}

            {/* ── Git ─────────────────────────────────────── */}
            {page === "Git" && (
              <div className="settings__page">
                <GitConfigPanel />
              </div>
            )}

            {/* ── AI assistant ────────────────────────────── */}
            {page === "Ai" && (
              <div className="settings__page">
                <AiSettingsPanel />
              </div>
            )}

            {/* ── SSH keys ────────────────────────────────── */}
            {page === "SshKeys" && (
              <div className="settings__page">
                <SshKeysPanel />
              </div>
            )}

            {/* ── Privacy ─────────────────────────────────── */}
            {page === "Privacy" && (
              <div className="settings__page">
                <PrivacyPanel />
              </div>
            )}

            {/* ── Security ────────────────────────────────── */}
            {page === "Security" && (
              <div className="settings__page">
                <SecurityPanel />
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

            {/* ── About ──────────────────────────────────── */}
            {page === "About" && (
              <div className="settings__page">
                <AboutPanel coreInfo={coreInfo} onCheckForUpdates={onCheckForUpdates} />
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
