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

export const useSettingsStore = create<SettingsState>((set) => ({
  locale: "zh",
  performanceOverlay: false,
  uiFontFamily: "IBM Plex Sans",
  uiScale: 1.0,
  monoFontFamily: "IBM Plex Mono",
  terminalFontSize: 13,
  cursorStyle: 0,
  cursorBlink: true,
  scrollbackLines: 10000,
  visualBell: true,
  audioBell: false,
  terminalRowSeparators: false,
  setLocale: (locale) => set({ locale }),
  setPerformanceOverlay: (performanceOverlay) => set({ performanceOverlay }),
  setUiFontFamily: (uiFontFamily) => {
    set({ uiFontFamily });
    document.documentElement.style.setProperty(
      "--sans",
      `"${uiFontFamily}", system-ui, -apple-system, "SF Pro Text", "Segoe UI", sans-serif`,
    );
    document.documentElement.style.setProperty("--font-ui", `var(--sans)`);
  },
  setUiScale: (uiScale) => {
    set({ uiScale });
    document.documentElement.style.setProperty("font-size", `${13 * uiScale}px`);
  },
  setMonoFontFamily: (monoFontFamily) => {
    set({ monoFontFamily });
    document.documentElement.style.setProperty(
      "--mono",
      `"${monoFontFamily}", ui-monospace, "SF Mono", Consolas, monospace`,
    );
    document.documentElement.style.setProperty("--font-mono", `var(--mono)`);
  },
  setTerminalFontSize: (terminalFontSize) => set({ terminalFontSize }),
  setCursorStyle: (cursorStyle) => set({ cursorStyle }),
  setCursorBlink: (cursorBlink) => set({ cursorBlink }),
  setScrollbackLines: (scrollbackLines) => set({ scrollbackLines }),
  setVisualBell: (visualBell) => set({ visualBell }),
  setAudioBell: (audioBell) => set({ audioBell }),
  setTerminalRowSeparators: (terminalRowSeparators) => set({ terminalRowSeparators }),
}));
