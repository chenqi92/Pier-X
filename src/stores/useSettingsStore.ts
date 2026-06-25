import { create } from "zustand";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { AiProviderKind } from "../lib/ai";
import { aiVendorById } from "../lib/aiVendors";

export type Locale = "en" | "zh";

/** One saved AI configuration (§5.14.2): a vendor/model combo the
 *  user can keep alongside others and re-activate from the settings
 *  page or the AI panel's switcher. The API key is NOT part of the
 *  profile — keys live in the OS keyring per vendor id, so profiles
 *  of the same vendor share one key. */
export type AiProfile = {
  id: string;
  name: string;
  vendorId: string;
  kind: AiProviderKind;
  baseUrl: string;
  model: string;
  maxTokens: number;
  /** `cli` backends only: path to the agent CLI binary (§5.14.8). */
  cliBin?: string;
  /** `cli` backends only: "m1" model-backend or "m2a" native-agent. */
  cliMode?: string;
};

function aiProfileName(vendorId: string, model: string): string {
  return `${aiVendorById(vendorId).label} · ${model}`;
}

type SettingsState = {
  // General
  locale: Locale;
  performanceOverlay: boolean;
  // Appearance
  uiFontFamily: string;
  uiScale: number;
  monoFontFamily: string;
  // Terminal
  terminalFontSize: number;
  cursorStyle: 0 | 1 | 2; // 0=Block, 1=Beam, 2=Underline
  cursorBlink: boolean;
  scrollbackLines: number;
  visualBell: boolean;
  audioBell: boolean;
  /** Show a 1px divider between terminal rows. Default off (iTerm/VSCode style). */
  terminalRowSeparators: boolean;
  /** Auto-copy the selected text to the clipboard (iTerm-style). */
  terminalCopyOnSelect: boolean;
  /** Enable Pier-X "Smart Mode" — fish-style autosuggest, syntax
   *  highlighting, Tab completion popover, and man-page assistant on
   *  top of bash/zsh. Off by default per PRODUCT-SPEC §4.2.1; remote
   *  SSH and alt-screen apps auto-bypass even when this is on. */
  terminalSmartMode: boolean;
  /** Persist Smart-Mode autosuggest history to disk so it survives
   *  app restarts. Off by default per PRODUCT-SPEC §4.2.1 — backend
   *  filters out lines that look like credentials before writing,
   *  but the safest stance is opt-in. In-memory ring still works
   *  for the current session regardless of this setting. */
  terminalHistoryPersist: boolean;
  /** Auto-capture the password typed at a terminal `sudo`/`su` prompt and
   *  reuse it (in-memory, this session only — never the keychain) so the
   *  right-side panels follow the terminal's elevation without a second
   *  prompt. The backend tries `sudo` then `su` with the captured secret,
   *  so a `sudo` (own) or `su` (root) password both work. A wrong capture
   *  self-heals via the panels' own permission-denied re-prompt. */
  followTerminalSudo: boolean;
  // SFTP file editor
  /** Default state of the wrap toggle in the SFTP editor dialog. */
  editorWrapDefault: boolean;
  /** Default state of the line-numbers toggle. */
  editorLineNumbersDefault: boolean;
  /** Tab width (in spaces) for the SFTP editor. */
  editorTabSize: number;
  /** When saving via the SFTP editor, strip trailing whitespace
   *  from every line first. */
  editorTrimTrailingOnSave: boolean;
  /** When saving, ensure the file ends with exactly one newline. */
  editorEnsureFinalNewlineOnSave: boolean;
  // Git
  /** When true, pier-x passes `-S` to every `git commit` it runs.
   *  The actual key is picked by the user's git config
   *  (`user.signingkey`, `gpg.format`). */
  gitCommitSigning: boolean;
  // Network
  /** When true, pier-x fetches the GitHub "latest release" on app
   *  start and toasts when a newer version is out. Default OFF to
   *  preserve the "offline, local" posture from PRODUCT-SPEC §1.1.
   *  "Check for updates now" is always available regardless. */
  updateCheckOnStartup: boolean;
  // Privacy / secret scanning
  /** Custom regex patterns the user wants flagged before a commit
   *  or paste. One per line. Storage-only for now — enforcement is
   *  a future feature. */
  secretScanPatterns: string;
  // AI assistant (PRODUCT-SPEC §5.14). Non-secret config only —
  // the API key lives in the OS keyring (`pier-x.ai.<vendor-id>`).
  /** Selected vendor preset id (see `lib/aiVendors.ts`). Drives the
   *  keyring slot; `aiProviderKind` / `aiBaseUrl` hold the resolved
   *  protocol + endpoint and stay user-editable. */
  aiVendorId: string;
  aiProviderKind: AiProviderKind;
  /** Endpoint base URL. Empty = the provider kind's default. */
  aiBaseUrl: string;
  /** `cli` backends only: path to the agent CLI binary. Empty =
   *  resolve the flavor's default name on PATH (§5.14.8). */
  aiCliBin: string;
  /** `cli` backends only: "m1" model-backend (default) or "m2a"
   *  native-agent — the CLI self-governs locally (§5.14.8). */
  aiCliMode: string;
  /** Model id. Empty = AI assistant unconfigured (panel shows guide). */
  aiModel: string;
  /** Per-turn output cap. 0 = backend default (4096). */
  aiMaxTokens: number;
  /** Send tab metadata (backend/host/cwd/services) with each turn. */
  aiAutoContext: boolean;
  /** Scrub secrets from attachments + tool results before they
   *  leave the machine. Default on. */
  aiRedact: boolean;
  /** Ask even for read-only (L0) operations. */
  aiAskReadOnly: boolean;
  /** Skip the type-the-first-word gate on L2 high-risk cards, so the
   *  Execute button is clickable directly. Default off — L3 red-line
   *  commands stay blocked regardless. */
  aiSkipL2Confirm: boolean;
  /** Save AI conversation history to disk (`ai-history/`). Off =
   *  memory-only, same stance as terminalHistoryPersist. */
  aiPersistHistory: boolean;
  /** Saved configurations: several vendor/model combos stored side
   *  by side. One can be active at a time; the AI panel switches
   *  between them. */
  aiProfiles: AiProfile[];
  /** Profile currently loaded into the working fields above.
   *  `null` = unsaved draft (e.g. right after switching vendor). */
  aiActiveProfileId: string | null;
  // Setters
  setLocale: (locale: Locale) => void;
  setPerformanceOverlay: (on: boolean) => void;
  setUiFontFamily: (font: string) => void;
  setUiScale: (scale: number) => void;
  setMonoFontFamily: (font: string) => void;
  setTerminalFontSize: (size: number) => void;
  setCursorStyle: (style: 0 | 1 | 2) => void;
  setCursorBlink: (blink: boolean) => void;
  setScrollbackLines: (lines: number) => void;
  setVisualBell: (on: boolean) => void;
  setAudioBell: (on: boolean) => void;
  setTerminalRowSeparators: (on: boolean) => void;
  setTerminalCopyOnSelect: (on: boolean) => void;
  setTerminalSmartMode: (on: boolean) => void;
  setTerminalHistoryPersist: (on: boolean) => void;
  setFollowTerminalSudo: (on: boolean) => void;
  setEditorWrapDefault: (on: boolean) => void;
  setEditorLineNumbersDefault: (on: boolean) => void;
  setEditorTabSize: (n: number) => void;
  setEditorTrimTrailingOnSave: (on: boolean) => void;
  setEditorEnsureFinalNewlineOnSave: (on: boolean) => void;
  setGitCommitSigning: (on: boolean) => void;
  setUpdateCheckOnStartup: (on: boolean) => void;
  setSecretScanPatterns: (patterns: string) => void;
  setAiVendorId: (id: string) => void;
  setAiProviderKind: (kind: AiProviderKind) => void;
  setAiBaseUrl: (url: string) => void;
  setAiCliBin: (path: string) => void;
  setAiCliMode: (mode: string) => void;
  setAiModel: (model: string) => void;
  setAiMaxTokens: (n: number) => void;
  setAiAutoContext: (on: boolean) => void;
  setAiRedact: (on: boolean) => void;
  setAiAskReadOnly: (on: boolean) => void;
  setAiSkipL2Confirm: (on: boolean) => void;
  setAiPersistHistory: (on: boolean) => void;
  /** Snapshot the current working fields as a profile (dedupes on
   *  vendor+baseUrl+model) and mark it active. */
  saveCurrentAsAiProfile: () => void;
  /** Load a profile into the working fields and mark it active. */
  activateAiProfile: (id: string) => void;
  deleteAiProfile: (id: string) => void;
};

