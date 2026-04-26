import { Cpu, HardDrive, KeyRound, MemoryStick, Network, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import * as cmd from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type { BlockDeviceEntryView, ServerSnapshotView, TabState } from "../lib/types";
import { effectiveSshTarget, isSshTargetReady } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { isMissingKeychainError, localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import DismissibleNote from "../components/DismissibleNote";
import StatusDot from "../components/StatusDot";
import { useUiActionsStore } from "../stores/useUiActionsStore";
import { logEvent } from "../lib/logger";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = {
  tab: TabState;
  /** Open the saved-connection editor when the keychain has lost the
   *  password for this tab's saved connection. */
  onEditConnection?: (index: number) => void;
  /** True when this panel is the visible right-side tool. When false
   *  the 5-second polling is suspended so keep-alive instances don't
   *  burn SSH probes in the background. */
  isActive?: boolean;
};

const MONITOR_ICON = RIGHT_TOOL_META.monitor.icon;

/**
 * Format a bytes-per-second number into a compact human-readable
 * string with units, used by the NETWORK gauge. Returns `null` when
 * the value is below the "no rate yet" sentinel so the gauge can
 * fall back to its placeholder.
 */
function formatRate(bps: number): { value: string; unit: string } | null {
  if (!Number.isFinite(bps) || bps < 0) return null;
  if (bps >= 1024 * 1024) return { value: (bps / (1024 * 1024)).toFixed(1), unit: "MB/s" };
  if (bps >= 1024) return { value: (bps / 1024).toFixed(1), unit: "KB/s" };
  return { value: bps.toFixed(0), unit: "B/s" };
}

type GaugeTone = "accent" | "pos" | "warn" | "off";

function Gauge({
  icon: Icon,
  label,
  value,
  sub,
  pct,
  tone = "accent",
}: {
  icon: ReactNode;
  label: string;
  value: ReactNode;
  sub: string;
  pct: number;
  tone?: GaugeTone;
}) {
  // "off" is the placeholder tone used before the first probe lands —
  // the bar renders empty and the fill color falls back to the muted
  // palette so the chrome stays visually neutral.
  const color =
    tone === "pos" ? "var(--pos)"
      : tone === "warn" ? "var(--warn)"
      : tone === "off" ? "var(--dim)"
      : "var(--accent)";
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <div className="mon-gauge">
      <div className="mon-gauge-label">
        {Icon}
        <span>{label}</span>
      </div>
      <div className="mon-gauge-value">{value}</div>
      <div className="mon-gauge-bar">
        <div className="mon-gauge-fill" style={{ width: `${clamped}%`, background: color }} />
      </div>
      <div className="mon-gauge-sub mono">{sub}</div>
    </div>
  );
}

function toneFromPct(pct: number): GaugeTone {
  if (pct >= 85) return "warn";
  if (pct >= 50) return "accent";
  return "pos";
}

function formatTimestamp(ts: number): string {
  if (!ts) return "—";
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Render lsblk's `SIZE` (bytes) using the same 1024-power units df uses,
 *  so the BLOCK DEVICES tree reads consistently with the DISKS table.
 *  Mirrors the backend's `format_df_size` — kept in sync intentionally. */
function formatBytes(n: number): string {
  if (!n || n <= 0) return "—";
  const units = [
    ["E", 1024 ** 6],
    ["P", 1024 ** 5],
    ["T", 1024 ** 4],
    ["G", 1024 ** 3],
    ["M", 1024 ** 2],
    ["K", 1024],
  ] as const;
  for (const [label, scale] of units) {
    if (n >= scale) {
      const v = n / scale;
      return v >= 10 ? `${v.toFixed(0)}${label}` : `${v.toFixed(1)}${label}`;
    }
  }
  return `${n}B`;
}

/** Compact descriptor for the DISKS-table TYPE column. e.g. "SSD · NVMe",
 *  "HDD · SATA", "virt · virtio". Returns null when no block-device row
 *  matches the mount (lsblk wasn't available, or the mount is on a path
 *  lsblk doesn't expose). */
function describeBlock(block: BlockDeviceEntryView | undefined): string | null {
  if (!block) return null;
  const tran = block.tran ? block.tran.toUpperCase() : null;
  // virtio is by definition a virtual disk; surface that explicitly so
  // the user can tell a passthrough nvme/sata apart from a hypervisor
  // virtual disk at a glance.
  if (block.tran === "virtio") return tran ? `virt · ${tran}` : "virt";
  const media = block.rota ? "HDD" : "SSD";
  return tran ? `${media} · ${tran}` : media;
}

/** Build a map from mountpoint → owning block device, walking up the
 *  pkname chain so a mount on `vg-home` (lvm) resolves all the way to
 *  the physical `nvme0n1` for media/transport info. We prefer the
 *  attributes of the *physical* root because that's what determines
 *  "is it really an SSD on an NVMe bus" — the lvm/crypt layers don't
 *  carry their own ROTA/TRAN and would otherwise read empty. */
function buildMountToBlock(
  blocks: BlockDeviceEntryView[],
): Map<string, BlockDeviceEntryView> {
  const byKname = new Map<string, BlockDeviceEntryView>();
  for (const b of blocks) byKname.set(b.kname, b);

  function rootOf(b: BlockDeviceEntryView): BlockDeviceEntryView {
    let cur = b;
    const seen = new Set<string>();
    while (cur.pkname && !seen.has(cur.kname)) {
      seen.add(cur.kname);
      const parent = byKname.get(cur.pkname);
      if (!parent) break;
      cur = parent;
    }
    return cur;
  }

  const out = new Map<string, BlockDeviceEntryView>();
  for (const b of blocks) {
    if (!b.mountpoint) continue;
    const root = rootOf(b);
    // Synthesise a row that keeps the leaf's identity (so tooltip can
    // show the actual mounted device) but borrows ROTA/TRAN/MODEL from
    // the physical disk that backs it.
    out.set(b.mountpoint, {
      ...b,
      rota: root.rota,
      tran: root.tran || b.tran,
      model: root.model || b.model,
    });
  }
  return out;
}

type BlockTreeNode = BlockDeviceEntryView & { children: BlockTreeNode[] };

/** One row in the BLOCK DEVICES tree. The connector glyph + indent
 *  encode the parent/child relationship without needing CSS lines. */
function BlockTreeRow({ node, depth }: { node: BlockTreeNode; depth: number }) {
  // Top-level (physical disk) gets the meaningful media/bus chips;
  // children inherit visually via indentation, so we only repeat the
  // bus on devices that have their own (none of the dm/crypt/lvm
  // layers do). MODEL goes into the row tooltip to keep the row tight.
  const tran = node.tran ? node.tran.toUpperCase() : null;
  const media = depth === 0 ? (node.rota ? "HDD" : "SSD") : null;
  const sizeText = node.sizeBytes > 0 ? formatBytes(node.sizeBytes) : "—";
  const titleParts = [node.kname, node.devType];
  if (node.model) titleParts.push(node.model);
  if (node.fsType) titleParts.push(node.fsType);
  if (node.mountpoint) titleParts.push(`→ ${node.mountpoint}`);
  return (
    <>
      <li className="mon-tree-row" title={titleParts.join(" · ")}>
        <span className="mon-tree-name mono">
          <span className="mon-tree-indent" style={{ width: depth * 12 }} aria-hidden />
          {depth > 0 && <span className="mon-tree-branch mono">└─ </span>}
          {node.name || node.kname}
        </span>
        <span className="mon-tree-meta mono">
          {media && <span className="mon-tree-chip">{media}</span>}
          {tran && <span className="mon-tree-chip">{tran}</span>}
          <span className="mon-tree-type">{node.devType || ""}</span>
        </span>
        <span className="mon-tree-size mono">{sizeText}</span>
        <span className="mon-tree-mount mono mon-cell-trunc">
          {node.mountpoint || node.fsType || "—"}
        </span>
      </li>
      {node.children.map((c) => (
        <BlockTreeRow key={c.kname} node={c} depth={depth + 1} />
      ))}
    </>
  );
}

/** Stitch the flat lsblk rows into the disk → part → crypt → lv tree.
 *  Roots are rows with empty `pkname` (physical disks) or rows whose
 *  parent isn't in the input (defensive — keeps orphan rows visible
 *  rather than silently dropping them). */
function buildBlockTree(blocks: BlockDeviceEntryView[]): BlockTreeNode[] {
  const nodes = new Map<string, BlockTreeNode>();
  for (const b of blocks) nodes.set(b.kname, { ...b, children: [] });
  const roots: BlockTreeNode[] = [];
  for (const node of nodes.values()) {
    if (node.pkname && nodes.has(node.pkname)) {
      nodes.get(node.pkname)!.children.push(node);
    } else {
      roots.push(node);
    }
  }
  // Stable order: physical disks first sorted by kname, children sorted
  // by kname too, so re-renders don't shuffle rows around.
  const sortRec = (arr: BlockTreeNode[]) => {
    arr.sort((a, b) => a.kname.localeCompare(b.kname));
    for (const n of arr) sortRec(n.children);
  };
  sortRec(roots);
  return roots;
}

export default function ServerMonitorPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <ServerMonitorPanelBody {...props} /> : <PanelSkeleton variant="chrome" />}
    </div>
  );
}

