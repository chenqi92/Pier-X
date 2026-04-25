import {
  Activity,
  ArrowDown,
  ArrowUp,
  Filter,
  Globe,
  Plug,
  RefreshCw,
  Send,
  Shield,
  ShieldAlert,
  ShieldCheck,
  Square as Block,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import * as cmd from "../lib/commands";
import type {
  FirewallBackend,
  FirewallSnapshotView,
  FirewallInterfaceCounter,
  TabState,
} from "../lib/types";
import { effectiveSshTarget } from "../lib/types";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import DbConnRow from "../components/DbConnRow";
import DismissibleNote from "../components/DismissibleNote";
import PanelHeader from "../components/PanelHeader";
import StatusDot from "../components/StatusDot";

type Props = {
  tab: TabState;
  /** True when this panel is the visible right-side tool. The 2-second
   *  Traffic poll only runs while active so background keep-alive
   *  instances don't stream `/proc/net/dev` over SSH for hidden tabs. */
  isActive?: boolean;
};

type FwTab = "listening" | "rules" | "mappings" | "traffic";

/** A non-empty `iptables -t nat -S DOCKER` line shape we expand into a
 *  port mapping. Example raw: `-A DOCKER ! -i br-abc -p tcp -m tcp
 *  --dport 8080 -j DNAT --to-destination 172.17.0.2:80`. */
type PortMapping = {
  proto: string;
  externalPort: number;
  internalAddr: string;
  internalPort: number;
  /** Source chain — `DOCKER` for compose/Docker mappings, `PREROUTING`
   *  for hand-rolled DNAT. Helps users tell their own rules apart from
   *  Docker's auto-generated ones. */
  chain: string;
  raw: string;
};

const TRAFFIC_POLL_MS = 2000;
const RATE_HISTORY_LEN = 60; // 60 samples × 2s = 2 min sparkline

function backendLabel(backend: FirewallBackend): string {
  switch (backend) {
    case "firewalld": return "firewalld";
    case "ufw": return "ufw";
    case "nftables": return "nftables";
    case "iptables": return "iptables";
    default: return "—";
  }
}

function backendIcon(backend: FirewallBackend, active: boolean) {
  if (!active) return <ShieldAlert size={11} />;
  if (backend === "none") return <Shield size={11} />;
  return <ShieldCheck size={11} />;
}

function formatBps(bps: number): string {
  if (!Number.isFinite(bps) || bps < 0) return "—";
  if (bps >= 1024 * 1024) return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

/** Diff two snapshots' interface byte counters into a per-iface byte/sec
 *  rate. Counters reset when an interface goes down/up; we treat any
 *  negative delta as zero rather than panicking with a wrap-around. */
function computeRates(
  prev: FirewallInterfaceCounter[],
  prevAt: number,
  cur: FirewallInterfaceCounter[],
  curAt: number,
): Record<string, { rxBps: number; txBps: number }> {
  const dt = (curAt - prevAt) / 1000;
  if (dt <= 0) return {};
  const prevById = new Map(prev.map((p) => [p.iface, p]));
  const out: Record<string, { rxBps: number; txBps: number }> = {};
  for (const c of cur) {
    const p = prevById.get(c.iface);
    if (!p) continue;
    const dRx = c.rxBytes - p.rxBytes;
    const dTx = c.txBytes - p.txBytes;
    out[c.iface] = {
      rxBps: dRx > 0 ? dRx / dt : 0,
      txBps: dTx > 0 ? dTx / dt : 0,
    };
  }
  return out;
}

/** Pull `-A DOCKER ... -j DNAT --to-destination IP:PORT` and equivalent
 *  PREROUTING DNATs out of the raw nat-table dump. Best-effort regex —
 *  iptables-save formatting is stable enough that we don't need a real
 *  parser, and any line we can't cleanly destructure just gets shown
 *  in the Rules tab instead. */
function parseMappings(natDump: string): PortMapping[] {
  const out: PortMapping[] = [];
  for (const line of natDump.split("\n")) {
    if (!line.startsWith("-A DOCKER") && !line.startsWith("-A PREROUTING")) continue;
    if (!line.includes("DNAT")) continue;
    const proto = /\s-p\s+(tcp|udp)/.exec(line)?.[1] ?? "";
    const dport = /--dport\s+(\d+)/.exec(line)?.[1];
    const dest = /--to-destination\s+([\d.:]+)/.exec(line)?.[1];
    if (!proto || !dport || !dest) continue;
    const [internalAddr, internalPort] = dest.split(":");
    out.push({
      proto,
      externalPort: parseInt(dport, 10),
      internalAddr,
      internalPort: parseInt(internalPort ?? "0", 10),
      chain: line.startsWith("-A DOCKER") ? "DOCKER" : "PREROUTING",
      raw: line,
    });
  }
  return out;
}

/** Build the right write command for the detected backend. We always
 *  prefix with `sudo` when the SSH user isn't root — the panel sends
 *  it to the terminal where the user can edit and supply a password. */
function buildOpenPortCmd(
  backend: FirewallBackend,
  proto: "tcp" | "udp",
  port: number,
  needsSudo: boolean,
): string {
  const sudo = needsSudo ? "sudo " : "";
  switch (backend) {
    case "ufw":
      return `${sudo}ufw allow ${port}/${proto}`;
    case "firewalld":
      return `${sudo}firewall-cmd --permanent --add-port=${port}/${proto} && ${sudo}firewall-cmd --reload`;
    case "nftables":
      return `${sudo}nft add rule inet filter input ${proto} dport ${port} accept`;
    default:
      return `${sudo}iptables -I INPUT -p ${proto} --dport ${port} -j ACCEPT`;
  }
}

function buildBlockPortCmd(
  backend: FirewallBackend,
  proto: "tcp" | "udp",
  port: number,
  needsSudo: boolean,
): string {
  const sudo = needsSudo ? "sudo " : "";
  switch (backend) {
    case "ufw":
      return `${sudo}ufw deny ${port}/${proto}`;
    case "firewalld":
      return `${sudo}firewall-cmd --permanent --remove-port=${port}/${proto} && ${sudo}firewall-cmd --reload`;
    case "nftables":
      return `${sudo}nft add rule inet filter input ${proto} dport ${port} drop`;
    default:
      return `${sudo}iptables -I INPUT -p ${proto} --dport ${port} -j DROP`;
  }
}

export default function FirewallPanel({ tab, isActive = true }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);

  const sshTarget = effectiveSshTarget(tab);
  const hasSsh = sshTarget !== null;
  const canRefresh = hasSsh;
  const terminalSessionId = tab.terminalSessionId;

  const [snap, setSnap] = useState<FirewallSnapshotView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [activeTab, setActiveTab] = useState<FwTab>("listening");
  const [search, setSearch] = useState("");
  const [composerPort, setComposerPort] = useState("");
  const [composerProto, setComposerProto] = useState<"tcp" | "udp">("tcp");
  // Confirmation modal for any send-to-terminal action. The user reviews
  // the exact command, edits if they want, and presses Enter themselves
  // in the actual terminal — the panel never executes anything itself.
  const [pendingCmd, setPendingCmd] = useState<{ cmd: string; description: string } | null>(null);

  const [rates, setRates] = useState<Record<string, { rxBps: number; txBps: number }>>({});
  const [rateHistory, setRateHistory] = useState<Record<string, { rx: number[]; tx: number[] }>>({});
  const lastSnapRef = useRef<FirewallSnapshotView | null>(null);

  const busyRef = useRef(false);
  busyRef.current = busy;

  async function probe(): Promise<FirewallSnapshotView | null> {
    if (!canRefresh || !sshTarget) return null;
    try {
      const s = await cmd.firewallSnapshot({
        host: sshTarget.host,
        port: sshTarget.port,
        user: sshTarget.user,
        authMode: sshTarget.authMode,
        password: sshTarget.password,
        keyPath: sshTarget.keyPath,
        savedConnectionIndex: sshTarget.savedConnectionIndex,
      });
      const prev = lastSnapRef.current;
      if (prev && prev.capturedAtMs > 0 && s.capturedAtMs > prev.capturedAtMs) {
        const next = computeRates(prev.interfaces, prev.capturedAtMs, s.interfaces, s.capturedAtMs);
        setRates(next);
        setRateHistory((prevHist) => {
          const out = { ...prevHist };
          for (const iface of Object.keys(next)) {
            const cur = out[iface] ?? { rx: [], tx: [] };
            const rx = [...cur.rx, next[iface].rxBps].slice(-RATE_HISTORY_LEN);
            const tx = [...cur.tx, next[iface].txBps].slice(-RATE_HISTORY_LEN);
            out[iface] = { rx, tx };
          }
          return out;
        });
      }
      lastSnapRef.current = s;
      setSnap(s);
      setError("");
      return s;
    } catch (e) {
      setError(formatError(e));
      return null;
    }
  }

  useEffect(() => {
    if (!canRefresh) return;
    setBusy(true);
    void probe().finally(() => setBusy(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab.id, canRefresh]);

  // Traffic-only 2s polling. Other tabs stay on the cached snapshot
  // until the user hits Refresh; firewall rules don't change every
  // 2 seconds, but interface counters do.
  useEffect(() => {
    if (!isActive || activeTab !== "traffic" || !canRefresh) return;
    const id = window.setInterval(() => {
      if (!busyRef.current) void probe();
    }, TRAFFIC_POLL_MS);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isActive, activeTab, canRefresh, tab.id]);

  async function refreshNow() {
    if (busy) return;
    setBusy(true);
    try {
      await probe();
    } finally {
      setBusy(false);
    }
  }

  function sendToTerminal(cmdText: string, description: string) {
    if (!terminalSessionId) {
      setError(t("This tab has no terminal session — open the terminal once before running firewall actions."));
      return;
    }
    setPendingCmd({ cmd: cmdText, description });
  }

  async function confirmSendToTerminal() {
    if (!pendingCmd || !terminalSessionId) return;
    try {
      // Trailing space, no newline — the user must press Enter
      // themselves in the terminal. That's what makes "走终端的通道"
      // work: sudo prompts handle themselves, the user can edit the
      // command, and there's no password handling in the panel.
      await cmd.terminalWrite(terminalSessionId, pendingCmd.cmd + " ");
      setPendingCmd(null);
    } catch (e) {
      setError(formatError(e));
    }
  }

  const backend = snap?.backend ?? "none";
  const backendActive = snap?.backendActive ?? false;
  const isRoot = snap?.root ?? false;
  const needsSudo = !isRoot;

  const filteredListening = useMemo(() => {
    const list = snap?.listening ?? [];
    const q = search.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (p) =>
        String(p.localPort).includes(q) ||
        p.process.toLowerCase().includes(q) ||
        p.proto.toLowerCase().includes(q) ||
        p.localAddr.toLowerCase().includes(q),
    );
  }, [snap, search]);

  const mappings = useMemo(() => parseMappings(snap?.natV4 ?? ""), [snap?.natV4]);

  const filteredMappings = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return mappings;
    return mappings.filter(
      (m) =>
        String(m.externalPort).includes(q) ||
        m.internalAddr.includes(q) ||
        String(m.internalPort).includes(q) ||
        m.chain.toLowerCase().includes(q),
    );
  }, [mappings, search]);

  const headerMeta = sshTarget ? `${sshTarget.user}@${sshTarget.host}:${sshTarget.port}` : "";
  const hostLabel = headerMeta || t("No connection");
  const hostSub = snap
    ? `${backendLabel(backend)}${backendActive ? "" : ` · ${t("inactive")}`} · ${snap.user || "?"}${isRoot ? " (root)" : ""}`
    : t("not yet probed");

  return (
    <>
      <PanelHeader icon={Shield} title={t("Firewall")} meta={headerMeta} />
      <div className="dk fw">
        <DbConnRow
          icon={Shield}
          tint="var(--svc-firewall)"
          iconTint="var(--svc-firewall)"
          name={hostLabel}
          sub={hostSub}
          tag={
            <>
              <StatusDot tone={snap && backendActive ? "pos" : "off"} />
              {backendIcon(backend, backendActive)}
              {backendLabel(backend)}
            </>
          }
        />

        {!canRefresh && <div className="lg-note">{t("SSH connection required for Firewall.")}</div>}

        {!terminalSessionId && canRefresh && (
          <div className="lg-note">
            {t("Open the terminal at least once for this tab to enable write actions.")}
          </div>
        )}

        {error && (
          <DismissibleNote tone="error" onDismiss={() => setError("")}>
            {error}
          </DismissibleNote>
        )}

        <div className="dk-tabs">
          {(["listening", "rules", "mappings", "traffic"] as FwTab[]).map((k) => (
            <button
              key={k}
              type="button"
              className={"dk-tab" + (activeTab === k ? " active" : "")}
              onClick={() => setActiveTab(k)}
            >
              {t(k.charAt(0).toUpperCase() + k.slice(1))}
            </button>
          ))}
        </div>

        <div className="dk-primary">
          {(activeTab === "listening" || activeTab === "mappings") && (
            <div className="dk-search">
              <Filter size={10} />
              <input
                placeholder={
                  activeTab === "listening"
                    ? t("Filter by port, process…")
                    : t("Filter by external or internal port…")
                }
                value={search}
                onChange={(e) => setSearch(e.currentTarget.value)}
              />
              {search && (
                <button className="lg-x" type="button" onClick={() => setSearch("")}>
                  <X size={10} />
                </button>
              )}
            </div>
          )}
          <div style={{ flex: 1 }} />
          <button
            className="dk-ic"
            type="button"
            title={t("Refresh")}
            disabled={!canRefresh || busy}
            onClick={() => void refreshNow()}
          >
            <RefreshCw size={11} className={busy ? "spin" : ""} />
          </button>
        </div>

        {activeTab === "listening" && (
          <div className="dk-body">
            <div className="dk-toolbar fw-composer">
              <input
                className="fw-port-input mono"
                type="number"
                placeholder={t("Port")}
                value={composerPort}
                onChange={(e) => setComposerPort(e.currentTarget.value)}
                min={1}
                max={65535}
              />
              <select
                className="fw-proto"
                value={composerProto}
                onChange={(e) => setComposerProto(e.currentTarget.value as "tcp" | "udp")}
              >
                <option value="tcp">TCP</option>
                <option value="udp">UDP</option>
              </select>
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={!composerPort || !terminalSessionId}
                onClick={() => {
                  const port = parseInt(composerPort, 10);
                  if (!port) return;
                  sendToTerminal(
                    buildOpenPortCmd(backend, composerProto, port, needsSudo),
                    t("Open port {port}/{proto}", { port: String(port), proto: composerProto }),
                  );
                }}
              >
                <Plug size={11} /> {t("Allow")}
              </button>
              <button
                type="button"
                className="btn is-compact"
                disabled={!composerPort || !terminalSessionId}
                onClick={() => {
                  const port = parseInt(composerPort, 10);
                  if (!port) return;
                  sendToTerminal(
                    buildBlockPortCmd(backend, composerProto, port, needsSudo),
                    t("Block port {port}/{proto}", { port: String(port), proto: composerProto }),
                  );
                }}
              >
                <Block size={11} /> {t("Deny")}
              </button>
              <div style={{ flex: 1 }} />
              <span className="mono text-muted" style={{ fontSize: "var(--size-micro)" }}>
                {t("{count} open", { count: filteredListening.length })}
              </span>
            </div>
            <div className="dk-card-list">
              {filteredListening.length === 0 ? (
                <div className="dk-empty">
                  {snap ? t("No listening sockets visible.") : t("Loading…")}
                </div>
              ) : (
                filteredListening.map((p, i) => (
                  <div key={`${p.proto}-${p.localAddr}-${p.localPort}-${i}`} className="dk-card">
                    <span className="dk-card-ic is-pos">
                      <Globe size={12} />
                    </span>
                    <div className="dk-card-body">
                      <div className="dk-card-title mono">
                        {p.localAddr}:{p.localPort}
                        <span className="text-muted"> · {p.proto}</span>
                      </div>
                      <div className="dk-card-sub mono">
                        {p.process || t("(unknown — root needed)")}
                        {p.pid !== null ? ` · pid ${p.pid}` : ""}
                      </div>
                    </div>
                    <div className="dk-card-actions" onClick={(e) => e.stopPropagation()}>
                      <button
                        className="mini-btn is-destructive"
                        type="button"
                        title={t("Send block command to terminal")}
                        disabled={!terminalSessionId}
                        onClick={() =>
                          sendToTerminal(
                            buildBlockPortCmd(
                              backend,
                              p.proto.startsWith("udp") ? "udp" : "tcp",
                              p.localPort,
                              needsSudo,
                            ),
                            t("Block port {port}/{proto}", {
                              port: String(p.localPort),
                              proto: p.proto.startsWith("udp") ? "udp" : "tcp",
                            }),
                          )
                        }
                      >
                        <Block size={10} />
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        )}

        {activeTab === "rules" && (
          <div className="dk-body">
            <div className="fw-policies mono">
              {Object.entries(snap?.defaultPolicies ?? {}).map(([chain, policy]) => (
                <span
                  key={chain}
                  className={"db-badge " + (policy === "DROP" ? "is-pos" : policy === "REJECT" ? "is-warn" : "is-muted")}
                  title={t("Default policy for chain {chain}", { chain })}
                >
                  {chain}: {policy}
                </span>
              ))}
              {(!snap?.defaultPolicies || Object.keys(snap.defaultPolicies).length === 0) && (
                <span className="text-muted">
                  {snap ? t("No default policies — root may be needed to read iptables.") : t("Loading…")}
                </span>
              )}
            </div>
            <pre className="fw-pre mono">
              {snap?.rulesV4 || (snap ? t("(empty — try refreshing as root)") : t("Loading…"))}
            </pre>
            {snap?.rulesV6 ? (
              <>
                <div className="fw-section-title mono">{t("IPv6")}</div>
                <pre className="fw-pre mono">{snap.rulesV6}</pre>
              </>
            ) : null}
          </div>
        )}

        {activeTab === "mappings" && (
          <div className="dk-body">
            <div className="dk-card-list">
              {filteredMappings.length === 0 ? (
                <div className="dk-empty">
                  {snap ? t("No DNAT / port mappings detected.") : t("Loading…")}
                </div>
              ) : (
                filteredMappings.map((m, i) => (
                  <div key={`${m.externalPort}-${m.internalAddr}-${i}`} className="dk-card">
                    <span className="dk-card-ic is-pos">
                      <Send size={12} />
                    </span>
                    <div className="dk-card-body">
                      <div className="dk-card-title mono">
                        :{m.externalPort}/{m.proto} → {m.internalAddr}:{m.internalPort}
                      </div>
                      <div className="dk-card-sub mono">
                        <span className={"db-badge " + (m.chain === "DOCKER" ? "is-info" : "is-muted")}>
                          {m.chain}
                        </span>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
            <div className="fw-hint mono">
              {t("DOCKER chain rules are auto-managed by the Docker daemon. Edit container port maps via the Docker panel instead of removing them here.")}
            </div>
          </div>
        )}

        {activeTab === "traffic" && (
          <div className="dk-body">
            <div className="dk-card-list">
              {(snap?.interfaces ?? []).length === 0 ? (
                <div className="dk-empty">
                  {snap ? t("No interfaces detected.") : t("Loading…")}
                </div>
              ) : (
                (snap?.interfaces ?? []).map((iface) => {
                  const r = rates[iface.iface];
                  const hist = rateHistory[iface.iface];
                  return (
                    <div key={iface.iface} className="fw-iface">
                      <div className="fw-iface-head">
                        <span className="mono">
                          <Activity size={11} /> {iface.iface}
                        </span>
                        <span className="mono fw-iface-rates">
                          <ArrowDown size={10} /> {formatBps(r?.rxBps ?? -1)}
                          {"  "}
                          <ArrowUp size={10} /> {formatBps(r?.txBps ?? -1)}
                        </span>
                      </div>
                      <Sparkline values={hist?.rx ?? []} stroke="var(--info)" />
                      <Sparkline values={hist?.tx ?? []} stroke="var(--warn)" />
                    </div>
                  );
                })
              )}
            </div>
            <div className="fw-hint mono">
              {t("Sampling /proc/net/dev every 2 s while this tab is visible. Loopback is hidden.")}
            </div>
          </div>
        )}
      </div>

      {pendingCmd && (
        <div className="fw-confirm-scrim" onClick={() => setPendingCmd(null)}>
          <div className="fw-confirm" onClick={(e) => e.stopPropagation()}>
            <div className="fw-confirm-title">{pendingCmd.description}</div>
            <div className="fw-confirm-body mono">{pendingCmd.cmd}</div>
            <div className="fw-confirm-hint">
              {t("Inserted into your terminal — you press Enter to execute. Sudo prompt (if any) handles itself.")}
            </div>
            <div className="fw-confirm-actions">
              <button type="button" className="btn is-compact" onClick={() => setPendingCmd(null)}>
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="btn is-primary is-compact"
                onClick={() => void confirmSendToTerminal()}
              >
                <Send size={11} /> {t("Send to terminal")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

function Sparkline({ values, stroke }: { values: number[]; stroke: string }) {
  if (values.length < 2) {
    return <div className="fw-spark fw-spark-empty" />;
  }
  const max = Math.max(...values, 1);
  const w = 200;
  const h = 24;
  const step = w / (values.length - 1);
  const points = values
    .map((v, i) => `${(i * step).toFixed(1)},${(h - (v / max) * h).toFixed(1)}`)
    .join(" ");
  return (
    <svg className="fw-spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <polyline points={points} fill="none" stroke={stroke} strokeWidth={1.2} />
    </svg>
  );
}
