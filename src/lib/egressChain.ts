import type { EgressProfile, SavedSshConnection } from "./types";

export type EgressHop = {
  /** Display name (profile.name when present, otherwise profile.id). */
  label: string;
  /** Underlying profile for tooltip detail. */
  profile: EgressProfile;
};

/** Walk an egress reference to its leaf, expanding `ssh_jump` hops via
 *  the saved SSH connection they reference. Cycles are guarded by
 *  `visited`; `maxDepth` caps recursion to match the backend's
 *  `SshJumpContext` limit so a degenerate chain can't render an
 *  unbounded badge. */
export function buildEgressChain(
  rootEgressId: string | null | undefined,
  profiles: EgressProfile[],
  connections: SavedSshConnection[],
  visited: Set<string> = new Set(),
  maxDepth = 8,
): EgressHop[] {
  if (!rootEgressId || visited.size >= maxDepth) return [];
  if (visited.has(rootEgressId)) return [];
  visited.add(rootEgressId);
  const profile = profiles.find((p) => p.id === rootEgressId);
  if (!profile) return [];
  const hop: EgressHop = { label: profile.name || profile.id, profile };
  if (profile.kind === "ssh_jump") {
    const conn = connections.find((c) => c.name === profile.viaConnection);
    if (conn) {
      return [hop, ...buildEgressChain(conn.egressId, profiles, connections, visited, maxDepth)];
    }
  }
  return [hop];
}

/** Render a chain into the compact `local → A → target` form used in
 *  the StatusBar badge. The leading "local" anchor is always present;
 *  pass an empty `hops` array for a direct connection. */
export function formatChain(hops: EgressHop[], targetLabel: string): string {
  const parts = ["local", ...hops.map((h) => h.label), targetLabel];
  return parts.join(" → ");
}

/** One-line tooltip describing each hop's kind + key fields, suitable
 *  for the badge's `title` attribute. */
export function describeHop(hop: EgressHop): string {
  const p = hop.profile;
  switch (p.kind) {
    case "socks5":
      return `${hop.label} · SOCKS5 ${p.host}:${p.port}`;
    case "http":
      return `${hop.label} · HTTP ${p.host}:${p.port}`;
    case "ssh_jump":
      return `${hop.label} · SSH jump via ${p.viaConnection}`;
    case "wireguard":
      return `${hop.label} · WireGuard ${p.confPath || "(managed)"}`;
    case "external_vpn":
      return `${hop.label} · ${p.engine}`;
    case "none":
      return `${hop.label} · direct`;
  }
}