export const UI_FONT_OPTIONS = [
  "IBM Plex Sans",
  "Inter",
  "SF Pro Text",
  "Segoe UI",
  "Noto Sans",
  "system-ui",
];

export const MONO_FONT_OPTIONS = [
  "IBM Plex Mono",
  "JetBrains Mono",
  "SF Mono",
  "Cascadia Code",
  "Fira Code",
  "Consolas",
  "monospace",
];

const PREFS_KEY = "pierx:settings";

type PersistedSettings = Partial<{
  locale: Locale;
  performanceOverlay: boolean;
  uiFontFamily: string;
  uiScale: number;
  monoFontFamily: string;
  terminalFontSize: number;
  cursorStyle: 0 | 1 | 2;
  cursorBlink: boolean;
  scrollbackLines: number;
  visualBell: boolean;
  audioBell: boolean;
  terminalRowSeparators: boolean;
  terminalCopyOnSelect: boolean;
  terminalSmartMode: boolean;
  terminalHistoryPersist: boolean;
  followTerminalSudo: boolean;
  editorWrapDefault: boolean;
  editorLineNumbersDefault: boolean;
  editorTabSize: number;
  editorTrimTrailingOnSave: boolean;
  editorEnsureFinalNewlineOnSave: boolean;
  gitCommitSigning: boolean;
  updateCheckOnStartup: boolean;
  secretScanPatterns: string;
  aiVendorId: string;
  aiProviderKind: AiProviderKind;
  aiBaseUrl: string;
  aiCliBin: string;
  aiCliMode: string;
  aiModel: string;
  aiMaxTokens: number;
  aiAutoContext: boolean;
  aiRedact: boolean;
  aiAskReadOnly: boolean;
  aiSkipL2Confirm: boolean;
  aiPersistHistory: boolean;
  aiProfiles: AiProfile[];
  aiActiveProfileId: string | null;
}>;

