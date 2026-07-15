import * as cmd from "./commands";
import { translate } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useTabStore } from "../stores/useTabStore";
import type { TabState, TunnelInfoView } from "./types";
import { effectiveSshTarget } from "./types";

type TunnelSlot = "mysql" | "postgres" | "redis" | "sqlserver" | "influx";
type UpdateTab = (id: string, patch: Partial<TabState>) => void;

const tunnelFields = {
  mysql: {
    id: "mysqlTunnelId",
    port: "mysqlTunnelPort",
  },
  postgres: {
    id: "pgTunnelId",
    port: "pgTunnelPort",
  },
  redis: {
    id: "redisTunnelId",
    port: "redisTunnelPort",
  },
  sqlserver: {
    id: "mssqlTunnelId",
    port: "mssqlTunnelPort",
  },
  influx: {
    id: "influxTunnelId",
    port: "influxTunnelPort",
  },
} as const satisfies Record<TunnelSlot, { id: keyof TabState; port: keyof TabState }>;

function normalizedRemoteHost(remoteHost: string) {
  return remoteHost.trim() || "127.0.0.1";
}

function message(key: string) {
  return translate(useSettingsStore.getState().locale, key);
}

function getTunnelId(tab: TabState, slot: TunnelSlot) {
  return tab[tunnelFields[slot].id] as string | null;
}

export function getTunnelPort(tab: TabState, slot: TunnelSlot) {
  return tab[tunnelFields[slot].port] as number | null;
}

function tunnelPatch(slot: TunnelSlot, info: TunnelInfoView | null): Partial<TabState> {
  return {
    [tunnelFields[slot].id]: info?.tunnelId ?? null,
    [tunnelFields[slot].port]: info?.localPort ?? null,
  } as Partial<TabState>;
}

export async function syncTunnelState(
  tab: TabState,
  slot: TunnelSlot,
  updateTab: UpdateTab,
) {
  const tunnelId = getTunnelId(tab, slot);
  if (!tunnelId) {
    return null;
  }

  try {
    const info = await cmd.sshTunnelInfo(tunnelId);
    if (!info.alive) {
      // Dead tunnel (its accept loop exited): close the backend
      // ManagedTunnel before dropping our reference, otherwise its listener
      // + map entry leak until process exit — nobody else holds the id.
      await cmd.sshTunnelClose(tunnelId).catch(() => {});
      updateTab(tab.id, tunnelPatch(slot, null));
      return null;
    }
    if (getTunnelPort(tab, slot) !== info.localPort) {
      updateTab(tab.id, tunnelPatch(slot, info));
    }
    return info;
  } catch {
    // A transient IPC error must NOT make us forget a possibly-live tunnel:
    // clearing the slot here would orphan the backend ManagedTunnel (it keeps
    // running, but the frontend no longer has its id to close it). Keep the
    // slot and let the next sync / operation re-check or reopen.
    return null;
  }
}

export async function closeTunnelSlot(
  tab: TabState,
  slot: TunnelSlot,
  updateTab: UpdateTab,
) {
  const tunnelId = getTunnelId(tab, slot);
  if (tunnelId) {
    await cmd.sshTunnelClose(tunnelId).catch(() => {});
  }
  updateTab(tab.id, tunnelPatch(slot, null));
}

type EnsureTunnelParams = {
  tab: TabState;
  slot: TunnelSlot;
  remoteHost: string;
  remotePort: number;
  updateTab: UpdateTab;
  force?: boolean;
};

// Overlapping ensure calls on the same (tab, slot) — e.g. two rapid
// draft connects, each with `force` — would race the close/reopen and
// leak whichever tunnel loses the final `updateTab`. Later calls queue
// behind the in-flight one and then run against the live tab state.
const ensureChain = new Map<string, Promise<unknown>>();

export async function ensureTunnelSlot(params: EnsureTunnelParams) {
  const key = `${params.tab.id}:${params.slot}`;
  const run = (ensureChain.get(key) ?? Promise.resolve())
    .catch(() => {})
    .then(() => ensureTunnelSlotNow(params));
  const link = run.catch(() => {});
  ensureChain.set(key, link);
  try {
    return await run;
  } finally {
    if (ensureChain.get(key) === link) ensureChain.delete(key);
  }
}

async function ensureTunnelSlotNow(params: EnsureTunnelParams) {
  const {
    tab,
    slot,
    remoteHost,
    remotePort,
    updateTab,
    force = false,
  } = params;

  // Tunnels can be opened against any SSH context — a real SSH tab,
  // a local terminal where the user typed `ssh user@host` (we mirror
  // the addressing onto the tab), or a nested-ssh overlay on top of
  // a real SSH tab. `effectiveSshTarget` picks whichever one applies.
  const target = effectiveSshTarget(tab);
  if (!target) {
    throw new Error(message("SSH connection required."));
  }
  if (!Number.isFinite(remotePort) || remotePort <= 0) {
    throw new Error(message("Tunnel remote port must not be empty."));
  }

  const resolvedRemoteHost = normalizedRemoteHost(remoteHost);
  // `tab` is the caller's render-time snapshot — a queued call must see
  // the tunnel a just-finished ensure opened (and close it under
  // `force`), so read the slot from the live store instead.
  const liveTab = useTabStore.getState().tabs.find((other) => other.id === tab.id) ?? tab;
  const tunnelId = getTunnelId(liveTab, slot);

  if (tunnelId) {
    try {
      const info = await cmd.sshTunnelInfo(tunnelId);
      if (
        !force &&
        info.alive &&
        info.remoteHost === resolvedRemoteHost &&
        info.remotePort === remotePort
      ) {
        if (getTunnelPort(liveTab, slot) !== info.localPort) {
          updateTab(tab.id, tunnelPatch(slot, info));
        }
        return info;
      }
    } catch {
      // Tunnel was already gone; reopen below.
    }

    await cmd.sshTunnelClose(tunnelId).catch(() => {});
    updateTab(tab.id, tunnelPatch(slot, null));
  }

  const info = await cmd.sshTunnelOpen({
    host: target.host,
    port: target.port,
    user: target.user,
    authMode: target.authMode,
    password: target.password,
    keyPath: target.keyPath,
    remoteHost: resolvedRemoteHost,
    remotePort,
    localPort: null,
    savedConnectionIndex: target.savedConnectionIndex,
  });
  // If the tab was closed while the open was in flight, `closeTabTunnels` ran
  // before this tunnel existed — nobody will ever close it, so the backend
  // ManagedTunnel + its local port would leak. Detect the vanished tab and
  // close the just-opened tunnel instead of writing to a dead tab record.
  const tabStillOpen = useTabStore
    .getState()
    .tabs.some((other) => other.id === tab.id);
  if (!tabStillOpen) {
    await cmd.sshTunnelClose(info.tunnelId).catch(() => {});
    return info;
  }
  updateTab(tab.id, tunnelPatch(slot, info));
  return info;
}