function ServerMonitorPanelBody({ tab, onEditConnection, isActive = true }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [snap, setSnap] = useState<ServerSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  // Which metric the top-processes table is sorted by. The backend
  // returns two separate top-8 lists (one per metric) so this flip
  // is a free render swap, no extra probe fired.
  const [procSort, setProcSort] = useState<"cpu" | "mem">("cpu");
  // Mirrors `busy` for the polling interval — reading it via ref
  // means we don't have to put `busy` in the effect's deps and pay
  // the interval-teardown-on-every-probe cost.
  const busyRef = useRef(false);
  busyRef.current = busy;
  const [error, setError] = useState("");
  // Track the missing-keychain condition separately so the recovery
  // button stays available even after a localized error string has
  // been transformed beyond regex recognition.
  const [needsPasswordRecovery, setNeedsPasswordRecovery] = useState(false);
  const [lastProbed, setLastProbed] = useState(0);

  // SSH context is "available" any time the tab has the addressing
  // bits filled in — either via the primary fields (real SSH tab),
  // mirrored fields (local terminal that ran `ssh user@host`), or
  // the nested-ssh overlay (`ssh user@host` inside an existing SSH
  // session). `effectiveSshTarget` collapses all three into one
  // shape so the probe / detect commands always reach the host the
  // user thinks they are looking at.
  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  // Only treat the tab as "local probe" when there is no SSH target
  // overlay; otherwise the SSH path takes priority.
  const isLocal = tab.backend === "local" && !hasSsh;

  // The probe runs in two cadences:
  //   • fast (5 s)  — CPU, memory, network, processes, uptime/load
  //   • full (30 s) — adds the disk segments (`df` + `lsblk`)
  //
  // `df` is cheap (statvfs against the kernel cache, no disk I/O) but
  // running it every 5 s burns SSH/remote CPU and makes the disk row
  // re-render constantly. Disks barely move, so the slow cadence is
  // enough; in between full polls the prior disks/blockDevices are
  // kept in state and rendered as-is.
  async function runProbe(includeDisks: boolean) {
    setBusy(true);
    setError("");
    setNeedsPasswordRecovery(false);
    const started = Date.now();
    const targetLabel = sshTarget
      ? `${sshTarget.user}@${sshTarget.host}:${sshTarget.port} (auth=${sshTarget.authMode}, password=${sshTarget.password ? `len${sshTarget.password.length}` : "none"}, savedIdx=${sshTarget.savedConnectionIndex ?? "-"})`
      : isLocal
        ? "local"
        : "no-connection";
    logEvent(
      "DEBUG",
      "monitor.panel",
      `tab=${tab.id} probe start (${includeDisks ? "full" : "fast"}) → ${targetLabel}`,
    );
    try {
      const s = isLocal
        ? await cmd.localSystemInfo(includeDisks)
        : sshTarget
          ? await cmd.serverMonitorProbe({
              host: sshTarget.host,
              port: sshTarget.port,
              user: sshTarget.user,
              authMode: sshTarget.authMode,
              password: sshTarget.password,
              keyPath: sshTarget.keyPath,
              savedConnectionIndex: sshTarget.savedConnectionIndex,
              includeDisks,
            })
          : null;
      if (!s) {
        setError(t("No connection available."));
        logEvent("WARN", "monitor.panel", `tab=${tab.id} probe → no target`);
        return;
      }
      // Fast probes don't carry disk data; preserve whatever the last
      // full probe wrote so the gauge / table don't blank out between
      // ticks. The first probe after mount is always full, so the
      // first fast one always finds something to merge against.
      setSnap((prev) => {
        if (includeDisks || !prev) return s;
        return {
          ...s,
          diskTotal: prev.diskTotal,
          diskUsed: prev.diskUsed,
          diskAvail: prev.diskAvail,
          diskUsePct: prev.diskUsePct,
          disks: prev.disks,
          blockDevices: prev.blockDevices,
        };
      });
      setLastProbed(Date.now());
      const elapsed = Date.now() - started;
      const degraded =
        s.cpuPct < 0 && s.memTotalMb < 0 && s.procCount === 0;
      logEvent(
        degraded ? "WARN" : "DEBUG",
        "monitor.panel",
        `tab=${tab.id} probe ok (${includeDisks ? "full" : "fast"}) in ${elapsed}ms${degraded ? " (all fields empty — remote output did not parse)" : ""}`,
      );
    } catch (e) {
      // Keep the last good snapshot visible instead of blanking the whole
      // panel — a transient SSH hiccup shouldn't unmount the gauges.
      const msg = formatError(e);
      setError(msg);
      if (isMissingKeychainError(e)) setNeedsPasswordRecovery(true);
      logEvent("ERROR", "monitor.panel", `tab=${tab.id} probe failed: ${msg}`);
    } finally {
      setBusy(false);
    }
  }

  // The recovery button dispatches via the global UI-action bus —
  // App.tsx subscribes to it and opens the saved-connection editor.
  // Going through the bus instead of a prop callback keeps the
  // affordance working no matter which wrapper renders this panel,
  // since props can be silently dropped if a parent forgets to
  // forward them.
  const requestEditConnection = useUiActionsStore((s) => s.requestEditConnection);
  const recoverableSavedIndex = sshTarget?.savedConnectionIndex ?? null;
  const canRecoverPassword =
    needsPasswordRecovery && recoverableSavedIndex !== null;
  const recoverPassword = () => {
    if (!canRecoverPassword || recoverableSavedIndex === null) return;
    requestEditConnection(recoverableSavedIndex);
    onEditConnection?.(recoverableSavedIndex);
  };

  const canProbe = isLocal || hasSsh;

  // Auto-probe + detect when this panel mounts for an SSH or local tab —
  // the component is keyed by tab.id in RightSidebar so this fires on
  // tab switch too. Password-auth saved tabs that haven't primed their
  // password yet will no-op here; user can tap "探测服务器" to retry.
  // Installs a 5-second tick that fires a fast probe (CPU/memory/network
  // /processes); every 6th tick (~30 s) is promoted to a full probe
  // that also runs `df` + `lsblk`. The `busy` guard prevents stacking
  // when a previous probe is still in flight on a slow remote.
  useEffect(() => {
    const haveCreds = isSshTargetReady(sshTarget);
    // For real SSH-backend tabs, hold off the first probe until the
    // terminal session is up. The backend's `terminal_create_ssh_*`
    // call seeds the shared SSH cache as soon as the russh handshake
    // completes; once we wait for it, the probe (and the 5-second
    // polling that follows) reuses that cached session instead of
    // racing the terminal handshake with a parallel one. On the
    // user's LAN this drops "double-click → usable terminal" from
    // several seconds (sshd serializing 3+ concurrent password
    // logins) to roughly one round-trip.
    //
    // Local tabs that mirrored an `ssh user@host` invocation have a
    // local-PTY `terminalSessionId` but the russh session is on the
    // panel side, so we don't need to wait — the first probe primes
    // the cache and subsequent ones reuse.
    const waitingForTerminal =
      tab.backend === "ssh" && tab.terminalSessionId === null;
    const ready = (isLocal || haveCreds) && !waitingForTerminal;
    if (!ready) return;
    // Hidden keep-alive panels must be quiet. Otherwise switching from
    // Monitor to another heavy tool (Docker in particular) fires one
    // extra monitor probe right as the new tool is doing its first load.
    if (!isActive) return;
    // First probe is full so the disk gauges populate immediately;
    // subsequent ticks split into 5 s fast (no disks) and 30 s full.
    void runProbe(true);
    let lastFullAt = Date.now();
    const tick = window.setInterval(() => {
      // Re-read busy from the latest closure via a state check —
      // intentionally letting the JS engine grab the freshest value
      // since `busy` isn't in the deps (we don't want the interval
      // teardown/recreate cycle every time it flips).
      if (busyRef.current) return;
      const now = Date.now();
      // Promote this tick to full if 30 s have elapsed since the last
      // full probe; otherwise just refresh the cheap fields.
      const wantFull = now - lastFullAt >= 30_000;
      if (wantFull) lastFullAt = now;
      void runProbe(wantFull);
    }, 5000);
    return () => window.clearInterval(tick);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab.id,
    tab.backend,
    tab.terminalSessionId !== null,
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    // Re-run once the async password resolution lands:
    (sshTarget?.password.length ?? 0) > 0,
    isActive,
  ]);

  const headerMeta = sshTarget
    ? `${sshTarget.host} · :${sshTarget.port}`
    : isLocal
      ? t("local")
      : "—";
  const connName = sshTarget
    ? `${sshTarget.user}@${sshTarget.host}`
    : isLocal
      ? t("Local Host")
      : t("Server Monitor");
  const connSub = sshTarget
    ? t("Port {port}", { port: sshTarget.port })
    : isLocal
      ? t("Local probe")
      : t("Not connected");
  const connTag = (
    <>
      <StatusDot tone={snap ? "pos" : "off"} />
      {snap ? t("ready") : t("offline")}
    </>
  );

  // Per-mount lookup into the lsblk topology — feeds the DISKS table's
  // TYPE column ("SSD · NVMe" etc.) and the row tooltip's MODEL line.
  // Memoised because rebuilding the map on every paint would be wasted
  // work given the disks list only changes on the 30 s slow tick.
  const mountToBlock = useMemo(
    () => buildMountToBlock(snap?.blockDevices ?? []),
    [snap?.blockDevices],
  );
  const blockTree = useMemo(
    () => buildBlockTree(snap?.blockDevices ?? []),
    [snap?.blockDevices],
  );

  const memPct = snap && snap.memTotalMb > 0 ? (snap.memUsedMb / snap.memTotalMb) * 100 : 0;
  const cpuPct = snap?.cpuPct ?? 0;
  const diskPct = snap && snap.diskUsePct >= 0 ? snap.diskUsePct : 0;
  const netRate = snap ? formatRate(snap.netRxBps + snap.netTxBps) : null;
  // Cap the network gauge at 100MB/s for the bar fill — pure cosmetic
  // ceiling, the readout itself shows the actual rate.
  const netPct = snap && snap.netRxBps >= 0 && snap.netTxBps >= 0
    ? Math.min(100, ((snap.netRxBps + snap.netTxBps) / (100 * 1024 * 1024)) * 100)
    : 0;
  const rxRate = snap ? formatRate(snap.netRxBps) : null;
  const txRate = snap ? formatRate(snap.netTxBps) : null;

  // Pick the matching list for the active sort. Fall back from
  // topProcessesMem to topProcesses when the backend didn't emit a
  // MEM slice (older cached snapshot, or a remote whose `ps` doesn't
  // accept `--sort=-pmem`) so the user still sees something useful
  // rather than an empty table.
  const procRows = useMemo(() => {
    if (!snap) return [];
    if (procSort === "mem") {
      return snap.topProcessesMem.length > 0 ? snap.topProcessesMem : snap.topProcesses;
    }
    return snap.topProcesses;
  }, [snap, procSort]);

  return (
    <>
      <DbConnRow
        icon={MONITOR_ICON}
        tint="var(--pos-dim)"
        iconTint="var(--pos)"
        name={connName}
        sub={connSub}
        tag={connTag}
      />
      <div className="panel-scroll">
      {/*
        Always-visible monitor section: chrome (host bar + gauges + probe
        button row) renders immediately so clicking the Monitor tool
        never flashes a blank panel. When snapshot is null we render
        placeholder "—" values; the four Gauge shells stay in place and
        fill in when probe() lands.
      */}
      <section className="mon">
        <div className="mon-host">
          <div className="mon-host-top">
            <StatusDot tone={snap ? "pos" : "off"} />
            <div className="mon-host-name">{connName}</div>
            <span className="mono mon-host-uptime">
              {snap ? `${t("uptime")} ${snap.uptime}` : t("not yet probed")}
            </span>
          </div>
          <div className="mon-host-meta mono">
            {snap?.osLabel || headerMeta}
            {snap && snap.load1 >= 0 ? (
              <> · {t("load")} {snap.load1.toFixed(2)} / {snap.load5.toFixed(2)} / {snap.load15.toFixed(2)}</>
            ) : null}
          </div>
        </div>

        <div className="mon-grid">
          <Gauge
            icon={<Cpu size={10} />}
            label={t("CPU")}
            value={snap ? <>{cpuPct.toFixed(1)}<span className="mon-gauge-unit">%</span></> : <>—</>}
            sub={snap && snap.load1 >= 0
              ? `${t("load")} ${snap.load1.toFixed(2)} · ${snap.load5.toFixed(2)} · ${snap.load15.toFixed(2)}`
              : "—"}
            pct={snap ? cpuPct : 0}
            tone={snap ? toneFromPct(cpuPct) : "off"}
          />
          <Gauge
            icon={<MemoryStick size={10} />}
            label={t("MEMORY")}
            value={snap ? <>{memPct.toFixed(0)}<span className="mon-gauge-unit">%</span></> : <>—</>}
            sub={snap
              ? `${(snap.memUsedMb / 1024).toFixed(1)} / ${(snap.memTotalMb / 1024).toFixed(1)} GB`
              : "—"}
            pct={snap ? memPct : 0}
            tone={snap ? toneFromPct(memPct) : "off"}
          />
          <Gauge
            icon={<HardDrive size={10} />}
            label={t("DISK")}
            value={snap
              ? <>{snap.diskUsePct >= 0 ? snap.diskUsePct.toFixed(0) : "—"}<span className="mon-gauge-unit">%</span></>
              : <>—</>}
            sub={snap
              ? snap.disks && snap.disks.length > 1
                ? `${snap.diskAvail} ${t("free of")} ${snap.diskTotal} · ${t("{count} mounts", { count: snap.disks.length })}`
                : `${snap.diskAvail} ${t("free of")} ${snap.diskTotal}`
              : "—"}
            pct={snap ? diskPct : 0}
            tone={snap ? toneFromPct(diskPct) : "off"}
          />
          <Gauge
            icon={<Network size={10} />}
            label={t("NETWORK")}
            value={netRate ? <>{netRate.value}<span className="mon-gauge-unit"> {netRate.unit}</span></> : <>—</>}
            sub={rxRate && txRate
              ? `↓ ${rxRate.value} ${rxRate.unit} · ↑ ${txRate.value} ${txRate.unit}`
              : t("warming up...")}
            pct={netPct}
            tone={netRate ? "pos" : "off"}
          />
        </div>

        {/*
          System-stats strip — pier-x-copy reference shows vCPU /
          total RAM / total disk / process count as compact pills
          underneath the gauges. Each pill stays as "—" until the
          backend probe fills the corresponding field, so the chrome
          doesn't shift after the first probe lands.
        */}
        <div className="mon-strip">
          <span className="mon-pill">
            <Cpu size={10} />
            {snap && snap.cpuCount > 0 ? `${snap.cpuCount} vCPU` : "—"}
          </span>
          <span className="mon-pill">
            <MemoryStick size={10} />
            {snap && snap.memTotalMb > 0
              ? `${(snap.memTotalMb / 1024).toFixed(1)} GB`
              : "—"}
          </span>
          <span className="mon-pill">
            <HardDrive size={10} />
            {snap?.diskTotal || "—"}
          </span>
          <span className="mon-pill">
            <Network size={10} />
            {snap && snap.procCount > 0
              ? t("{count} procs", { count: snap.procCount })
              : "—"}
          </span>
        </div>

        {/*
          Per-filesystem disk breakdown — populated from `df -hPT`.
          Pseudo / docker-managed mounts are filtered on the backend
          so space numbers stay honest (no overlay double-counting).
        */}
        <div className="mon-block">
          <div className="mon-block-head">
            <span>{t("DISKS")}</span>
            <span className="mono mon-block-meta">{t("df -h")}</span>
          </div>
          <table className="mon-table mon-table--disks">
            <thead>
              <tr>
                <th>{t("MOUNT")}</th>
                <th style={{ width: 64, textAlign: "left" }}>{t("TYPE")}</th>
                <th style={{ width: 48, textAlign: "right" }}>{t("SIZE")}</th>
                <th style={{ width: 48, textAlign: "right" }}>{t("USED")}</th>
                <th style={{ width: 48, textAlign: "right" }}>{t("AVAIL")}</th>
                <th style={{ width: 40, textAlign: "right" }}>{t("USE%")}</th>
              </tr>
            </thead>
            <tbody>
              {snap && snap.disks && snap.disks.length > 0 ? (
                snap.disks.map((disk, i) => {
                  const pct = disk.usePct >= 0 ? disk.usePct.toFixed(0) : "—";
                  const toneCls = disk.usePct >= 85
                    ? "mon-cell-warn"
                    : disk.usePct >= 50
                      ? ""
                      : "mon-cell-muted";
                  // Resolve TYPE / MODEL from the matching lsblk row.
                  // Empty when lsblk wasn't available — column shows "—".
                  const block = mountToBlock.get(disk.mountpoint);
                  const typeLabel = describeBlock(block);
                  const modelHint = block?.model ? ` · ${block.model}` : "";
                  const rowTitle = `${disk.filesystem}${disk.fsType ? ` (${disk.fsType})` : ""} → ${disk.mountpoint}${modelHint}`;
                  return (
                    <tr key={`${disk.mountpoint}-${i}`} title={rowTitle}>
                      <td className="mono mon-cell-trunc">{disk.mountpoint}</td>
                      <td className="mono mon-cell-type">{typeLabel ?? "—"}</td>
                      <td className="mono mon-cell-right">{disk.total}</td>
                      <td className="mono mon-cell-right">{disk.used}</td>
                      <td className="mono mon-cell-right">{disk.avail}</td>
                      <td className={`mono mon-cell-right ${toneCls}`}>{pct}%</td>
                    </tr>
                  );
                })
              ) : (
                <tr>
                  <td colSpan={6} className="mon-empty mono">
                    {snap ? t("(no disk data)") : "—"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {/*
          Top processes table. Backend runs `ps -eo …` twice per
          probe — once with `--sort=-pcpu` and once with
          `--sort=-pmem` — so the MEM toggle surfaces real memory
          hogs (low-CPU DB/browser heaps) rather than a client-side
          re-sort of the CPU list.

          Sits directly below the DISKS table — the everyday
          "what's eating my server" pair stays adjacent, and the
          BLOCK DEVICES infra readout (which changes far less
          often) drops to the bottom of the panel.

          Layout is dense by design: the right panel is narrow so
          `table-layout: fixed` + ellipsis on the COMMAND column
          keep everything on one row. The `TIME` / etime column
          used to live here but was always clipped to two digits
          in practice; the elapsed value is still available via
          the row tooltip.
        */}
        <div className="mon-block">
          <div className="mon-block-head">
            <span>{t("TOP PROCESSES")}</span>
            <div className="mon-block-meta mon-sort-group mono">
              <span>{t("Sort:")}</span>
              <button
                type="button"
                className={"dk-sort" + (procSort === "cpu" ? " active" : "")}
                onClick={() => setProcSort("cpu")}
              >
                {t("CPU")}
              </button>
              <button
                type="button"
                className={"dk-sort" + (procSort === "mem" ? " active" : "")}
                onClick={() => setProcSort("mem")}
              >
                {t("MEM")}
              </button>
            </div>
          </div>
          <table className="mon-table mon-table--procs">
            <thead>
              <tr>
                <th style={{ width: 54 }}>{t("PID")}</th>
                <th>{t("COMMAND")}</th>
                <th style={{ width: 48, textAlign: "right" }}>{t("CPU%")}</th>
                <th style={{ width: 48, textAlign: "right" }}>{t("MEM%")}</th>
              </tr>
            </thead>
            <tbody>
              {snap && procRows.length > 0 ? (
                procRows.map((row, i) => (
                  <tr
                    key={`${row.pid}-${i}`}
                    title={`${row.command} · PID ${row.pid} · ${t("elapsed")} ${row.elapsed}`}
                  >
                    <td className="mono mon-cell-muted">{row.pid}</td>
                    <td className="mono mon-cell-trunc">{row.command}</td>
                    <td className="mono mon-cell-right">{row.cpuPct}</td>
                    <td className="mono mon-cell-right">{row.memPct}</td>
                  </tr>
                ))
              ) : (
                <tr>
                  <td colSpan={4} className="mon-empty mono">
                    {snap ? t("(no process data)") : "—"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {/*
          Block-device topology — shown only when the remote returned an
          lsblk readout (Linux with util-linux). Renders the disk →
          part → crypt → lv tree so the user can see physical disks
          (including unmounted ones), media type (SSD vs HDD), bus
          (NVMe / SATA / virtio / USB), and the model string. Hidden
          on macOS local probes and on BusyBox-only remotes where lsblk
          isn't installed.
        */}
        {snap && snap.blockDevices && snap.blockDevices.length > 0 && (
          <div className="mon-block">
            <div className="mon-block-head">
              <span>{t("BLOCK DEVICES")}</span>
              <span className="mono mon-block-meta">{t("lsblk")}</span>
            </div>
            <ul className="mon-tree">
              {blockTree.map((node) => (
                <BlockTreeRow key={node.kname} node={node} depth={0} />
              ))}
            </ul>
          </div>
        )}

        <div className="mon-actions">
          <button
            type="button"
            className="btn is-ghost is-compact"
            disabled={!canProbe || busy}
            onClick={() => void runProbe(true)}
          >
            <RefreshCw size={11} /> {busy ? t("Probing...") : snap ? t("Probe now") : t("Probe Server")}
          </button>
          <span className="mono mon-actions-hint">
            {!canProbe
              ? t("No connection available.")
              : lastProbed
                ? `${t("last")}: ${formatTimestamp(lastProbed)}`
                : t("not yet probed")}
          </span>
        </div>
        {error && (
          <DismissibleNote variant="status" tone="error" onDismiss={() => setError("")}>
            <span>{error}</span>
            {canRecoverPassword && (
              <button
                type="button"
                className="mini-button"
                onClick={recoverPassword}
              >
                <KeyRound size={11} /> {t("Re-enter password")}
              </button>
            )}
          </DismissibleNote>
        )}
      </section>
    </div>
    </>
  );
}
