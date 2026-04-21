import { create } from "zustand";
import type { RightTool } from "../lib/types";

type Status = "pending" | "ready" | "error";

type TabEntry = {
  status: Status;
  tools: Set<RightTool>;
};

type Store = {
  byTab: Record<string, TabEntry>;
  setPending: (tabId: string) => void;
  setReady: (tabId: string, tools: RightTool[]) => void;
  setError: (tabId: string) => void;
  clearTab: (tabId: string) => void;
};

export const useDetectedServicesStore = create<Store>((set) => ({
  byTab: {},
  setPending: (tabId) =>
    set((state) => ({
      byTab: { ...state.byTab, [tabId]: { status: "pending", tools: new Set() } },
    })),
  setReady: (tabId, tools) =>
    set((state) => ({
      byTab: {
        ...state.byTab,
        [tabId]: { status: "ready", tools: new Set(tools) },
      },
    })),
  setError: (tabId) =>
    set((state) => ({
      byTab: { ...state.byTab, [tabId]: { status: "error", tools: new Set() } },
    })),
  clearTab: (tabId) =>
    set((state) => {
      if (!state.byTab[tabId]) return state;
      const next = { ...state.byTab };
      delete next[tabId];
      return { byTab: next };
    }),
}));

export function mapServiceToTool(name: string): RightTool | null {
  switch (name) {
    case "mysql":
      return "mysql";
    case "postgresql":
      return "postgres";
    case "redis":
      return "redis";
    case "docker":
      return "docker";
    default:
      return null;
  }
}