const DEFAULTS = {
  locale: "zh" as Locale,
  performanceOverlay: false,
  uiFontFamily: "IBM Plex Sans",
  uiScale: 1.0,
  monoFontFamily: "IBM Plex Mono",
  terminalFontSize: 13,
  cursorStyle: 0 as 0 | 1 | 2,
  cursorBlink: true,
  scrollbackLines: 10000,
  visualBell: true,
  audioBell: false,
  terminalRowSeparators: false,
  terminalCopyOnSelect: false,
  // Off by default per PRODUCT-SPEC §4.2.1: Smart Mode is opt-in and
  // the terminal history ring is memory-only unless the user opts in
  // to persistence. Shipping these on wrote command history (incl.
  // secrets typed at password prompts) to disk without consent.
  terminalSmartMode: false,
  terminalHistoryPersist: false,
  // On by default: the captured value (sudo = own password, su = root
  // password) is kept in memory for the session and never persisted —
  // the friendly "panels follow the terminal" behavior the user expects.
  followTerminalSudo: true,
  editorWrapDefault: false,
  editorLineNumbersDefault: true,
  editorTabSize: 2,
  editorTrimTrailingOnSave: false,
  editorEnsureFinalNewlineOnSave: false,
  gitCommitSigning: false,
  updateCheckOnStartup: false,
  secretScanPatterns: "",
  // AI is opt-in (PRODUCT-SPEC §1.1 / §5.14): unconfigured by
  // default — the panel shows a guide and makes zero requests.
  aiVendorId: "openai",
  aiProviderKind: "openai" as AiProviderKind,
  aiBaseUrl: "",
  aiCliBin: "",
  aiCliMode: "m1",
  aiModel: "",
  aiMaxTokens: 0,
  aiAutoContext: true,
  aiRedact: true,
  aiAskReadOnly: false,
  aiSkipL2Confirm: false,
  aiPersistHistory: true,
  aiProfiles: [] as AiProfile[],
  aiActiveProfileId: null as string | null,
};

function loadPrefs(): PersistedSettings {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as PersistedSettings;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function savePrefs(next: PersistedSettings) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(next));
  } catch {
    /* swallow quota errors */
  }
}

