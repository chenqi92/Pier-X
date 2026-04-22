import { create } from "zustand";
import type { DetectedDbInstance, RightTool } from "../lib/types";

type Status = "pending" | "ready" | "error";

type TabEntry = {
  status: Status;
  tools: Set<RightTool>;
};

type DbInstanceEntry = {
  status: Status;
  instances: DetectedDbInstance[];
  /** CLI availability mirror from `DbDetectionReport`. */
  mysqlCli: boolean;
  psqlCli: boolean;
  redisCli: boolean;
  sqliteCli: boolean;
  /** Last-refreshed timestamp (ms). Consumers skip a refresh
   *  if `Date.now() - at < 60_000`. */
  at: number;
};

type Store = {
  byTab: Record<string, TabEntry>;
  /** Parallel map for DB instances from `db_detect`. Kept
   *  separate from `byTab` because the service-chip flow is
   *  boolean-by-tool; DB instances are first-class rows. */
  instancesByTab: Record<string, DbInstanceEntry>;
  setPending: (tabId: string) => void;
  setReady: (tabId: string, tools: RightTool[]) => void;
  setError: (tabId: string) => void;
  clearTab: (tabId: string) => void;
  setDbInstancesPending: (tabId: string) => void;
  setDbInstances: (
    tabId: string,
    payload: {
      instances: DetectedDbInstance[];
      mysqlCli: boolean;
      psqlCli: boolean;
      redisCli: boolean;
      sqliteCli: boolean;
    },
  ) => void;
  setDbInstancesError: (tabId: string) => void;
};

const EMPTY_DB_ENTRY: DbInstanceEntry = {
  status: "ready",
  instances: [],
  mysqlCli: false,
  psqlCli: false,
  redisCli: false,
  sqliteCli: false,
  at: 0,
};

export const useDetectedServicesStore = create<Store>((set) => ({
  byTab: {},
  instancesByTab: {},
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
      const byTabChanged = tabId in state.byTab;
      const instancesChanged = tabId in state.instancesByTab;
      if (!byTabChanged && !instancesChanged) return state;
      const nextByTab = { ...state.byTab };
      delete nextByTab[tabId];
      const nextInstances = { ...state.instancesByTab };
      delete nextInstances[tabId];
      return { byTab: nextByTab, instancesByTab: nextInstances };
    }),
  setDbInstancesPending: (tabId) =>
    set((state) => ({
      instancesByTab: {
        ...state.instancesByTab,
        [tabId]: {
          ...(state.instancesByTab[tabId] ?? EMPTY_DB_ENTRY),
          status: "pending",
        },
      },
    })),
  setDbInstances: (tabId, payload) =>
    set((state) => ({
      instancesByTab: {
        ...state.instancesByTab,
        [tabId]: {
          status: "ready",
          instances: payload.instances,
          mysqlCli: payload.mysqlCli,
          psqlCli: payload.psqlCli,
          redisCli: payload.redisCli,
          sqliteCli: payload.sqliteCli,
          at: Date.now(),
        },
      },
    })),
  setDbInstancesError: (tabId) =>
    set((state) => ({
      instancesByTab: {
        ...state.instancesByTab,
        [tabId]: {
          ...(state.instancesByTab[tabId] ?? EMPTY_DB_ENTRY),
          status: "error",
        },
      },
    })),
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
