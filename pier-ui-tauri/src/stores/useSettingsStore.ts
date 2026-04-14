import { create } from "zustand";

export type Locale = "en" | "zh";

type SettingsState = {
  locale: Locale;
  terminalFontSize: number;
  cursorStyle: 0 | 1 | 2; // 0=Block, 1=Beam, 2=Underline
  cursorBlink: boolean;
  scrollbackLines: number;
  visualBell: boolean;
  audioBell: boolean;
  setLocale: (locale: Locale) => void;
  setTerminalFontSize: (size: number) => void;
  setCursorStyle: (style: 0 | 1 | 2) => void;
  setCursorBlink: (blink: boolean) => void;
  setScrollbackLines: (lines: number) => void;
  setVisualBell: (on: boolean) => void;
  setAudioBell: (on: boolean) => void;
};

export const useSettingsStore = create<SettingsState>((set) => ({
  locale: "en",
  terminalFontSize: 13,
  cursorStyle: 0,
  cursorBlink: true,
  scrollbackLines: 10000,
  visualBell: true,
  audioBell: false,
  setLocale: (locale) => set({ locale }),
  setTerminalFontSize: (terminalFontSize) => set({ terminalFontSize }),
  setCursorStyle: (cursorStyle) => set({ cursorStyle }),
  setCursorBlink: (cursorBlink) => set({ cursorBlink }),
  setScrollbackLines: (scrollbackLines) => set({ scrollbackLines }),
  setVisualBell: (visualBell) => set({ visualBell }),
  setAudioBell: (audioBell) => set({ audioBell }),
}));