function applyUiFont(family: string) {
  document.documentElement.style.setProperty(
    "--sans",
    `"${family}", system-ui, -apple-system, "SF Pro Text", "Segoe UI", sans-serif`,
  );
  document.documentElement.style.setProperty("--font-ui", `var(--sans)`);
}

function applyMonoFont(family: string) {
  document.documentElement.style.setProperty(
    "--mono",
    `"${family}", ui-monospace, "SF Mono", Consolas, monospace`,
  );
  document.documentElement.style.setProperty("--font-mono", `var(--mono)`);
}

function applyUiScale(scale: number) {
  // Native webview zoom scales the ENTIRE interface — fonts, lucide
  // icons (px-sized via props), spacing tokens, hardcoded px — and
  // stays crisp (DPR-aware re-render, not bitmap scaling). The old
  // `--ui-scale` CSS var only multiplied font-size tokens, which is
  // why "scale" used to grow text but not icons or padding.
  const fallback = () => {
    // Permission/platform fallback: type-only scaling via the var
    // (pre-zoom behavior).
    document.documentElement.style.setProperty("--ui-scale", String(scale));
  };
  try {
    getCurrentWebview()
      .setZoom(scale)
      .then(() => {
        // Zoom owns the scaling — pin the var so font tokens don't
        // double-apply on top of it.
        document.documentElement.style.setProperty("--ui-scale", "1");
      })
      .catch(fallback);
  } catch {
    // Not running inside a Tauri webview (plain-browser vite dev).
    fallback();
  }
}

