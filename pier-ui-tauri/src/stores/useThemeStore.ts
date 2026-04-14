import { create } from "zustand";

export type ThemeMode = "system" | "dark" | "light";

export type TerminalTheme = {
  name: string;
  fg: string;
  bg: string;
  ansi: string[];
};

export const TERMINAL_THEMES: TerminalTheme[] = [
  {
    name: "Default Dark",
    fg: "#e8eaed",
    bg: "#0f1115",
    ansi: ["#000000","#CD0000","#00CD00","#CDCD00","#3B78FF","#CD00CD","#00CDCD","#E5E5E5","#7F7F7F","#FF0000","#00FF00","#FFFF00","#5C5CFF","#FF00FF","#00FFFF","#FFFFFF"],
  },
  {
    name: "Default Light",
    fg: "#1f2329",
    bg: "#fbfcfd",
    ansi: ["#000000","#CD0000","#00A000","#A07000","#0000EE","#CD00CD","#00A0A0","#666666","#555555","#FF0000","#00CD00","#CDCD00","#5C5CFF","#FF00FF","#00CDCD","#444444"],
  },
  {
    name: "Solarized Dark",
    fg: "#839496",
    bg: "#002B36",
    ansi: ["#073642","#DC322F","#859900","#B58900","#268BD2","#D33682","#2AA198","#EEE8D5","#002B36","#CB4B16","#586E75","#657B83","#839496","#6C71C4","#93A1A1","#FDF6E3"],
  },
  {
    name: "Dracula",
    fg: "#F8F8F2",
    bg: "#282A36",
    ansi: ["#21222C","#FF5555","#50FA7B","#F1FA8C","#BD93F9","#FF79C6","#8BE9FD","#F8F8F2","#6272A4","#FF6E6E","#69FF94","#FFFFA5","#D6ACFF","#FF92DF","#A4FFFF","#FFFFFF"],
  },
  {
    name: "Monokai",
    fg: "#F8F8F2",
    bg: "#272822",
    ansi: ["#272822","#F92672","#A6E22E","#F4BF75","#66D9EF","#AE81FF","#A1EFE4","#F8F8F2","#75715E","#F92672","#A6E22E","#F4BF75","#66D9EF","#AE81FF","#A1EFE4","#F9F8F5"],
  },
  {
    name: "Nord",
    fg: "#D8DEE9",
    bg: "#2E3440",
    ansi: ["#3B4252","#BF616A","#A3BE8C","#EBCB8B","#81A1C1","#B48EAD","#88C0D0","#E5E9F0","#4C566A","#BF616A","#A3BE8C","#EBCB8B","#81A1C1","#B48EAD","#8FBCBB","#ECEFF4"],
  },
];

type ThemeState = {
  mode: ThemeMode;
  resolvedDark: boolean;
  terminalThemeIndex: number;
  setMode: (mode: ThemeMode) => void;
  setTerminalTheme: (index: number) => void;
};

function resolveTheme(mode: ThemeMode): boolean {
  if (mode === "dark") return true;
  if (mode === "light") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyTheme(dark: boolean) {
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

export const useThemeStore = create<ThemeState>((set) => {
  const initialMode: ThemeMode = "dark";
  const initialDark = resolveTheme(initialMode);
  applyTheme(initialDark);

  // Listen for system theme changes
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", () => {
    const state = useThemeStore.getState();
    if (state.mode === "system") {
      const dark = resolveTheme("system");
      applyTheme(dark);
      set({ resolvedDark: dark });
    }
  });

  return {
    mode: initialMode,
    resolvedDark: initialDark,
    terminalThemeIndex: 0,
    setMode: (mode) => {
      const dark = resolveTheme(mode);
      applyTheme(dark);
      set({ mode, resolvedDark: dark });
    },
    setTerminalTheme: (index) =>
      set({ terminalThemeIndex: Math.max(0, Math.min(index, TERMINAL_THEMES.length - 1)) }),
  };
});
