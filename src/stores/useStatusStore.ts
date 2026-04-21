import { create } from "zustand";

type StatusState = {
  branch: string | null;
  ahead: number;
  behind: number;
  terminalCols: number | null;
  terminalRows: number | null;
  setGitStatus: (branch: string | null, ahead: number, behind: number) => void;
  clearGitStatus: () => void;
  setTerminalSize: (cols: number | null, rows: number | null) => void;
};

export const useStatusStore = create<StatusState>((set) => ({
  branch: null,
  ahead: 0,
  behind: 0,
  terminalCols: null,
  terminalRows: null,
  setGitStatus: (branch, ahead, behind) => set({ branch, ahead, behind }),
  clearGitStatus: () => set({ branch: null, ahead: 0, behind: 0 }),
  setTerminalSize: (terminalCols, terminalRows) => set({ terminalCols, terminalRows }),
}));