export const useSettingsStore = create<SettingsState>((set, get) => {
  const stored = loadPrefs();

  const initial = {
    locale: stored.locale ?? DEFAULTS.locale,
    performanceOverlay: stored.performanceOverlay ?? DEFAULTS.performanceOverlay,
    uiFontFamily: stored.uiFontFamily ?? DEFAULTS.uiFontFamily,
    uiScale: stored.uiScale ?? DEFAULTS.uiScale,
    monoFontFamily: stored.monoFontFamily ?? DEFAULTS.monoFontFamily,
    terminalFontSize: stored.terminalFontSize ?? DEFAULTS.terminalFontSize,
    cursorStyle: stored.cursorStyle ?? DEFAULTS.cursorStyle,
    cursorBlink: stored.cursorBlink ?? DEFAULTS.cursorBlink,
    scrollbackLines: stored.scrollbackLines ?? DEFAULTS.scrollbackLines,
    visualBell: stored.visualBell ?? DEFAULTS.visualBell,
    audioBell: stored.audioBell ?? DEFAULTS.audioBell,
    terminalRowSeparators:
      stored.terminalRowSeparators ?? DEFAULTS.terminalRowSeparators,
    terminalCopyOnSelect:
      stored.terminalCopyOnSelect ?? DEFAULTS.terminalCopyOnSelect,
    terminalSmartMode:
      stored.terminalSmartMode ?? DEFAULTS.terminalSmartMode,
    terminalHistoryPersist:
      stored.terminalHistoryPersist ?? DEFAULTS.terminalHistoryPersist,
    followTerminalSudo:
      stored.followTerminalSudo ?? DEFAULTS.followTerminalSudo,
    editorWrapDefault: stored.editorWrapDefault ?? DEFAULTS.editorWrapDefault,
    editorLineNumbersDefault:
      stored.editorLineNumbersDefault ?? DEFAULTS.editorLineNumbersDefault,
    editorTabSize: stored.editorTabSize ?? DEFAULTS.editorTabSize,
    editorTrimTrailingOnSave:
      stored.editorTrimTrailingOnSave ?? DEFAULTS.editorTrimTrailingOnSave,
    editorEnsureFinalNewlineOnSave:
      stored.editorEnsureFinalNewlineOnSave ?? DEFAULTS.editorEnsureFinalNewlineOnSave,
    gitCommitSigning: stored.gitCommitSigning ?? DEFAULTS.gitCommitSigning,
    updateCheckOnStartup: stored.updateCheckOnStartup ?? DEFAULTS.updateCheckOnStartup,
    secretScanPatterns:
      stored.secretScanPatterns ?? DEFAULTS.secretScanPatterns,
    // Pre-vendor-registry configs only stored the protocol kind; the
    // three original kinds double as vendor ids, so falling back to
    // `aiProviderKind` keeps their keyring slots working unchanged.
    aiVendorId: stored.aiVendorId ?? stored.aiProviderKind ?? DEFAULTS.aiVendorId,
    aiProviderKind: stored.aiProviderKind ?? DEFAULTS.aiProviderKind,
    aiBaseUrl: stored.aiBaseUrl ?? DEFAULTS.aiBaseUrl,
    aiCliBin: stored.aiCliBin ?? DEFAULTS.aiCliBin,
    aiCliMode: stored.aiCliMode ?? DEFAULTS.aiCliMode,
    aiModel: stored.aiModel ?? DEFAULTS.aiModel,
    aiMaxTokens: stored.aiMaxTokens ?? DEFAULTS.aiMaxTokens,
    aiAutoContext: stored.aiAutoContext ?? DEFAULTS.aiAutoContext,
    aiRedact: stored.aiRedact ?? DEFAULTS.aiRedact,
    aiAskReadOnly: stored.aiAskReadOnly ?? DEFAULTS.aiAskReadOnly,
    aiSkipL2Confirm: stored.aiSkipL2Confirm ?? DEFAULTS.aiSkipL2Confirm,
    aiPersistHistory: stored.aiPersistHistory ?? DEFAULTS.aiPersistHistory,
    aiProfiles: stored.aiProfiles ?? DEFAULTS.aiProfiles,
    aiActiveProfileId: stored.aiActiveProfileId ?? DEFAULTS.aiActiveProfileId,
  };

  // Migrate a pre-profiles single config into the first profile so
  // existing setups appear in the new switcher unchanged.
  if (initial.aiProfiles.length === 0 && initial.aiModel.trim()) {
    const migrated: AiProfile = {
      id: crypto.randomUUID(),
      name: aiProfileName(initial.aiVendorId, initial.aiModel),
      vendorId: initial.aiVendorId,
      kind: initial.aiProviderKind,
      baseUrl: initial.aiBaseUrl,
      model: initial.aiModel,
      maxTokens: initial.aiMaxTokens,
      cliBin: initial.aiCliBin,
      cliMode: initial.aiCliMode,
    };
    initial.aiProfiles = [migrated];
    initial.aiActiveProfileId = migrated.id;
  }

  applyUiFont(initial.uiFontFamily);
  applyMonoFont(initial.monoFontFamily);
  applyUiScale(initial.uiScale);

  const persist = () => {
    const s = get();
    savePrefs({
      locale: s.locale,
      performanceOverlay: s.performanceOverlay,
      uiFontFamily: s.uiFontFamily,
      uiScale: s.uiScale,
      monoFontFamily: s.monoFontFamily,
      terminalFontSize: s.terminalFontSize,
      cursorStyle: s.cursorStyle,
      cursorBlink: s.cursorBlink,
      scrollbackLines: s.scrollbackLines,
      visualBell: s.visualBell,
      audioBell: s.audioBell,
      terminalRowSeparators: s.terminalRowSeparators,
      terminalCopyOnSelect: s.terminalCopyOnSelect,
      terminalSmartMode: s.terminalSmartMode,
      terminalHistoryPersist: s.terminalHistoryPersist,
      followTerminalSudo: s.followTerminalSudo,
      editorWrapDefault: s.editorWrapDefault,
      editorLineNumbersDefault: s.editorLineNumbersDefault,
      editorTabSize: s.editorTabSize,
      editorTrimTrailingOnSave: s.editorTrimTrailingOnSave,
      editorEnsureFinalNewlineOnSave: s.editorEnsureFinalNewlineOnSave,
      gitCommitSigning: s.gitCommitSigning,
      updateCheckOnStartup: s.updateCheckOnStartup,
      secretScanPatterns: s.secretScanPatterns,
      aiVendorId: s.aiVendorId,
      aiProviderKind: s.aiProviderKind,
      aiBaseUrl: s.aiBaseUrl,
      aiCliBin: s.aiCliBin,
      aiCliMode: s.aiCliMode,
      aiModel: s.aiModel,
      aiMaxTokens: s.aiMaxTokens,
      aiAutoContext: s.aiAutoContext,
      aiRedact: s.aiRedact,
      aiAskReadOnly: s.aiAskReadOnly,
      aiSkipL2Confirm: s.aiSkipL2Confirm,
      aiPersistHistory: s.aiPersistHistory,
      aiProfiles: s.aiProfiles,
      aiActiveProfileId: s.aiActiveProfileId,
    });
  };

  /** Mirror edits of the working fields into the active profile so
   *  "edit settings" and "edit the active profile" stay one action.
   *  Renames the profile when the model changes. */
  const syncActiveAiProfile = (
    patch: Partial<Pick<AiProfile, "baseUrl" | "model" | "maxTokens" | "cliBin" | "cliMode">>,
  ) => {
    const s = get();
    if (!s.aiActiveProfileId) return;
    set({
      aiProfiles: s.aiProfiles.map((p) => {
        if (p.id !== s.aiActiveProfileId) return p;
        const next = { ...p, ...patch };
        if (patch.model !== undefined) {
          next.name = aiProfileName(p.vendorId, patch.model);
        }
        return next;
      }),
    });
  };

  return {
    ...initial,
    setLocale: (locale) => {
      set({ locale });
      persist();
    },
    setPerformanceOverlay: (performanceOverlay) => {
      set({ performanceOverlay });
      persist();
    },
    setUiFontFamily: (uiFontFamily) => {
      applyUiFont(uiFontFamily);
      set({ uiFontFamily });
      persist();
    },
    setUiScale: (uiScale) => {
      applyUiScale(uiScale);
      set({ uiScale });
      persist();
    },
    setMonoFontFamily: (monoFontFamily) => {
      applyMonoFont(monoFontFamily);
      set({ monoFontFamily });
      persist();
    },
    setTerminalFontSize: (terminalFontSize) => {
      set({ terminalFontSize });
      persist();
    },
    setCursorStyle: (cursorStyle) => {
      set({ cursorStyle });
      persist();
    },
    setCursorBlink: (cursorBlink) => {
      set({ cursorBlink });
      persist();
    },
    setScrollbackLines: (scrollbackLines) => {
      set({ scrollbackLines });
      persist();
    },
    setVisualBell: (visualBell) => {
      set({ visualBell });
      persist();
    },
    setAudioBell: (audioBell) => {
      set({ audioBell });
      persist();
    },
    setTerminalRowSeparators: (terminalRowSeparators) => {
      set({ terminalRowSeparators });
      persist();
    },
    setTerminalCopyOnSelect: (terminalCopyOnSelect) => {
      set({ terminalCopyOnSelect });
      persist();
    },
    setTerminalSmartMode: (terminalSmartMode) => {
      set({ terminalSmartMode });
      persist();
    },
    setFollowTerminalSudo: (followTerminalSudo) => {
      set({ followTerminalSudo });
      persist();
    },
    setTerminalHistoryPersist: (terminalHistoryPersist) => {
      set({ terminalHistoryPersist });
      persist();
    },
    setEditorWrapDefault: (editorWrapDefault) => {
      set({ editorWrapDefault });
      persist();
    },
    setEditorLineNumbersDefault: (editorLineNumbersDefault) => {
      set({ editorLineNumbersDefault });
      persist();
    },
    setEditorTabSize: (editorTabSize) => {
      set({ editorTabSize: Math.max(1, Math.min(8, Math.round(editorTabSize))) });
      persist();
    },
    setEditorTrimTrailingOnSave: (editorTrimTrailingOnSave) => {
      set({ editorTrimTrailingOnSave });
      persist();
    },
    setEditorEnsureFinalNewlineOnSave: (editorEnsureFinalNewlineOnSave) => {
      set({ editorEnsureFinalNewlineOnSave });
      persist();
    },
    setGitCommitSigning: (gitCommitSigning) => {
      set({ gitCommitSigning });
      persist();
    },
    setUpdateCheckOnStartup: (updateCheckOnStartup) => {
      set({ updateCheckOnStartup });
      persist();
    },
    setSecretScanPatterns: (secretScanPatterns) => {
      set({ secretScanPatterns });
      persist();
    },
    setAiVendorId: (aiVendorId) => {
      // Switching vendor starts a fresh draft: detach from the
      // active profile so the reset of baseUrl/model that follows
      // doesn't silently rewrite a saved configuration.
      set({ aiVendorId, aiActiveProfileId: null });
      persist();
    },
    setAiProviderKind: (aiProviderKind) => {
      set({ aiProviderKind });
      persist();
    },
    setAiBaseUrl: (aiBaseUrl) => {
      const trimmed = aiBaseUrl.trim();
      set({ aiBaseUrl: trimmed });
      syncActiveAiProfile({ baseUrl: trimmed });
      persist();
    },
    setAiCliBin: (aiCliBin) => {
      const trimmed = aiCliBin.trim();
      set({ aiCliBin: trimmed });
      syncActiveAiProfile({ cliBin: trimmed });
      persist();
    },
    setAiCliMode: (aiCliMode) => {
      set({ aiCliMode });
      syncActiveAiProfile({ cliMode: aiCliMode });
      persist();
    },
    setAiModel: (aiModel) => {
      const trimmed = aiModel.trim();
      set({ aiModel: trimmed });
      syncActiveAiProfile({ model: trimmed });
      persist();
    },
    setAiMaxTokens: (n) => {
      const clamped = Math.max(0, Math.min(64000, Math.round(n)));
      set({ aiMaxTokens: clamped });
      syncActiveAiProfile({ maxTokens: clamped });
      persist();
    },
    setAiAutoContext: (aiAutoContext) => {
      set({ aiAutoContext });
      persist();
    },
    setAiRedact: (aiRedact) => {
      set({ aiRedact });
      persist();
    },
    setAiAskReadOnly: (aiAskReadOnly) => {
      set({ aiAskReadOnly });
      persist();
    },
    setAiSkipL2Confirm: (aiSkipL2Confirm) => {
      set({ aiSkipL2Confirm });
      persist();
    },
    setAiPersistHistory: (aiPersistHistory) => {
      set({ aiPersistHistory });
      persist();
    },
    saveCurrentAsAiProfile: () => {
      const s = get();
      // CLI backends may run on the account's default model, so an empty
      // model is valid for them (§5.14.8); other kinds still require one.
      if (!s.aiModel.trim() && s.aiProviderKind !== "cli") return;
      // Dedupe: re-saving an identical combo just re-activates it.
      const existing = s.aiProfiles.find(
        (p) => p.vendorId === s.aiVendorId && p.baseUrl === s.aiBaseUrl && p.model === s.aiModel,
      );
      if (existing) {
        set({
          aiActiveProfileId: existing.id,
          aiProfiles: s.aiProfiles.map((p) =>
            p.id === existing.id
              ? {
                  ...p,
                  maxTokens: s.aiMaxTokens,
                  kind: s.aiProviderKind,
                  cliBin: s.aiCliBin,
                  cliMode: s.aiCliMode,
                }
              : p,
          ),
        });
        persist();
        return;
      }
      const profile: AiProfile = {
        id: crypto.randomUUID(),
        name: aiProfileName(s.aiVendorId, s.aiModel || s.aiProviderKind),
        vendorId: s.aiVendorId,
        kind: s.aiProviderKind,
        baseUrl: s.aiBaseUrl,
        model: s.aiModel,
        maxTokens: s.aiMaxTokens,
        cliBin: s.aiCliBin,
        cliMode: s.aiCliMode,
      };
      set({ aiProfiles: [...s.aiProfiles, profile], aiActiveProfileId: profile.id });
      persist();
    },
    activateAiProfile: (id) => {
      const s = get();
      const profile = s.aiProfiles.find((p) => p.id === id);
      if (!profile) return;
      set({
        aiActiveProfileId: profile.id,
        aiVendorId: profile.vendorId,
        aiProviderKind: profile.kind,
        aiBaseUrl: profile.baseUrl,
        aiCliBin: profile.cliBin ?? "",
        aiCliMode: profile.cliMode ?? "m1",
        aiModel: profile.model,
        aiMaxTokens: profile.maxTokens,
      });
      persist();
    },
    deleteAiProfile: (id) => {
      const s = get();
      set({
        aiProfiles: s.aiProfiles.filter((p) => p.id !== id),
        // Working fields keep their values; only the link is cut.
        aiActiveProfileId: s.aiActiveProfileId === id ? null : s.aiActiveProfileId,
      });
      persist();
    },
  };
});
