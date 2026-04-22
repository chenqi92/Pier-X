import * as cmd from "./commands";
import { translate } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import type { TabState, TunnelInfoView } from "./types";
import { effectiveSshTarget } from "./types";

type TunnelSlot = "mysql" | "postgres" | "redis";
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
      updateTab(tab.id, tunnelPatch(slot, null));
      return null;
    }
    if (getTunnelPort(tab, slot) !== info.localPort) {
      updateTab(tab.id, tunnelPatch(slot, info));
    }
    return info;
  } catch {
    updateTab(tab.id, tunnelPatch(slot, null));
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

export async function ensureTunnelSlot(params: {
  tab: TabState;
  slot: TunnelSlot;
  remoteHost: string;
  remotePort: number;
  updateTab: UpdateTab;
  force?: boolean;
}) {
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
  const tunnelId = getTunnelId(tab, slot);

  if (tunnelId) {
    try {
      const info = await cmd.sshTunnelInfo(tunnelId);
      if (
        !force &&
        info.alive &&
        info.remoteHost === resolvedRemoteHost &&
        info.remotePort === remotePort
      ) {
        if (getTunnelPort(tab, slot) !== info.localPort) {
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
  updateTab(tab.id, tunnelPatch(slot, info));
  return info;
}
