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
  Search,
  Server,
  Settings as SettingsIcon,
  Sun,
  Terminal as TerminalIcon,
  Trash2,
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
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
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
  | "Connections"
  | "Profiles"
  | "Git"
  | "SshKeys"
  | "Diagnostics"
  | "Privacy"
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
  Connections: "Connections",
  Profiles: "Profiles",
  Git: "Git",
  SshKeys: "SSH keys",
  Diagnostics: "Diagnostics",
  Privacy: "Privacy",
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
              onClick={() => logPath && void openPath(logPath).catch(() => {})}
              disabled={!logPath}
            >
              {t("Open")}
            </button>
            <button
              type="button"
              className="mini-button mini-button--ghost"
              onClick={() => logPath && void revealItemInDir(logPath).catch(() => {})}
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
                onClick={() => void openPath(k.path).catch(() => {})}
                title={t("Open private key in default editor")}
              >
                {t("Open")}
              </button>
              <button
                type="button"
                className="mini-button mini-button--ghost"
                onClick={() => void revealItemInDir(k.path).catch(() => {})}
                title={t("Reveal in folder")}
              >
                {t("Show")}
              </button>
            </div>
          ))}
        </div>
      )}

      <KnownHostsList />
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
