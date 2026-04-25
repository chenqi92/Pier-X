import { create } from "zustand";

export type Locale = "en" | "zh";

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
  setEditorWrapDefault: (on: boolean) => void;
  setEditorLineNumbersDefault: (on: boolean) => void;
  setEditorTabSize: (n: number) => void;
  setEditorTrimTrailingOnSave: (on: boolean) => void;
  setEditorEnsureFinalNewlineOnSave: (on: boolean) => void;
  setGitCommitSigning: (on: boolean) => void;
  setUpdateCheckOnStartup: (on: boolean) => void;
  setSecretScanPatterns: (patterns: string) => void;
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
  editorWrapDefault: boolean;
  editorLineNumbersDefault: boolean;
  editorTabSize: number;
  editorTrimTrailingOnSave: boolean;
  editorEnsureFinalNewlineOnSave: boolean;
  gitCommitSigning: boolean;
  updateCheckOnStartup: boolean;
  secretScanPatterns: string;
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
  editorWrapDefault: false,
  editorLineNumbersDefault: true,
  editorTabSize: 2,
  editorTrimTrailingOnSave: false,
  editorEnsureFinalNewlineOnSave: false,
  gitCommitSigning: false,
  updateCheckOnStartup: false,
  secretScanPatterns: "",
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
  document.documentElement.style.setProperty("font-size", `${13 * scale}px`);
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
  };

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
      editorWrapDefault: s.editorWrapDefault,
      editorLineNumbersDefault: s.editorLineNumbersDefault,
      editorTabSize: s.editorTabSize,
      editorTrimTrailingOnSave: s.editorTrimTrailingOnSave,
      editorEnsureFinalNewlineOnSave: s.editorEnsureFinalNewlineOnSave,
      gitCommitSigning: s.gitCommitSigning,
      updateCheckOnStartup: s.updateCheckOnStartup,
      secretScanPatterns: s.secretScanPatterns,
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
  };
});
