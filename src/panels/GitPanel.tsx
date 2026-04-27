import {
  Archive,
  ArrowDown,
  ArrowDownCircle,
  ArrowRight,
  ArrowUp,
  ArrowUpCircle,
  Calendar,
  Check,
  ChevronDown,
  Copy,
  Download,
  FileText,
  Folder,
  GitBranch,
  GitMerge,
  History,
  Layers,
  Minus,
  Network,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Tag,
  Trash2,
  Upload,
  User,
  X,
} from "lucide-react";
import type { ComponentType, CSSProperties, MouseEvent as ReactMouseEvent, MutableRefObject, ReactNode, UIEvent as ReactUIEvent } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { memo, startTransition, useCallback, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isBrowsableRepoPath } from "../lib/browserPath";
import * as cmd from "../lib/commands";
import type { GitReflogEntry } from "../lib/commands";
import { writeClipboardText } from "../lib/clipboard";
import Dialog from "../components/Dialog";
import DiffDialog, { type DiffFileInput } from "../shell/DiffDialog";
import "../styles/git-panel.css";
import type {
  GitBlameLineView,
  GitCommitDetailView,
  GitComparisonFileView,
  GitConfigEntryView,
  GitConflictFileView,
  GitConflictHunkView,
  GitGraphMetadata,
  GitGraphRowView,
  GitPanelState,
  GitRebaseItemView,
  GitRebasePlanView,
  GitRemoteView,
  GitStashEntry,
  GitSubmoduleView,
  GitTagView,
  GitUnpushedCommit,
} from "../lib/types";
import { localizeError } from "../i18n/localizeMessage";
import { useI18n, type I18nValue } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";

type Props = {
  browserPath: string;
  /** True when this panel is the currently-selected right-side tool AND the
   *  right column is expanded. Drives background polling so hidden panels
   *  don't burn IPC on `git_panel_state` every 3s. */
  isActive?: boolean;
};

type PanelTab = "changes" | "history" | "stash" | "conflicts";
type PopoverKind =
  | "branchMenu"
  | "historyOptions"
  | "historyBranchFilter"
  | "historyAuthorFilter"
  | "historyDateFilter"
  | "historyPathFilter"
  | "changeFileMenu"
  | "historyCommit"
  | "branchManager"
  | "tagManager"
  | "remoteManager"
  | "configManager"
  | "rebaseManager"
  | "submoduleManager"
  | "stashMenu"
  | "unpushedCommits";

type DiffTarget =
  | { kind: "working"; path: string; staged: boolean; untracked: boolean }
  | null;

type PopoverState = {
  kind: PopoverKind;
  left: number;
  top: number;
  width: number;
  // Computed at open-time from the trigger's viewport position so the
  // popover's internal scroller (overflow:auto in atoms.css .popover)
  // never overflows the bottom edge.
  maxHeight: number;
  data?: unknown;
} | null;

type BannerState = { success: boolean; message: string } | null;
type ButtonTone = "ghost" | "primary" | "destructive";
type PillTone = "success" | "warning" | "error" | "info" | "neutral";
type RepoPathTreeNode = {
  id: string;
  kind: "directory" | "file";
  name: string;
  path: string;
  children: RepoPathTreeNode[];
};
type ChangeFileMenuState = {
  file: GitPanelState["stagedFiles"][number];
  staged: boolean;
};

const GRAPH_PALETTE = [
  "var(--status-success)",
  "var(--accent)",
  "var(--warn)",
  "var(--info)",
  "var(--status-error)",
  "var(--accent-hover)",
  "var(--mod)",
  "var(--neg)",
];

function extractErrorMessage(error: unknown, t: I18nValue["t"]) {
  return localizeError(error, t);
}

function repoNameFromPath(path: string) {
  const normalized = path.replace(/[\\/]+$/, "");
  if (!normalized) return "Git";
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || "Git";
}

function parentPathLabel(path: string) {
  const value = String(path || "");
  const index = value.lastIndexOf("/");
  return index > 0 ? value.slice(0, index) : "";
}

function pathAncestors(path: string) {
  const parts = String(path || "")
    .split("/")
    .filter(Boolean);
  const ancestors: string[] = [];
  let current = "";
  for (let index = 0; index < parts.length - 1; index += 1) {
    current = current ? `${current}/${parts[index]}` : parts[index];
    ancestors.push(current);
  }
  return ancestors;
}

function buildRepoPathTree(paths: string[]) {
  const root: RepoPathTreeNode[] = [];

  for (const rawPath of paths) {
    const parts = String(rawPath || "")
      .split("/")
      .filter(Boolean);
    if (!parts.length) continue;

    let currentChildren = root;
    let currentPath = "";

    parts.forEach((part, index) => {
      currentPath = currentPath ? `${currentPath}/${part}` : part;
      const isLeaf = index === parts.length - 1;
      let node = currentChildren.find((candidate) => candidate.name === part);

      if (!node) {
        node = {
          id: `${isLeaf ? "file" : "dir"}:${currentPath}`,
          kind: isLeaf ? "file" : "directory",
          name: part,
          path: currentPath,
          children: [],
        };
        currentChildren.push(node);
      } else if (!isLeaf) {
        node.kind = "directory";
      }

      currentChildren = node.children;
    });
  }

  const sortNodes = (nodes: RepoPathTreeNode[]) => {
    nodes.sort((left, right) => {
      if (left.kind !== right.kind) return left.kind === "directory" ? -1 : 1;
      return left.name.localeCompare(right.name);
    });
    nodes.forEach((node) => sortNodes(node.children));
    return nodes;
  };

  return sortNodes(root);
}

function workingFileKey(path: string, staged: boolean) {
  return (staged ? "S|" : "W|") + path;
}

function gitFileEqual(a: GitPanelState["stagedFiles"][number], b: GitPanelState["stagedFiles"][number]): boolean {
  return (
    a.path === b.path &&
    a.fileName === b.fileName &&
    a.status === b.status &&
    a.staged === b.staged &&
    a.additions === b.additions &&
    a.deletions === b.deletions
  );
}

function panelStateEqual(a: GitPanelState, b: GitPanelState): boolean {
  if (
    a.repoPath !== b.repoPath ||
    a.currentBranch !== b.currentBranch ||
    a.trackingBranch !== b.trackingBranch ||
    a.aheadCount !== b.aheadCount ||
    a.behindCount !== b.behindCount ||
    a.totalChanges !== b.totalChanges ||
    a.conflictCount !== b.conflictCount ||
    a.workingTreeClean !== b.workingTreeClean ||
    a.stagedFiles.length !== b.stagedFiles.length ||
    a.unstagedFiles.length !== b.unstagedFiles.length
  ) {
    return false;
  }
  for (let i = 0; i < a.stagedFiles.length; i++) {
    if (!gitFileEqual(a.stagedFiles[i], b.stagedFiles[i])) return false;
  }
  for (let i = 0; i < a.unstagedFiles.length; i++) {
    if (!gitFileEqual(a.unstagedFiles[i], b.unstagedFiles[i])) return false;
  }
  return true;
}

function workingDiffStatusFromLetter(code: string): DiffFileInput["status"] {
  const value = String(code || "").trim().toUpperCase();
  if (value === "A") return "added";
  if (value === "D") return "deleted";
  if (value === "R") return "renamed";
  if (value === "?" || value === "??") return "untracked";
  return "modified";
}

function filterRepoPathTree(nodes: RepoPathTreeNode[], needle: string): RepoPathTreeNode[] {
  const query = needle.trim().toLowerCase();
  if (!query) return nodes;

  const visit = (node: RepoPathTreeNode): RepoPathTreeNode | null => {
    const children = node.children.map(visit).filter(Boolean) as RepoPathTreeNode[];
    const matched = node.name.toLowerCase().includes(query) || node.path.toLowerCase().includes(query);
    if (!matched && !children.length) return null;
    return { ...node, children };
  };

  return nodes.map(visit).filter(Boolean) as RepoPathTreeNode[];
}

function defaultExpandedHistoryPaths(paths: string[], selection: string[]) {
  const expanded = new Set<string>();
  for (const path of paths) {
    const firstSlash = path.indexOf("/");
    if (firstSlash > 0) expanded.add(path.slice(0, firstSlash));
  }
  for (const selectedPath of selection) {
    for (const ancestor of pathAncestors(selectedPath)) expanded.add(ancestor);
  }
  return Array.from(expanded);
}

function countRepoPathLeaves(node: RepoPathTreeNode): number {
  if (node.kind === "file" || !node.children.length) return 1;
  return node.children.reduce((sum, child) => sum + countRepoPathLeaves(child), 0);
}

function refTokens(rawRefs: string) {
  return String(rawRefs || "")
    .replace(/^\s*\(/, "")
    .replace(/\)\s*$/, "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function formatGraphDate(timestamp: number) {
  if (!timestamp) return "";
  const date = new Date(timestamp * 1000);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}`;
}

function authorInitial(author: string) {
  const trimmed = String(author || "").trim();
  if (!trimmed) return "?";
  const first = trimmed[0];
  return first.toUpperCase();
}

function authorColor(author: string) {
  const value = String(author || "");
  let hash = 0;
  for (let i = 0; i < value.length; i += 1) hash = (hash * 31 + value.charCodeAt(i)) | 0;
  const hue = Math.abs(hash * 37) % 360;
  return `hsl(${hue} 55% 45%)`;
}

function statusToneFromCode(code: string): PillTone {
  switch (code) {
    case "A":
      return "success";
    case "D":
      return "error";
    case "U":
      return "warning";
    case "M":
    case "R":
    case "C":
      return "info";
    default:
      return "neutral";
  }
}

function graphColor(index: number) {
  return GRAPH_PALETTE[Math.abs(index || 0) % GRAPH_PALETTE.length] || "var(--accent)";
}

function refBadgeToneClass(token: string) {
  if (token.startsWith("HEAD")) return "git-ref-badge--head";
  if (token.startsWith("tag:")) return "git-ref-badge--tag";
  if (token.includes("/")) return "git-ref-badge--remote";
  return "git-ref-badge--local";
}

function historyRowIsMerge(row: GitGraphRowView | null | undefined) {
  const parents = String(row?.parents || "").trim();
  return parents.length > 0 && parents.split(/\s+/).length > 1;
}

function normalizeRemoteBaseUrl(url: string) {
  const raw = String(url || "").trim();
  if (!raw) return "";
  if (raw.startsWith("git@")) {
    const match = raw.match(/^git@([^:]+):(.+?)(?:\.git)?$/);
    if (match) return `https://${match[1]}/${match[2]}`;
  }
  if (raw.startsWith("ssh://git@")) {
    return raw.replace(/^ssh:\/\/git@/, "https://").replace(/:(\d+)\//, "/").replace(/\.git$/, "");
  }
  if (raw.startsWith("http://") || raw.startsWith("https://")) {
    return raw.replace(/\.git$/, "");
  }
  return "";
}

function diffLineTone(line: string) {
  if (line.startsWith("+++") || line.startsWith("---")) return "meta";
  if (line.startsWith("@@")) return "accent";
  if (line.startsWith("+")) return "added";
  if (line.startsWith("-")) return "removed";
  return "plain";
}

function isLocalBranch(name: string) {
  return !String(name || "").includes("/");
}

function GitPill({ tone, children }: { tone: PillTone; children: ReactNode }) {
  return <span className={`git-pill git-pill--${tone}`}>{children}</span>;
}

function GitFileDelta({ additions, deletions }: { additions: number; deletions: number }) {
  if (!additions && !deletions) return null;
  return (
    <span className="git-file-row__delta mono">
      {additions ? <span className="git-file-row__delta-add">+{additions}</span> : null}
      {deletions ? <span className="git-file-row__delta-del">−{deletions}</span> : null}
    </span>
  );
}

// Adapter over the shared `.btn` / `.icon-btn` atoms so existing call
// sites keep using <GitButton tone="primary" compact /> while the
// rendered chrome matches the rest of the app.
function GitButton({
  tone = "ghost",
  compact = false,
  className = "",
  children,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  tone?: ButtonTone;
  compact?: boolean;
}) {
  const toneClass =
    tone === "primary" ? "is-primary" : tone === "destructive" ? "is-danger" : "is-ghost";
  const classes = ["btn", toneClass, compact ? "is-compact" : "", className]
    .filter(Boolean)
    .join(" ");
  return (
    <button {...props} className={classes} type={props.type ?? "button"}>
      {children}
    </button>
  );
}

function GitIconButton({
  icon: Icon,
  active = false,
  className = "",
  title,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  icon: ComponentType<{ size?: number; className?: string; strokeWidth?: number }>;
  active?: boolean;
}) {
  const tooltip = title ?? (props["aria-label"] as string | undefined);
  return (
    <button
      {...props}
      title={tooltip}
      className={["icon-btn", active ? "is-active" : "", className].filter(Boolean).join(" ")}
      type={props.type ?? "button"}
    >
      <Icon size={14} strokeWidth={2} />
    </button>
  );
}

function GitSectionHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="git-section-header">
      <div className="git-section-header__copy">
        <div className="git-section-header__title">{title}</div>
        {subtitle ? <div className="git-section-header__subtitle">{subtitle}</div> : null}
      </div>
      {actions ? <div className="git-section-header__actions">{actions}</div> : null}
    </div>
  );
}

function GitEmptyState({
  icon: Icon,
  title,
  description,
  accent = "var(--accent)",
  action,
}: {
  icon: ComponentType<{ size?: number; className?: string; strokeWidth?: number }>;
  title: string;
  description: string;
  accent?: string;
  action?: ReactNode;
}) {
  return (
    <div className="git-empty">
      <div className="git-empty__icon" style={{ "--git-accent": accent } as CSSProperties}>
        <Icon size={16} />
      </div>
      <div className="git-empty__title">{title}</div>
      <div className="git-empty__description">{description}</div>
      {action ? <div className="git-empty__action">{action}</div> : null}
    </div>
  );
}

// Thin adapter over the shared `.popover` chrome (atoms.css). Kept as a
// component so callers can keep their `<GitPopover kind="...">` markup,
// but the visual is now identical to other panels' popovers.
function GitPopover({
  popover,
  kind,
  onClose,
  children,
}: {
  popover: PopoverState;
  kind: PopoverKind;
  onClose: () => void;
  children: ReactNode;
}) {
  if (!popover || popover.kind !== kind) return null;
  return (
    <div className="popover-layer" onMouseDown={onClose}>
      <div
        className="popover"
        onMouseDown={(event) => event.stopPropagation()}
        style={{
          left: popover.left,
          top: popover.top,
          width: popover.width,
          maxHeight: popover.maxHeight,
        }}
      >
        {children}
      </div>
    </div>
  );
}

// Thin adapter over the shared <Dialog/> primitive so existing call sites
// keep `<GitDialog wide tall>` while the chrome (head/body/foot) follows
// the project-wide `.dlg-*` styles.
function GitDialog({
  open,
  title,
  subtitle,
  wide = false,
  tall = false,
  onClose,
  children,
  footer,
}: {
  open: boolean;
  title: string;
  subtitle?: string;
  wide?: boolean;
  tall?: boolean;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <Dialog
      open={open}
      title={title}
      subtitle={subtitle}
      size={wide ? "lg" : "md"}
      tall={tall}
      onClose={onClose}
      footer={footer}
    >
      {children}
    </Dialog>
  );
}

// Graph geometry — matches Pier's IDEA-style renderer.
const GRAPH_LANE_PX = 14;
const GRAPH_ROW_H = 24;
const GRAPH_DOT_R = 3.5;
const GRAPH_LANE_MIN_W = 56;
const GIT_STATUS_POLL_MS = 10_000;

function GitGraphLane({ row, isHead, width }: { row: GitGraphRowView; isHead: boolean; width: number }) {
  const dotColor = graphColor(row.colorIndex);
  // Half-pixel offset so the 2px stroke straddles a pixel boundary cleanly
  // (avoids WebKit sub-pixel anti-aliasing turning the vertical line into a
  // washed-out 3-pixel band — the visual gap that made Pier-X's graph look
  // "dotted" compared to Pier's Canvas-rendered IDEA-style lines).
  const cx = row.nodeColumn * GRAPH_LANE_PX + GRAPH_LANE_PX / 2 + 4;
  const cy = GRAPH_ROW_H / 2;
  return (
    <svg
      className="git-graph-lane"
      width={width}
      height={GRAPH_ROW_H}
      viewBox={`0 0 ${width} ${GRAPH_ROW_H}`}
      shapeRendering="geometricPrecision"
      aria-hidden="true"
    >
      {row.segments.map((segment, index) => {
        const isVertical = Math.abs(segment.xTop - segment.xBottom) < 0.5;
        // Vertical lines: snap x to half-pixel + use crispEdges so the
        // 2px stroke renders as a sharp 2-pixel column instead of a fuzzy
        // 3-pixel band. Diagonals: keep geometricPrecision (set on the
        // root <svg>) for smooth anti-aliased arcs.
        const x1 = isVertical ? Math.round(segment.xTop) + 0.5 : segment.xTop;
        const x2 = isVertical ? Math.round(segment.xBottom) + 0.5 : segment.xBottom;
        return (
          <line
            key={`${row.hash}-segment-${index}`}
            x1={x1}
            y1={segment.yTop}
            x2={x2}
            y2={segment.yBottom}
            stroke={graphColor(segment.colorIndex)}
            strokeWidth="2"
            strokeLinecap="butt"
            shapeRendering={isVertical ? "crispEdges" : undefined}
          />
        );
      })}
      {row.arrows.map((arrow, index) => {
        const arrowColor = graphColor(arrow.colorIndex);
        const armLen = 5;
        const halfW = 4;
        const points = arrow.isDown
          ? `${arrow.x - halfW},${arrow.y - armLen} ${arrow.x},${arrow.y} ${arrow.x + halfW},${arrow.y - armLen}`
          : `${arrow.x - halfW},${arrow.y + armLen} ${arrow.x},${arrow.y} ${arrow.x + halfW},${arrow.y + armLen}`;
        return (
          <polyline
            key={`${row.hash}-arrow-${index}`}
            points={points}
            fill="none"
            stroke={arrowColor}
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        );
      })}
      {isHead ? (
        <circle cx={cx} cy={cy} r={GRAPH_DOT_R + 2} fill="none" stroke={dotColor} strokeWidth="1.5" />
      ) : null}
      <circle cx={cx} cy={cy} r={GRAPH_DOT_R} fill={dotColor} />
    </svg>
  );
}

// Mirrors Pier (which uses 500). Smaller values cut more long-spanning edges
// across page boundaries — e.g. main → old-main edges that pass through a
// merged-in side branch — and lose the IDEA-style ↓/↑ chevrons + diagonal
// converge segments that mark the transition. 500 keeps almost all real-world
// histories in a single page.
const HISTORY_PAGE_SIZE = 500;

// Column resizer for the history table. Drives a CSS variable on the
// surface element so the header, all rows, and the resizer guide stay
// in sync without forcing React to re-render every row.
function ColResizer({
  surfaceRef,
  cssVar,
  initial,
  min,
  max,
  variant,
  inline = false,
  onPersist,
}: {
  surfaceRef: MutableRefObject<HTMLElement | null>;
  cssVar: string;
  initial: number;
  min: number;
  max: number;
  variant: "author" | "date" | "hash";
  inline?: boolean;
  onPersist: (next: number) => void;
}) {
  const [active, setActive] = useState(false);
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.stopPropagation();
    const surface = surfaceRef.current;
    if (!surface) return;
    const current =
      parseFloat(getComputedStyle(surface).getPropertyValue(cssVar)) || initial;
    dragRef.current = { startX: event.clientX, startWidth: current };
    setActive(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const move = (e: PointerEvent) => {
      const drag = dragRef.current;
      const surf = surfaceRef.current;
      if (!drag || !surf) return;
      // Resizer sits at the RIGHT edge of the column it controls, so dragging
      // right = wider, dragging left = narrower (intuitive grab-the-handle).
      const dx = e.clientX - drag.startX;
      const next = Math.max(min, Math.min(max, drag.startWidth + dx));
      surf.style.setProperty(cssVar, `${Math.round(next)}px`);
    };

    const up = () => {
      const surf = surfaceRef.current;
      if (surf) {
        const final = parseFloat(getComputedStyle(surf).getPropertyValue(cssVar));
        if (Number.isFinite(final)) onPersist(Math.round(final));
      }
      dragRef.current = null;
      setActive(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <div
      className={`git-col-resizer git-col-resizer--${variant}${inline ? " git-col-resizer--inline" : ""}${active ? " is-active" : ""}`}
      onPointerDown={onPointerDown}
      role="separator"
      aria-orientation="vertical"
    />
  );
}

// Click target whose children remain text-selectable. A native <button> would
// block selection (per the global chrome rule). Mousedown position is recorded
// so a drag-to-select gesture doesn't accidentally fire `onActivate`.
function SelectableFileRow({
  onActivate,
  title,
  children,
}: {
  onActivate: () => void;
  title?: string;
  children: ReactNode;
}) {
  const downRef = useRef<{ x: number; y: number } | null>(null);
  return (
    <div
      role="button"
      tabIndex={0}
      className="git-history-inline__file"
      title={title}
      onMouseDown={(e) => {
        downRef.current = { x: e.clientX, y: e.clientY };
      }}
      onClick={(e) => {
        const down = downRef.current;
        downRef.current = null;
        if (down && (Math.abs(e.clientX - down.x) > 4 || Math.abs(e.clientY - down.y) > 4)) {
          return;
        }
        onActivate();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onActivate();
        }
      }}
    >
      {children}
    </div>
  );
}

type GitHistoryVirtualListProps = {
  rows: GitGraphRowView[];
  rowHeight: number;
  selectedIndex: number;
  detailNode: ReactNode | null;
  renderRow: (row: GitGraphRowView, index: number) => ReactNode;
  footer?: ReactNode;
  overscan?: number;
  className?: string;
  style?: CSSProperties;
  scrollRef?: MutableRefObject<HTMLDivElement | null>;
  onScroll?: (event: ReactUIEvent<HTMLDivElement>) => void;
};

// Fixed-height virtualizer that knows about a single inline-expanded row.
// Rows are rowHeight tall; the selected row reserves an extra measured
// detailHeight slot directly below it. An optional footer sits at the very
// bottom. DOM cost scales with viewport, not rows.length.
function GitHistoryVirtualList({
  rows,
  rowHeight,
  selectedIndex,
  detailNode,
  renderRow,
  footer,
  overscan = 12,
  className,
  style,
  scrollRef,
  onScroll,
}: GitHistoryVirtualListProps) {
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [detailHeight, setDetailHeight] = useState(0);
  const [footerHeight, setFooterHeight] = useState(0);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const update = () => setViewportHeight(el.clientHeight);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const detailObserverRef = useRef<ResizeObserver | null>(null);
  const setDetailRef = useCallback((node: HTMLDivElement | null) => {
    if (detailObserverRef.current) {
      detailObserverRef.current.disconnect();
      detailObserverRef.current = null;
    }
    if (!node) {
      setDetailHeight(0);
      return;
    }
    const measure = () => setDetailHeight(node.getBoundingClientRect().height);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(node);
    detailObserverRef.current = ro;
  }, []);

  const footerObserverRef = useRef<ResizeObserver | null>(null);
  const setFooterRef = useCallback((node: HTMLDivElement | null) => {
    if (footerObserverRef.current) {
      footerObserverRef.current.disconnect();
      footerObserverRef.current = null;
    }
    if (!node) {
      setFooterHeight(0);
      return;
    }
    const measure = () => setFooterHeight(node.getBoundingClientRect().height);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(node);
    footerObserverRef.current = ro;
  }, []);

  const hasDetail =
    selectedIndex >= 0 && selectedIndex < rows.length && detailNode != null;
  const effectiveDetailH = hasDetail ? detailHeight : 0;
  const rowsTotal = rows.length * rowHeight + effectiveDetailH;
  const totalHeight = rowsTotal + footerHeight;

  const yOfRow = (i: number): number => {
    if (!hasDetail || i <= selectedIndex) return i * rowHeight;
    return i * rowHeight + effectiveDetailH;
  };
  const rowAtY = (y: number): number => {
    if (!hasDetail) return Math.floor(y / rowHeight);
    const b1 = (selectedIndex + 1) * rowHeight;
    const b2 = b1 + effectiveDetailH;
    if (y < b1) return Math.floor(y / rowHeight);
    if (y < b2) return selectedIndex;
    return selectedIndex + 1 + Math.floor((y - b2) / rowHeight);
  };

  const firstIdx = Math.max(0, rowAtY(scrollTop) - overscan);
  const lastIdx = Math.min(
    rows.length,
    rowAtY(scrollTop + viewportHeight) + overscan + 1,
  );

  const setScrollerRef = (node: HTMLDivElement | null) => {
    scrollerRef.current = node;
    if (scrollRef) scrollRef.current = node;
  };

  return (
    <div
      ref={setScrollerRef}
      className={className}
      style={{
        overflow: "auto",
        position: "relative",
        // Promote the scroll container to its own compositor layer up front
        // so the first scroll doesn't pay a "layerize on demand" latency hit.
        willChange: "transform",
        ...style,
      }}
      onScroll={(event) => {
        setScrollTop(event.currentTarget.scrollTop);
        onScroll?.(event);
      }}
    >
      <div style={{ height: totalHeight, position: "relative" }}>
        {rows.slice(firstIdx, lastIdx).map((row, i) => {
          const idx = firstIdx + i;
          return (
            <div
              key={row.hash}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                height: rowHeight,
                // Use compositor-only transform instead of `top: y` so
                // scrolling re-uses cached paint layers per row and doesn't
                // re-lay out the row tree on every frame.
                transform: `translate3d(0,${yOfRow(idx)}px,0)`,
                contain: "content",
              }}
            >
              {renderRow(row, idx)}
            </div>
          );
        })}
        {hasDetail ? (
          <div
            ref={setDetailRef}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              right: 0,
              transform: `translate3d(0,${(selectedIndex + 1) * rowHeight}px,0)`,
              contain: "content",
            }}
          >
            {detailNode}
          </div>
        ) : null}
        {footer ? (
          <div
            ref={setFooterRef}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              right: 0,
              transform: `translate3d(0,${rowsTotal}px,0)`,
            }}
          >
            {footer}
          </div>
        ) : null}
      </div>
    </div>
  );
}

type HistoryRowProps = {
  row: GitGraphRowView;
  active: boolean;
  dimmed: boolean;
  zebraStripe: boolean;
  refs: string[];
  isHead: boolean;
  graphLaneWidth: number;
  authorColorValue: string;
  authorInitialValue: string;
  formattedDate: string;
  showAuthor: boolean;
  showDate: boolean;
  showHash: boolean;
  titleText: string;
  surfaceRef: MutableRefObject<HTMLElement | null>;
  colWidthsRef: MutableRefObject<HistoryColWidths>;
  onPersistCol: (field: keyof HistoryColWidths, value: number) => void;
  onSelect: (hash: string, wasActive: boolean) => void;
  onDoubleClick: (hash: string) => void;
  onContextMenu: (event: ReactMouseEvent<HTMLButtonElement>, row: GitGraphRowView) => void;
};

const HistoryRow = memo(function HistoryRow({
  row,
  active,
  dimmed,
  zebraStripe,
  refs,
  isHead,
  graphLaneWidth,
  authorColorValue,
  authorInitialValue,
  formattedDate,
  showAuthor,
  showDate,
  showHash,
  titleText,
  surfaceRef,
  colWidthsRef,
  onPersistCol,
  onSelect,
  onDoubleClick,
  onContextMenu,
}: HistoryRowProps) {
  return (
    <div
      className={[
        "git-history-entry",
        active ? "git-history-entry--active" : "",
        dimmed ? "git-history-entry--dimmed" : "",
        zebraStripe ? "git-history-entry--zebra" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <button
        className={[
          "git-history-row",
          active ? "git-history-row--active" : "",
          dimmed ? "git-history-row--dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        onClick={() => onSelect(row.hash, active)}
        onDoubleClick={() => onDoubleClick(row.hash)}
        onContextMenu={(event) => onContextMenu(event, row)}
        type="button"
        title={titleText}
      >
        <GitGraphLane row={row} isHead={isHead} width={graphLaneWidth} />
        <div className="git-history-row__content">
          <div className="git-history-row__subject">
            {refs.slice(0, 3).map((token) => (
              <span key={`${row.hash}-${token}`} className={["git-ref-badge", refBadgeToneClass(token)].join(" ")}>
                {token}
              </span>
            ))}
            {refs.length > 3 ? <span className="git-history-row__more">{`+${refs.length - 3}`}</span> : null}
            <span className="git-history-row__message">{row.message}</span>
          </div>
          {showAuthor ? (
            <span className="git-history-row__author" title={row.author}>
              <span
                className="git-history-row__avatar"
                style={{ background: authorColorValue }}
                aria-hidden="true"
              >
                {authorInitialValue}
              </span>
              <span className="git-history-row__author-name">{row.author}</span>
            </span>
          ) : null}
          {showAuthor && (showDate || showHash) ? (
            <ColResizer
              cssVar="--col-author-w"
              initial={colWidthsRef.current.author}
              max={HISTORY_COL_MAX.author}
              min={HISTORY_COL_MIN.author}
              onPersist={(value) => onPersistCol("author", value)}
              surfaceRef={surfaceRef}
              variant="author"
              inline
            />
          ) : null}
          {showDate ? (
            <span className="git-history-row__date" title={formattedDate}>{formattedDate}</span>
          ) : null}
          {showDate && showHash ? (
            <ColResizer
              cssVar="--col-date-w"
              initial={colWidthsRef.current.date}
              max={HISTORY_COL_MAX.date}
              min={HISTORY_COL_MIN.date}
              onPersist={(value) => onPersistCol("date", value)}
              surfaceRef={surfaceRef}
              variant="date"
              inline
            />
          ) : null}
          {showHash ? (
            <span className="git-history-row__hash" title={row.shortHash}>{row.shortHash}</span>
          ) : null}
        </div>
      </button>
    </div>
  );
});

function GitDiffCode({ text }: { text: string }) {
  const allLines = text.split("\n");
  const limit = 1600;
  const lines = allLines.slice(0, limit);
  const truncated = allLines.length > limit;
  return (
    <pre className="git-diff-code">
      {lines.map((line, index) => (
        <div key={`${index}-${line}`} className={`git-diff-code__line git-diff-code__line--${diffLineTone(line)}`}>
          {line || " "}
        </div>
      ))}
      {truncated ? (
        <div className="git-diff-code__line git-diff-code__line--meta">
          {`... diff preview truncated at ${limit} lines`}
        </div>
      ) : null}
    </pre>
  );
}

function GitMenuItem({
  active = false,
  destructive = false,
  checkable = false,
  children,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  destructive?: boolean;
  /** Render a check on the right when `active`. Used by toggle / radio
   *  groups (history options) so the active state is communicated by a
   *  glyph instead of a heavy filled background. */
  checkable?: boolean;
}) {
  return (
    <button
      {...props}
      className={[
        "popover-item",
        checkable ? "popover-item--checkable" : "",
        active ? "is-active" : "",
        destructive ? "is-destructive" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      type={props.type ?? "button"}
    >
      <span className="popover-item__label">{children}</span>
      {checkable ? (
        <span className="popover-item__check" aria-hidden="true">
          {active ? <Check size={12} /> : null}
        </span>
      ) : null}
    </button>
  );
}

const CHANGES_LAYOUT_STORAGE_KEY = "pierx.git.changes.layout.v1";

function readChangesLayout(): { staged: number; working: number; commit: number } | undefined {
  try {
    const raw = localStorage.getItem(CHANGES_LAYOUT_STORAGE_KEY);
    if (!raw) return undefined;
    const parsed = JSON.parse(raw) as Record<string, number>;
    if (
      typeof parsed.staged === "number" &&
      typeof parsed.working === "number" &&
      typeof parsed.commit === "number"
    ) {
      return { staged: parsed.staged, working: parsed.working, commit: parsed.commit };
    }
    return undefined;
  } catch {
    return undefined;
  }
}

const HISTORY_COLS_STORAGE_KEY = "pierx.git.history.cols.v1";

type HistoryColWidths = { author: number; date: number; hash: number };
const HISTORY_COL_DEFAULTS: HistoryColWidths = { author: 96, date: 120, hash: 64 };
const HISTORY_COL_MIN: HistoryColWidths = { author: 60, date: 80, hash: 50 };
const HISTORY_COL_MAX: HistoryColWidths = { author: 240, date: 220, hash: 110 };

function readHistoryColWidths(): HistoryColWidths {
  try {
    const raw = localStorage.getItem(HISTORY_COLS_STORAGE_KEY);
    if (!raw) return HISTORY_COL_DEFAULTS;
    const parsed = JSON.parse(raw) as Partial<HistoryColWidths>;
    return {
      author:
        typeof parsed.author === "number" ? parsed.author : HISTORY_COL_DEFAULTS.author,
      date: typeof parsed.date === "number" ? parsed.date : HISTORY_COL_DEFAULTS.date,
      hash: typeof parsed.hash === "number" ? parsed.hash : HISTORY_COL_DEFAULTS.hash,
    };
  } catch {
    return HISTORY_COL_DEFAULTS;
  }
}

function persistHistoryColWidths(widths: HistoryColWidths) {
  try {
    localStorage.setItem(HISTORY_COLS_STORAGE_KEY, JSON.stringify(widths));
  } catch {
    /* ignore */
  }
}

export default function GitPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <GitPanelBody {...props} /> : <PanelSkeleton variant="rows" rows={10} />}
    </div>
  );
}

function GitPanelBody({ browserPath, isActive = true }: Props) {
  const { t } = useI18n();
  const panelRef = useRef<HTMLDivElement>(null);

  const [panelState, setPanelState] = useState<GitPanelState | null>(null);
  const changesLayout = useMemo(() => readChangesLayout(), []);
  const persistChangesLayout = useCallback((layout: Record<string, number>) => {
    try {
      localStorage.setItem(CHANGES_LAYOUT_STORAGE_KEY, JSON.stringify(layout));
    } catch {
      /* ignore quota / privacy errors */
    }
  }, []);

  const historyListSurfaceRef = useRef<HTMLElement | null>(null);
  const historyColWidthsRef = useRef<HistoryColWidths>(readHistoryColWidths());
  const persistHistoryCol = useCallback((field: keyof HistoryColWidths, value: number) => {
    historyColWidthsRef.current = { ...historyColWidthsRef.current, [field]: value };
    persistHistoryColWidths(historyColWidthsRef.current);
  }, []);
  const setGitStatus = useStatusStore((s) => s.setGitStatus);
  const clearGitStatus = useStatusStore((s) => s.clearGitStatus);
  const [gitReady, setGitReady] = useState(false);
  const [gitError, setGitError] = useState("");
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<BannerState>(null);
  const [selectedTab, setSelectedTab] = useState<PanelTab>("changes");
  // Branch manager is now a popover (kind="branchManager"); previously
  // it was a full Dialog with a backdrop, which felt out-of-place next
  // to the other anchored toolbar popovers.
  const [branchMenuBranches, setBranchMenuBranches] = useState<string[]>([]);
  const [graphMetadata, setGraphMetadata] = useState<GitGraphMetadata | null>(null);
  const [graphRows, setGraphRows] = useState<GitGraphRowView[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyLoadingMore, setHistoryLoadingMore] = useState(false);
  const [historyHasMore, setHistoryHasMore] = useState(false);
  const historyListRef = useRef<HTMLDivElement | null>(null);
  const [historySearchText, setHistorySearchText] = useState("");
  const [historyBranchFilter, setHistoryBranchFilter] = useState("");
  const [historyAuthorFilter, setHistoryAuthorFilter] = useState("");
  const [historyDateFilter, setHistoryDateFilter] = useState("all");
  const [historyPaths, setHistoryPaths] = useState<string[]>([]);
  const [historySortMode, setHistorySortMode] = useState<"topo" | "date">("topo");
  const [historyFirstParent, setHistoryFirstParent] = useState(false);
  const [historyNoMerges, setHistoryNoMerges] = useState(false);
  const [historyShowLongEdges, setHistoryShowLongEdges] = useState(true);
  const [historyShowZebraStripes, setHistoryShowZebraStripes] = useState(true);
  const [historyShowHash, setHistoryShowHash] = useState(true);
  const [historyShowAuthor, setHistoryShowAuthor] = useState(true);
  const [historyShowDate, setHistoryShowDate] = useState(true);
  const [historyHighlightMode, setHistoryHighlightMode] = useState<"none" | "mine" | "merge" | "branch">("none");
  const [historySelectedHash, setHistorySelectedHash] = useState("");
  const [historyContextCommit, setHistoryContextCommit] = useState<GitGraphRowView | null>(null);
  const [historyPathSearchText, setHistoryPathSearchText] = useState("");
  const [historyPathSelection, setHistoryPathSelection] = useState<string[]>([]);
  // historyPathSelectionState is called for every node on every render of
  // the path tree. Keeping the linear scans inside it (`includes` +
  // `some(startsWith)`) makes selection state O(n·m). Precompute a direct-
  // match set and an ancestor-directory set once per selection change so
  // each lookup is O(1).
  const historyPathSelectionSet = useMemo(
    () => new Set(historyPathSelection),
    [historyPathSelection],
  );
  const historyPathAncestorSet = useMemo(() => {
    const out = new Set<string>();
    for (const p of historyPathSelection) {
      let idx = p.lastIndexOf("/");
      while (idx > 0) {
        const prefix = p.slice(0, idx);
        if (out.has(prefix)) break;
        out.add(prefix);
        idx = p.lastIndexOf("/", idx - 1);
      }
    }
    return out;
  }, [historyPathSelection]);
  const [historyPathExpanded, setHistoryPathExpanded] = useState<string[]>([]);
  const [historyBranchDialogOpen, setHistoryBranchDialogOpen] = useState(false);
  const [historyTagDialogOpen, setHistoryTagDialogOpen] = useState(false);
  const [historyResetDialogOpen, setHistoryResetDialogOpen] = useState(false);
  const [historyEditDialogOpen, setHistoryEditDialogOpen] = useState(false);
  const [historyDropDialogOpen, setHistoryDropDialogOpen] = useState(false);
  const [historyCompareDialogOpen, setHistoryCompareDialogOpen] = useState(false);
  const [reflogDialogOpen, setReflogDialogOpen] = useState(false);
  const [reflogEntries, setReflogEntries] = useState<GitReflogEntry[]>([]);
  const [reflogLoading, setReflogLoading] = useState(false);
  const [reflogError, setReflogError] = useState("");
  const [historyBranchDraftName, setHistoryBranchDraftName] = useState("");
  const [historyTagDraftName, setHistoryTagDraftName] = useState("");
  const [historyTagDraftMessage, setHistoryTagDraftMessage] = useState("");
  const [historyResetMode, setHistoryResetMode] = useState<"soft" | "mixed" | "hard">("mixed");
  const [historyAmendMessage, setHistoryAmendMessage] = useState("");
  const [commitDetail, setCommitDetail] = useState<GitCommitDetailView | null>(null);
  const [comparisonBaseHash, setComparisonBaseHash] = useState("");
  const [comparisonFiles, setComparisonFiles] = useState<GitComparisonFileView[]>([]);
  const [comparisonSelectedPath, setComparisonSelectedPath] = useState("");
  const [comparisonDiff, setComparisonDiff] = useState("");
  const [comparisonExpandedPaths, setComparisonExpandedPaths] = useState<string[]>([]);
  const [branchManagerMode, setBranchManagerMode] = useState<"local" | "remote">("local");
  const [branchManagerSearchText, setBranchManagerSearchText] = useState("");
  const [branchCreateExpanded, setBranchCreateExpanded] = useState(false);
  const [branchDraftName, setBranchDraftName] = useState("");
  const [branchRenameSource, setBranchRenameSource] = useState("");
  const [branchRenameTarget, setBranchRenameTarget] = useState("");
  const [trackingBranchTarget, setTrackingBranchTarget] = useState("");
  const [trackingUpstreamTarget, setTrackingUpstreamTarget] = useState("");
  const [tags, setTags] = useState<GitTagView[]>([]);
  const [tagCreateExpanded, setTagCreateExpanded] = useState(false);
  const [tagDraftName, setTagDraftName] = useState("");
  const [tagDraftMessage, setTagDraftMessage] = useState("");
  const [tagSearchText, setTagSearchText] = useState("");
  const [remotes, setRemotes] = useState<GitRemoteView[]>([]);
  const [remoteComposerExpanded, setRemoteComposerExpanded] = useState(false);
  const [remoteDraftName, setRemoteDraftName] = useState("");
  const [remoteDraftUrl, setRemoteDraftUrl] = useState("");
  const [remoteEditSourceName, setRemoteEditSourceName] = useState("");
  const [remoteSearchText, setRemoteSearchText] = useState("");
  const [configEntries, setConfigEntries] = useState<GitConfigEntryView[]>([]);
  const [configDraftKey, setConfigDraftKey] = useState("");
  const [configDraftValue, setConfigDraftValue] = useState("");
  const [configDraftGlobal, setConfigDraftGlobal] = useState(false);
  // Inline-edit state for the config row: tracks which key is currently
  // in edit mode plus the draft value. Composer at the top is reserved
  // for adding *new* keys; existing keys edit in place.
  const [configEditingKey, setConfigEditingKey] = useState<string | null>(null);
  const [configEditingDraft, setConfigEditingDraft] = useState("");
  const [configSelectedGlobal, setConfigSelectedGlobal] = useState(false);
  const [configSearchText, setConfigSearchText] = useState("");
  const [configComposerExpanded, setConfigComposerExpanded] = useState(false);
  const [rebasePlan, setRebasePlan] = useState<GitRebasePlanView>({ inProgress: false, items: [] });
  const [rebaseCommitCount, setRebaseCommitCount] = useState(10);
  const [rebaseDraftItems, setRebaseDraftItems] = useState<GitRebaseItemView[]>([]);
  const [submodules, setSubmodules] = useState<GitSubmoduleView[]>([]);
  const [submoduleSearchText, setSubmoduleSearchText] = useState("");
  const [stashes, setStashes] = useState<GitStashEntry[]>([]);
  const [conflicts, setConflicts] = useState<GitConflictFileView[]>([]);
  const [conflictDrafts, setConflictDrafts] = useState<Record<string, GitConflictHunkView[]>>({});
  const [selectedConflictPath, setSelectedConflictPath] = useState("");
  const [blameDialogOpen, setBlameDialogOpen] = useState(false);
  const [blameFilePath, setBlameFilePath] = useState("");
  const [blameLines, setBlameLines] = useState<GitBlameLineView[]>([]);
  const [diffTarget, setDiffTarget] = useState<DiffTarget>(null);
  const [workingDiffOpen, setWorkingDiffOpen] = useState(false);
  const [workingDiffCache, setWorkingDiffCache] = useState<Record<string, string | null>>({});
  const [commitAmend, setCommitAmend] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [commitDiffOpen, setCommitDiffOpen] = useState(false);
  const [commitDiffHash, setCommitDiffHash] = useState("");
  const [commitDiffActivePath, setCommitDiffActivePath] = useState("");
  const [commitDiffCache, setCommitDiffCache] = useState<Record<string, string | null>>({});
  const [stashMessage, setStashMessage] = useState("");
  const [stashEditingIndex, setStashEditingIndex] = useState<string | null>(null);
  const [stashEditingDraft, setStashEditingDraft] = useState("");
  const [unpushedCommits, setUnpushedCommits] = useState<GitUnpushedCommit[]>([]);
  const [unpushedEditingHash, setUnpushedEditingHash] = useState<string | null>(null);
  const [unpushedEditingDraft, setUnpushedEditingDraft] = useState("");
  // Cmd+Enter fires keydown which calls commitEdit, then the same edit
  // exit causes blur on the unmounted textarea — both want to dispatch
  // the reword. The lock keeps it to a single inflight save per hash.
  const unpushedSavingRef = useRef<string | null>(null);
  const [popover, setPopover] = useState<PopoverState>(null);
  const panelStateRequestRef = useRef<Promise<void> | null>(null);
  const graphMetadataRequestRef = useRef<Promise<void> | null>(null);

  const deferredHistorySearch = useDeferredValue(historySearchText);
  const deferredHistoryPathSearch = useDeferredValue(historyPathSearchText);
  const currentRepoPath = panelState?.repoPath || browserPath;
  const repoName = repoNameFromPath(currentRepoPath);
  const browserPathRef = useRef(browserPath);
  const currentRepoPathRef = useRef(currentRepoPath);

  useEffect(() => {
    browserPathRef.current = browserPath;
    currentRepoPathRef.current = currentRepoPath;
  }, [browserPath, currentRepoPath]);

  // History list: apply persisted column widths and observe surface
  // width to drop columns at narrow sizes (Hash → Date → Author).
  useLayoutEffect(() => {
    const surface = historyListSurfaceRef.current;
    if (!surface) return;
    const widths = historyColWidthsRef.current;
    surface.style.setProperty("--col-author-w", `${widths.author}px`);
    surface.style.setProperty("--col-date-w", `${widths.date}px`);
    surface.style.setProperty("--col-hash-w", `${widths.hash}px`);
  }, [selectedTab]);

  useEffect(() => {
    if (selectedTab !== "history") return;
    const surface = historyListSurfaceRef.current;
    if (!surface || typeof ResizeObserver === "undefined") return;
    const apply = (width: number) => {
      let cols = 4;
      if (width < 280) cols = 1;
      else if (width < 360) cols = 2;
      else if (width < 480) cols = 3;
      surface.dataset.cols = String(cols);
    };
    apply(surface.getBoundingClientRect().width);
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) apply(entry.contentRect.width);
    });
    ro.observe(surface);
    return () => ro.disconnect();
  }, [selectedTab]);

  const activeCommitDetail = commitDetail && commitDetail.hash === historySelectedHash ? commitDetail : null;

  const filteredTagEntries = useMemo(() => {
    const needle = tagSearchText.trim().toLowerCase();
    return tags.filter((tag) => {
      if (!needle) return true;
      return [tag.name, tag.hash, tag.message].some((value) => value.toLowerCase().includes(needle));
    });
  }, [tagSearchText, tags]);

  const filteredRemoteEntries = useMemo(() => {
    const needle = remoteSearchText.trim().toLowerCase();
    return remotes.filter((remote) => {
      if (!needle) return true;
      return [remote.name, remote.fetchUrl, remote.pushUrl].some((value) => value.toLowerCase().includes(needle));
    });
  }, [remoteSearchText, remotes]);

  const filteredSubmodules = useMemo(() => {
    const needle = submoduleSearchText.trim().toLowerCase();
    return submodules.filter((submodule) => {
      if (!needle) return true;
      return [submodule.path, submodule.url, submodule.shortHash].some((value) =>
        value.toLowerCase().includes(needle),
      );
    });
  }, [submoduleSearchText, submodules]);

  const localBranches = useMemo(
    () => (graphMetadata?.branches || []).filter((name) => isLocalBranch(name)),
    [graphMetadata?.branches],
  );
  const remoteBranches = useMemo(
    () => (graphMetadata?.branches || []).filter((name) => !isLocalBranch(name)),
    [graphMetadata?.branches],
  );

  const navigationTabs = useMemo(
    () => [
      { key: "changes" as PanelTab, label: t("Changes"), icon: FileText, badge: panelState?.totalChanges ? String(panelState.totalChanges) : "" },
      { key: "history" as PanelTab, label: t("History"), icon: History, badge: "" },
      { key: "stash" as PanelTab, label: t("Stash"), icon: Archive, badge: stashes.length ? String(stashes.length) : "" },
      { key: "conflicts" as PanelTab, label: t("Conflicts"), icon: Layers, badge: conflicts.length ? String(conflicts.length) : "" },
    ],
    [panelState?.totalChanges, stashes.length, conflicts.length, t],
  );

  const filteredManagerLocalBranches = useMemo(() => {
    const needle = branchManagerSearchText.trim().toLowerCase();
    return localBranches.filter((name) => !needle || name.toLowerCase().includes(needle));
  }, [branchManagerSearchText, localBranches]);
  const filteredManagerRemoteBranches = useMemo(() => {
    const needle = branchManagerSearchText.trim().toLowerCase();
    return remoteBranches.filter((name) => !needle || name.toLowerCase().includes(needle));
  }, [branchManagerSearchText, remoteBranches]);

  const historyPathTree = useMemo(() => buildRepoPathTree(graphMetadata?.repoFiles || []), [graphMetadata?.repoFiles]);
  const filteredHistoryPathTree = useMemo(
    () => filterRepoPathTree(historyPathTree, deferredHistoryPathSearch),
    [deferredHistoryPathSearch, historyPathTree],
  );
  const historyPathExpandedSet = useMemo(() => new Set(historyPathExpanded), [historyPathExpanded]);
  const comparisonPathTree = useMemo(
    () => buildRepoPathTree(comparisonFiles.map((file) => file.path)),
    [comparisonFiles],
  );
  const comparisonExpandedSet = useMemo(() => new Set(comparisonExpandedPaths), [comparisonExpandedPaths]);

  const selectedConflictFile = useMemo(
    () => conflicts.find((file) => file.path === selectedConflictPath) || conflicts[0] || null,
    [conflicts, selectedConflictPath],
  );
  const selectedConflictHunks = useMemo(() => {
    if (!selectedConflictFile) return [];
    return conflictDrafts[selectedConflictFile.path] || selectedConflictFile.conflicts || [];
  }, [conflictDrafts, selectedConflictFile]);

  const workingDiffFiles = useMemo<DiffFileInput[]>(() => {
    if (!panelState) return [];
    const out: DiffFileInput[] = [];
    for (const file of panelState.stagedFiles) {
      out.push({
        id: workingFileKey(file.path, true),
        path: file.path,
        status: workingDiffStatusFromLetter(file.status),
        diffText: workingDiffCache[workingFileKey(file.path, true)] ?? null,
        additions: file.additions,
        deletions: file.deletions,
      });
    }
    for (const file of panelState.unstagedFiles) {
      out.push({
        id: workingFileKey(file.path, false),
        path: file.path,
        status: workingDiffStatusFromLetter(file.status),
        diffText: workingDiffCache[workingFileKey(file.path, false)] ?? null,
        additions: file.additions,
        deletions: file.deletions,
      });
    }
    return out;
  }, [panelState, workingDiffCache]);

  const workingDiffActiveId = useMemo(() => {
    if (!diffTarget || diffTarget.kind !== "working") return undefined;
    return workingFileKey(diffTarget.path, diffTarget.staged);
  }, [diffTarget]);

  function openWorkingDiff(target: { path: string; staged: boolean; untracked: boolean }) {
    setDiffTarget({ kind: "working", path: target.path, staged: target.staged, untracked: target.untracked });
    setWorkingDiffOpen(true);
  }

  function openWorkingDiffById(id: string) {
    const prefix = id.slice(0, 2);
    const path = id.slice(2);
    const staged = prefix === "S|";
    const all = [...(panelState?.stagedFiles || []), ...(panelState?.unstagedFiles || [])];
    const match = all.find((file) => file.path === path && file.staged === staged);
    setDiffTarget({
      kind: "working",
      path,
      staged,
      untracked: !staged && match?.status === "?",
    });
  }

  function showBanner(success: boolean, message: string) {
    setBanner({ success, message: message || (success ? t("Operation finished") : t("Operation failed")) });
  }

  function openPopoverFromElement(kind: PopoverKind, element: HTMLElement, width: number, data?: unknown) {
    // Anchor the popover's top-left so its top sits 4px under the
    // trigger's bottom edge (standard dropdown placement). Horizontal
    // alignment keeps the right edge flush with the trigger's right
    // edge — most callers are toolbar icons in the Git panel's right
    // pane, where right-align reads better than left-align.
    //
    // The popover layer is `position: fixed; inset: 0` (atoms.css), so
    // these coordinates are in viewport space. maxHeight is sized from
    // the real space below the trigger — that lets `.popover`'s
    // built-in `overflow: auto` produce a scroller that ends right at
    // the viewport bottom instead of being clipped by the static
    // 82vh CSS cap.
    const rect = element.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const GAP = 4;
    const MARGIN = 8;
    const left = Math.max(MARGIN, Math.min(vw - width - MARGIN, rect.right - width));
    const top = rect.bottom + GAP;
    const maxHeight = Math.max(160, vh - top - MARGIN);
    setPopover({ kind, left, top, width, maxHeight, data });
  }

  function openPopoverAt(kind: PopoverKind, clientX: number, clientY: number, width: number, data?: unknown) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const MARGIN = 8;
    const left = Math.max(MARGIN, Math.min(vw - width - MARGIN, clientX));
    const top = Math.max(MARGIN, Math.min(vh - 16, clientY));
    const maxHeight = Math.max(160, vh - top - MARGIN);
    setPopover({ kind, left, top, width, maxHeight, data });
  }

  function openChangeFileMenu(event: ReactMouseEvent<HTMLButtonElement>, file: GitPanelState["stagedFiles"][number], staged: boolean) {
    event.preventDefault();
    openPopoverAt("changeFileMenu", event.clientX, event.clientY, 196, {
      file,
      staged,
    } satisfies ChangeFileMenuState);
  }

  async function loadPanelState() {
    if (panelStateRequestRef.current) {
      return panelStateRequestRef.current;
    }
    const targetPath = browserPath;
    const request = (async () => {
    // Sidebar is on the drives sentinel ("This PC") or pre-bootstrap —
    // no real directory to inspect. Clear any stale snapshot from a
    // previous repo path so the view renders the "pick a directory"
    // empty state instead of lingering on old commits / branch state.
    // This also short-circuits the 3s polling tick below.
    if (!isBrowsableRepoPath(targetPath)) {
      setPanelState(null);
      setGitReady(false);
      setGitError("");
      return;
    }
    try {
      const next = await cmd.gitPanelState(targetPath);
      if (browserPathRef.current !== targetPath) return;
      // Keep the same reference when the data is equal so the 3s poll
      // doesn't cascade into workingDiffCache resets, which would make
      // the diff dialog flash "Loading…" every tick.
      setPanelState((prev) => (prev && panelStateEqual(prev, next) ? prev : next));
      setGitReady(true);
      setGitError("");
    } catch (error) {
      if (browserPathRef.current !== targetPath) return;
      setPanelState(null);
      setGitReady(false);
      setGitError(extractErrorMessage(error, t));
    }
    })().finally(() => {
      if (panelStateRequestRef.current === request) {
        panelStateRequestRef.current = null;
      }
    });
    panelStateRequestRef.current = request;
    return request;
  }

  async function loadGraphMetadata() {
    if (graphMetadataRequestRef.current) {
      return graphMetadataRequestRef.current;
    }
    const targetPath = currentRepoPath;
    const request = (async () => {
    if (!gitReady) return;
    try {
      const next = await cmd.gitGraphMetadata(targetPath);
      if (currentRepoPathRef.current === targetPath) {
        setGraphMetadata(next);
      }
    } catch {
      if (currentRepoPathRef.current === targetPath) {
        setGraphMetadata(null);
      }
    }
    })().finally(() => {
      if (graphMetadataRequestRef.current === request) {
        graphMetadataRequestRef.current = null;
      }
    });
    graphMetadataRequestRef.current = request;
    return request;
  }

  function historyAfterTimestamp() {
    const now = Math.floor(Date.now() / 1000);
    switch (historyDateFilter) {
      case "7d":
        return now - 7 * 24 * 60 * 60;
      case "30d":
        return now - 30 * 24 * 60 * 60;
      case "90d":
        return now - 90 * 24 * 60 * 60;
      case "365d":
        return now - 365 * 24 * 60 * 60;
      default:
        return 0;
    }
  }

  async function loadGraphRows(reset = true) {
    if (!gitReady) return;
    if (reset) {
      setHistoryLoading(true);
      setHistoryHasMore(false);
    } else {
      if (historyLoadingMore || historyLoading || !historyHasMore) return;
      setHistoryLoadingMore(true);
    }
    try {
      // Pier's pattern: each loadMore re-fetches the FULL window from row 0
      // up to the new ceiling and recomputes the layout for all of them at
      // once, then replaces graphRows. Appending per-page slices instead
      // (the previous behaviour) loses cross-page edges — main's long
      // first-parent edge through an off-main branch — and produces
      // inconsistent lane assignments at page boundaries because each page
      // saw a different commit set.
      const desiredCount = reset
        ? HISTORY_PAGE_SIZE
        : graphRows.length + HISTORY_PAGE_SIZE;
      const previousCount = reset ? 0 : graphRows.length;
      const rows = await cmd.gitGraphHistory({
        path: currentRepoPath,
        limit: desiredCount,
        skip: 0,
        branch: historyBranchFilter || null,
        author: historyAuthorFilter || null,
        searchText: deferredHistorySearch || null,
        firstParent: historyFirstParent,
        noMerges: historyNoMerges,
        afterTimestamp: historyAfterTimestamp(),
        paths: historyPaths.length ? historyPaths : null,
        topoOrder: historySortMode === "topo",
        showLongEdges: historyShowLongEdges,
      });
      setGraphRows(rows);
      // hasMore = true while git keeps producing new commits. If a loadMore
      // didn't actually grow the row count, we've hit the bottom of the log.
      setHistoryHasMore(rows.length >= desiredCount && rows.length > previousCount);
    } catch (error) {
      showBanner(false, extractErrorMessage(error, t));
      if (reset) setGraphRows([]);
      setHistoryHasMore(false);
    } finally {
      if (reset) {
        setHistoryLoading(false);
      } else {
        setHistoryLoadingMore(false);
      }
    }
  }

  async function loadStashes() {
    if (!gitReady) return;
    try {
      setStashes(await cmd.gitStashList(currentRepoPath));
    } catch {
      setStashes([]);
    }
  }

  async function loadTags() {
    if (!gitReady) return;
    try {
      setTags(await cmd.gitTagsList(currentRepoPath));
    } catch {
      setTags([]);
    }
  }

  async function loadRemotes() {
    if (!gitReady) return;
    try {
      setRemotes(await cmd.gitRemotesList(currentRepoPath));
    } catch {
      setRemotes([]);
    }
  }

  async function loadConfigEntries() {
    if (!gitReady) return;
    try {
      setConfigEntries(await cmd.gitConfigList(currentRepoPath));
    } catch {
      setConfigEntries([]);
    }
  }

  async function loadRebase() {
    if (!gitReady) return;
    try {
      const next = await cmd.gitRebasePlan(currentRepoPath, rebaseCommitCount);
      setRebasePlan(next);
      setRebaseDraftItems(next.items);
    } catch {
      setRebasePlan({ inProgress: false, items: [] });
      setRebaseDraftItems([]);
    }
  }

  async function loadSubmodules() {
    if (!gitReady) return;
    try {
      setSubmodules(await cmd.gitSubmodulesList(currentRepoPath));
    } catch {
      setSubmodules([]);
    }
  }

  async function loadConflicts() {
    if (!gitReady) return;
    try {
      setConflicts(await cmd.gitConflictsList(currentRepoPath));
    } catch {
      setConflicts([]);
    }
  }

  async function loadBranchesMenu() {
    if (!gitReady) return;
    try {
      setBranchMenuBranches(await cmd.gitBranchList(currentRepoPath));
    } catch {
      setBranchMenuBranches([]);
    }
  }

  async function loadCommitDetail(hash: string): Promise<GitCommitDetailView | null> {
    if (!gitReady || !hash) return null;
    try {
      const detail = await cmd.gitCommitDetail(currentRepoPath, hash);
      setCommitDetail(detail);
      if (graphRows[0]?.hash === detail.hash) {
        setHistoryAmendMessage(detail.message || "");
      }
      return detail;
    } catch {
      setCommitDetail(null);
      return null;
    }
  }

  async function refreshAfterMutation(extra?: {
    stash?: boolean;
    tags?: boolean;
    remotes?: boolean;
    config?: boolean;
    rebase?: boolean;
    submodules?: boolean;
    conflicts?: boolean;
  }) {
    await loadPanelState();
    await loadGraphMetadata();
    if (selectedTab === "history") {
      await loadGraphRows();
    }
    if (selectedTab === "stash" || extra?.stash) {
      await loadStashes();
    }
    if (selectedTab === "conflicts" || extra?.conflicts) {
      await loadConflicts();
    }
    if (extra?.tags || popover?.kind === "tagManager") {
      await loadTags();
    }
    if (extra?.remotes || popover?.kind === "remoteManager") {
      await loadRemotes();
    }
    if (extra?.config || popover?.kind === "configManager") {
      await loadConfigEntries();
    }
    if (extra?.rebase || popover?.kind === "rebaseManager") {
      await loadRebase();
    }
    if (extra?.submodules || popover?.kind === "submoduleManager") {
      await loadSubmodules();
    }
  }

  async function runGitAction(
    action: () => Promise<unknown>,
    options?: {
      successMessage?: string;
      refresh?: boolean;
      stash?: boolean;
      tags?: boolean;
      remotes?: boolean;
      config?: boolean;
      rebase?: boolean;
      submodules?: boolean;
      conflicts?: boolean;
    },
  ) {
    setBusy(true);
    try {
      const result = await action();
      const resultText = typeof result === "string" ? result.trim() : "";
      showBanner(true, options?.successMessage || resultText || t("Operation finished"));
      if (options?.refresh !== false) {
        await refreshAfterMutation(options);
      }
      return result;
    } catch (error) {
      showBanner(false, extractErrorMessage(error, t));
      throw error;
    } finally {
      setBusy(false);
    }
  }

  // Keep-alive refresh. Runs only while this panel is the active right-side
  // tool AND the window is foregrounded. On becoming active (or on path
  // change while active) we fetch once immediately so the UI is fresh,
  // then poll at a conservative cadence. Hidden panels sit idle — the RightSidebar keep-
  // alive means this component stays mounted but costs zero IPC.
  useEffect(() => {
    if (!isActive) return undefined;
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      void loadPanelState();
    };
    tick();
    const timer = window.setInterval(tick, GIT_STATUS_POLL_MS);
    // Debounce visibility flips: rapid Cmd-Tab in/out previously
    // issued one `git_panel_state` IPC per transition; now a 300ms
    // quiet period collapses the burst into a single fetch.
    let visTimer: number | null = null;
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      if (visTimer !== null) window.clearTimeout(visTimer);
      visTimer = window.setTimeout(() => {
        visTimer = null;
        tick();
      }, 300);
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      if (visTimer !== null) window.clearTimeout(visTimer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [isActive, browserPath]);

  useEffect(() => {
    if (!banner) return undefined;
    const timer = window.setTimeout(() => setBanner(null), 2800);
    return () => window.clearTimeout(timer);
  }, [banner]);

  useEffect(() => {
    graphMetadataRequestRef.current = null;
    setGraphMetadata(null);
    setGraphRows([]);
    setTags([]);
    setRemotes([]);
    setConfigEntries([]);
    setSubmodules([]);
    setRebasePlan({ inProgress: false, items: [] });
    setRebaseDraftItems([]);
    setStashes([]);
    setConflicts([]);
    setWorkingDiffCache({});
    setCommitDiffCache({});
  }, [currentRepoPath]);

  useEffect(() => {
    panelStateRequestRef.current = null;
  }, [browserPath]);

  useEffect(() => {
    if (popover?.kind !== "historyPathFilter" || !graphMetadata?.repoFiles.length) return;
    setHistoryPathExpanded(defaultExpandedHistoryPaths(graphMetadata.repoFiles, historyPathSelection));
  }, [graphMetadata?.repoFiles, popover?.kind, historyPathSelection]);

  const historyPathsKey = useMemo(() => historyPaths.join("\n"), [historyPaths]);

  useEffect(() => {
    if (!gitReady) return;
    if (selectedTab === "history") {
      void loadGraphMetadata();
      const timer = window.setTimeout(() => {
        void loadGraphRows();
      }, 220);
      return () => window.clearTimeout(timer);
    }
    if (selectedTab === "stash") {
      void loadStashes();
    }
    if (selectedTab === "conflicts") {
      void loadConflicts();
    }
    return undefined;
  }, [
    gitReady,
    selectedTab,
    currentRepoPath,
    historyBranchFilter,
    historyAuthorFilter,
    deferredHistorySearch,
    historyDateFilter,
    historyFirstParent,
    historyNoMerges,
    historyPathsKey,
    historySortMode,
    historyShowLongEdges,
  ]);

  // The branch manager popover and the path-filter popover need fresh
  // graph metadata (branches list, repo files), but they shouldn't
  // trigger a full history reload — that wipes the visible graph rows
  // and forces the user back to the loading skeleton on every open.
  useEffect(() => {
    if (!gitReady) return;
    if (popover?.kind === "branchManager") void loadGraphMetadata();
  }, [gitReady, popover?.kind]);

  useEffect(() => {
    if (!panelState) {
      setDiffTarget(null);
      return;
    }
    const staged = panelState.stagedFiles;
    const unstaged = panelState.unstagedFiles;
    const all = [...staged, ...unstaged];
    if (all.length === 0) {
      setDiffTarget(null);
      return;
    }
    setDiffTarget((current) => {
      if (
        current &&
        current.kind === "working" &&
        all.some((file) => file.path === current.path && file.staged === current.staged)
      ) {
        return current;
      }
      const preferred = staged[0] || unstaged[0];
      return preferred
        ? {
            kind: "working",
            path: preferred.path,
            staged: preferred.staged,
            untracked: preferred.status === "?" && !preferred.staged,
          }
        : null;
    });
  }, [panelState]);

  useEffect(() => {
    if (panelState) {
      setGitStatus(
        panelState.currentBranch || null,
        panelState.aheadCount ?? 0,
        panelState.behindCount ?? 0,
      );
    } else {
      clearGitStatus();
    }
    return () => clearGitStatus();
  }, [panelState, setGitStatus, clearGitStatus]);

  useEffect(() => {
    if (!diffTarget) return;
    if (diffTarget.kind !== "working") return;
    const key = workingFileKey(diffTarget.path, diffTarget.staged);
    if (workingDiffCache[key] != null) return;
    let cancelled = false;
    const load = async () => {
      try {
        const next = await cmd.gitDiff(currentRepoPath, diffTarget.path, diffTarget.staged, diffTarget.untracked);
        if (!cancelled) setWorkingDiffCache((prev) => ({ ...prev, [key]: next || "" }));
      } catch (error) {
        if (!cancelled) setWorkingDiffCache((prev) => ({ ...prev, [key]: extractErrorMessage(error, t) }));
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [currentRepoPath, diffTarget, workingDiffCache, t]);

  useEffect(() => {
    setWorkingDiffCache({});
  }, [panelState]);

  useEffect(() => {
    if (!graphRows.length) {
      setHistorySelectedHash("");
      setCommitDetail(null);
      return;
    }
    setHistorySelectedHash((current) =>
      current && graphRows.some((row) => row.hash === current) ? current : "",
    );
  }, [graphRows]);

  useEffect(() => {
    if (!historySelectedHash) {
      setCommitDetail(null);
      return;
    }
    void loadCommitDetail(historySelectedHash);
  }, [historySelectedHash, currentRepoPath]);

  useEffect(() => {
    if (!comparisonFiles.length) {
      setComparisonSelectedPath("");
      setComparisonDiff("");
      setComparisonExpandedPaths([]);
      return;
    }
    setComparisonSelectedPath((current) =>
      comparisonFiles.some((file) => file.path === current) ? current : comparisonFiles[0].path,
    );
  }, [comparisonFiles]);

  useEffect(() => {
    if (!comparisonFiles.length) return;
    setComparisonExpandedPaths(
      defaultExpandedHistoryPaths(
        comparisonFiles.map((file) => file.path),
        comparisonSelectedPath ? [comparisonSelectedPath] : [],
      ),
    );
  }, [comparisonFiles]);

  useEffect(() => {
    if (!comparisonSelectedPath) return;
    setComparisonExpandedPaths((current) => {
      const next = new Set(current);
      let changed = false;
      for (const ancestor of pathAncestors(comparisonSelectedPath)) {
        if (!next.has(ancestor)) {
          next.add(ancestor);
          changed = true;
        }
      }
      return changed ? Array.from(next) : current;
    });
  }, [comparisonSelectedPath]);

  useEffect(() => {
    if (!historyCompareDialogOpen || !comparisonBaseHash || !comparisonSelectedPath) return;
    let cancelled = false;
    void cmd
      .gitComparisonDiff(currentRepoPath, comparisonBaseHash, comparisonSelectedPath)
      .then((next) => {
        if (!cancelled) setComparisonDiff(next);
      })
      .catch((error) => {
        if (!cancelled) setComparisonDiff(extractErrorMessage(error, t));
      });
    return () => {
      cancelled = true;
    };
  }, [historyCompareDialogOpen, comparisonBaseHash, comparisonSelectedPath, currentRepoPath]);

  useEffect(() => {
    if (!conflicts.length) {
      setSelectedConflictPath("");
      setConflictDrafts({});
      return;
    }
    setSelectedConflictPath((current) =>
      conflicts.some((file) => file.path === current) ? current : conflicts[0].path,
    );
    setConflictDrafts((current) => {
      const next: Record<string, GitConflictHunkView[]> = {};
      for (const file of conflicts) {
        next[file.path] = current[file.path] || file.conflicts.map((hunk) => ({ ...hunk }));
      }
      return next;
    });
  }, [conflicts]);

  function historyPathSummary() {
    if (historyPaths.length === 0) return t("Path");
    if (historyPaths.length === 1) return historyPaths[0];
    return `${historyPaths.length} ${t("paths")}`;
  }

  function historyDateFilterLabel() {
    switch (historyDateFilter) {
      case "7d": return t("Last 7 days");
      case "30d": return t("Last 30 days");
      case "90d": return t("Last 90 days");
      case "365d": return t("Last year");
      default: return t("Date");
    }
  }

  function historyPathSelectionState(node: RepoPathTreeNode) {
    if (historyPathSelectionSet.has(node.path)) return "selected";
    if (node.kind === "directory" && historyPathAncestorSet.has(node.path)) return "partial";
    return "none";
  }

  function toggleHistoryPathSelection(path: string) {
    setHistoryPathSelection((current) =>
      current.includes(path) ? current.filter((item) => item !== path) : [...current, path],
    );
  }

  function toggleHistoryPathExpanded(path: string) {
    setHistoryPathExpanded((current) =>
      current.includes(path) ? current.filter((item) => item !== path) : [...current, path],
    );
  }

  function toggleComparisonExpanded(path: string) {
    setComparisonExpandedPaths((current) =>
      current.includes(path) ? current.filter((item) => item !== path) : [...current, path],
    );
  }

  function renderHistoryPathTree(nodes: RepoPathTreeNode[], depth = 0): ReactNode {
    return nodes.map((node) => {
      const state = historyPathSelectionState(node);
      const expanded = deferredHistoryPathSearch.trim() ? true : historyPathExpandedSet.has(node.path);
      return (
        <div key={node.id} className="git-path-tree__node">
          <button
            className={["git-path-row", state === "selected" ? "git-path-row--active" : "", state === "partial" ? "git-path-row--partial" : ""]
              .filter(Boolean)
              .join(" ")}
            onClick={() => toggleHistoryPathSelection(node.path)}
            style={{ "--git-path-depth": depth } as CSSProperties}
            type="button"
          >
            <span className="git-path-row__indent" />
            {node.kind === "directory" ? (
              <span
                className="git-path-row__toggle"
                onClick={(event) => {
                  event.stopPropagation();
                  toggleHistoryPathExpanded(node.path);
                }}
              >
                {expanded ? <ChevronDown size={10} /> : <ArrowRight size={10} />}
              </span>
            ) : (
              <span className="git-path-row__toggle git-path-row__toggle--placeholder" />
            )}
            <span
              className={[
                "git-path-row__check",
                state === "selected" ? "git-path-row__check--active" : "",
                state === "partial" ? "git-path-row__check--partial" : "",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              {state === "selected" ? <Check size={10} /> : state === "partial" ? <Minus size={10} /> : null}
            </span>
            <span className={"git-path-row__icon" + (node.kind === "directory" ? " git-path-row__icon--dir" : "")}>
              {node.kind === "directory" ? <Folder size={13} /> : <FileText size={12} />}
            </span>
            <span className="git-path-row__text">{node.name}</span>
          </button>
          {node.kind === "directory" && expanded && node.children.length ? renderHistoryPathTree(node.children, depth + 1) : null}
        </div>
      );
    });
  }

  function renderComparisonTree(nodes: RepoPathTreeNode[], depth = 0): ReactNode {
    return nodes.map((node) => {
      const expanded = comparisonExpandedSet.has(node.path);
      if (node.kind === "directory") {
        return (
          <div key={node.id} className="git-path-tree__node">
            <button
              className="git-compare-tree__row git-compare-tree__row--directory"
              onClick={() => toggleComparisonExpanded(node.path)}
              style={{ "--git-path-depth": depth } as CSSProperties}
              type="button"
            >
              <span className="git-path-row__indent" />
              <span className="git-path-row__toggle">
                {expanded ? <ChevronDown size={10} /> : <ArrowRight size={10} />}
              </span>
              <span className="git-path-row__icon">
                <Folder size={12} />
              </span>
              <span className="git-path-row__text">{node.name}</span>
              <span className="git-path-row__meta">{countRepoPathLeaves(node)}</span>
            </button>
            {expanded && node.children.length ? renderComparisonTree(node.children, depth + 1) : null}
          </div>
        );
      }

      return (
        <button
          key={node.id}
          className={[
            "git-compare-file",
            "git-compare-tree__row",
            "git-compare-tree__row--file",
            comparisonSelectedPath === node.path ? "git-compare-file--active" : "",
          ]
            .filter(Boolean)
            .join(" ")}
          onClick={() => setComparisonSelectedPath(node.path)}
          style={{ "--git-path-depth": depth } as CSSProperties}
          type="button"
        >
          <span className="git-path-row__indent" />
          <span className="git-path-row__toggle git-path-row__toggle--placeholder" />
          <span className="git-path-row__icon">
            <FileText size={12} />
          </span>
          <span className="git-compare-file__copy">
            <span className="git-compare-file__name">{node.name}</span>
          </span>
        </button>
      );
    });
  }

  async function ensureCommitDiff(hash: string, filePath: string) {
    setCommitDiffCache((cache) => (filePath in cache ? cache : { ...cache, [filePath]: null }));
    try {
      const text = await cmd.gitCommitFileDiff(currentRepoPath, hash, filePath);
      setCommitDiffCache((cache) => ({ ...cache, [filePath]: text || "" }));
    } catch (error) {
      setCommitDiffCache((cache) => ({ ...cache, [filePath]: extractErrorMessage(error, t) }));
    }
  }

  function openCommitMultiDiff(detail: GitCommitDetailView, initialPath?: string) {
    if (!detail.changedFiles.length) return;
    setCommitDiffHash(detail.hash);
    const seed: Record<string, string | null> = {};
    for (const file of detail.changedFiles) seed[file.path] = null;
    setCommitDiffCache(seed);
    const start = initialPath || detail.changedFiles[0].path;
    setCommitDiffActivePath(start);
    setCommitDiffOpen(true);
    void ensureCommitDiff(detail.hash, start);
  }

  function renderHistoryInlineDetail(detail: GitCommitDetailView) {
    const subject = detail.message.split("\n", 1)[0] || "";
    const body = detail.message.slice(subject.length).replace(/^\n+/, "");
    return (
      <div className="git-history-inline">
        <div className="git-history-inline__meta mono">
          <span className="git-history-inline__hash" title={detail.hash}>{detail.shortHash}</span>
          <button
            type="button"
            className="git-history-inline__copy-btn"
            onClick={() => void writeClipboardText(detail.hash)}
            title={t("Copy hash") + " · " + detail.hash}
            aria-label={t("Copy hash")}
          >
            <Copy size={11} />
          </button>
          <span className="git-history-inline__author">{detail.author}</span>
          <span className="git-history-inline__date">{detail.date}</span>
        </div>
        <div className="git-history-inline__subject">{subject}</div>
        {body ? <pre className="git-history-inline__body mono">{body}</pre> : null}
        {detail.changedFiles.length ? (
          <div className="git-history-inline__files">
            <div className="git-history-inline__files-head mono">
              <span>{t("Changed files")}</span>
              <span className="git-history-inline__files-count">{detail.changedFiles.length}</span>
            </div>
            {detail.changedFiles.map((file) => (
              <SelectableFileRow
                key={`${detail.hash}-${file.path}`}
                onActivate={() => openCommitMultiDiff(detail, file.path)}
                title={t("Open diff") + " · " + file.path}
              >
                <span className="git-history-inline__file-delta mono">
                  {file.additions > 0 ? <span className="git-file-row__delta-add">+{file.additions}</span> : null}
                  {file.deletions > 0 ? <span className="git-file-row__delta-del">−{file.deletions}</span> : null}
                </span>
                <span className="git-history-inline__file-path mono" title={file.path}>{file.path}</span>
              </SelectableFileRow>
            ))}
          </div>
        ) : null}
      </div>
    );
  }

  function commitDiffStatus(file: { additions: number; deletions: number }): DiffFileInput["status"] {
    if (file.deletions === 0 && file.additions > 0) return "added";
    if (file.additions === 0 && file.deletions > 0) return "deleted";
    return "modified";
  }

  function historyContextParentHash(commit: GitGraphRowView | null) {
    if (!commit) return "";
    if (activeCommitDetail && activeCommitDetail.hash === commit.hash && activeCommitDetail.parentHash) {
      return activeCommitDetail.parentHash;
    }
    return String(commit.parents || "").trim().split(/\s+/)[0] || "";
  }

  function historyContextIsHead(commit: GitGraphRowView | null) {
    return !!(commit && graphRows[0] && commit.hash === graphRows[0].hash);
  }

  function historyContextCheckoutTargets(commit: GitGraphRowView | null) {
    const items: { label: string; target: string; tracking?: string }[] = [];
    if (!commit?.hash) return items;
    items.push({ label: t("Checkout this revision"), target: commit.hash });
    const seen = new Set<string>();
    for (const token of refTokens(commit.refs)) {
      let ref = token;
      if (!ref || ref === "HEAD" || ref.startsWith("tag:")) continue;
      if (ref.startsWith("HEAD -> ")) ref = ref.slice("HEAD -> ".length);
      if (!ref) continue;
      let target = ref;
      let tracking = "";
      if (ref.includes("/")) {
        tracking = ref;
        target = ref.replace(/^[^/]+\//, "");
      }
      const key = `${target}::${tracking}`;
      if (seen.has(key)) continue;
      seen.add(key);
      items.push({
        label: `${t("Checkout branch")} '${ref}'`,
        target,
        tracking,
      });
    }
    return items;
  }

  function browserUrlForCommit(hash: string) {
    for (const remote of remotes) {
      const base = normalizeRemoteBaseUrl(remote.fetchUrl || remote.pushUrl);
      if (!base) continue;
      if (base.includes("github.com/") || base.includes("gitlab.com/") || base.includes("gitlab.")) {
        return `${base}/commit/${hash}`;
      }
    }
    return "";
  }

  async function openCommitInBrowser(hash: string) {
    const url = browserUrlForCommit(hash);
    if (!url) return;
    try {
      await openUrl(url);
    } catch (error) {
      showBanner(false, extractErrorMessage(error, t));
    }
  }

  async function copyText(value: string) {
    if (!value) return;
    await writeClipboardText(value);
    showBanner(true, t("Copied"));
  }

  function beginRemoteEdit(remote: GitRemoteView) {
    setRemoteEditSourceName(remote.name);
    setRemoteDraftName(remote.name);
    setRemoteDraftUrl(remote.fetchUrl || remote.pushUrl);
    setRemoteComposerExpanded(true);
  }

  function clearRemoteDraft() {
    setRemoteEditSourceName("");
    setRemoteDraftName("");
    setRemoteDraftUrl("");
    setRemoteComposerExpanded(false);
  }

  function beginConfigEdit(entry: GitConfigEntryView) {
    setConfigEditingKey(`${entry.scope}:${entry.key}`);
    setConfigEditingDraft(entry.value);
  }

  function commitConfigEdit(entry: GitConfigEntryView) {
    const next = configEditingDraft;
    const original = entry.value;
    setConfigEditingKey(null);
    if (next === original) return;
    void runGitAction(
      () => cmd.gitSetConfigValue(currentRepoPath, entry.key, next, entry.scope === "global"),
      { config: true },
    );
  }

  // IMPORTANT: open the popover synchronously with the trigger element
  // BEFORE awaiting any data load. React clears `event.currentTarget` once
  // the handler returns, so reading it after `await` gives `null` and
  // `getBoundingClientRect()` throws — the rejection is then swallowed by
  // the `void openXyz(event)` onClick, making the buttons silently no-op.
  function openBranchMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("branchMenu", event.currentTarget, 320);
    void loadBranchesMenu();
  }

  function openBranchManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("branchManager", event.currentTarget, 460);
    void loadGraphMetadata();
  }

  async function loadUnpushedCommits() {
    if (!currentRepoPath) return;
    try {
      const commits = await cmd.gitUnpushedCommits(currentRepoPath);
      setUnpushedCommits(commits);
    } catch {
      setUnpushedCommits([]);
    }
  }

  function openUnpushedCommits(event: ReactMouseEvent<HTMLButtonElement>) {
    setUnpushedEditingHash(null);
    setUnpushedEditingDraft("");
    openPopoverFromElement("unpushedCommits", event.currentTarget, 420);
    void loadUnpushedCommits();
  }

  function openTagManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("tagManager", event.currentTarget, 344);
    void loadTags();
  }

  function openRemoteManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("remoteManager", event.currentTarget, 372);
    void loadRemotes();
  }

  function openConfigManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("configManager", event.currentTarget, 372);
    void loadConfigEntries();
  }

  function openRebaseManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("rebaseManager", event.currentTarget, 432);
    void loadRebase();
  }

  function openSubmoduleManager(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("submoduleManager", event.currentTarget, 392);
    void loadSubmodules();
  }

  function openHistoryBranchFilter(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("historyBranchFilter", event.currentTarget, 340);
    void loadGraphMetadata();
  }

  function openHistoryAuthorFilter(event: ReactMouseEvent<HTMLButtonElement>) {
    openPopoverFromElement("historyAuthorFilter", event.currentTarget, 280);
    void loadGraphMetadata();
  }

  function openHistoryPathFilter(event: ReactMouseEvent<HTMLButtonElement>) {
    setHistoryPathSelection(historyPaths);
    setHistoryPathSearchText("");
    setHistoryPathExpanded(defaultExpandedHistoryPaths(graphMetadata?.repoFiles || [], historyPaths));
    openPopoverFromElement("historyPathFilter", event.currentTarget, 380);
    void loadGraphMetadata();
  }

  const workingTreeClean = panelState?.workingTreeClean ?? true;

  const historyHandlersRef = useRef({
    loadCommitDetail,
    openCommitMultiDiff,
    openPopoverAt,
    setHistoryContextCommit,
  });
  historyHandlersRef.current = {
    loadCommitDetail,
    openCommitMultiDiff,
    openPopoverAt,
    setHistoryContextCommit,
  };

  const handleHistorySelect = useCallback((hash: string, wasActive: boolean) => {
    setHistorySelectedHash(wasActive ? "" : hash);
  }, []);

  const handleHistoryDoubleClick = useCallback((hash: string) => {
    setHistorySelectedHash(hash);
    void historyHandlersRef.current.loadCommitDetail(hash).then((detail) => {
      if (detail) historyHandlersRef.current.openCommitMultiDiff(detail);
    });
  }, []);

  const handleHistoryContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLButtonElement>, row: GitGraphRowView) => {
      event.preventDefault();
      historyHandlersRef.current.setHistoryContextCommit(row);
      historyHandlersRef.current.openPopoverAt(
        "historyCommit",
        event.clientX,
        event.clientY,
        232,
        row,
      );
    },
    [],
  );

  const rowMetaByHash = useMemo(() => {
    const map = new Map<
      string,
      {
        refs: string[];
        isHead: boolean;
        authorColorValue: string;
        authorInitialValue: string;
        formattedDate: string;
        dimmed: boolean;
      }
    >();
    const myName = graphMetadata?.gitUserName || "";
    const currentBranch = panelState?.currentBranch || "";
    for (const row of graphRows) {
      let dimmed = false;
      switch (historyHighlightMode) {
        case "mine":
          dimmed = !!row.author && row.author !== myName;
          break;
        case "merge":
          dimmed = !historyRowIsMerge(row);
          break;
        case "branch":
          dimmed = !row.refs.includes(currentBranch) && !row.refs.includes("HEAD");
          break;
        default:
          dimmed = false;
      }
      const tokens = refTokens(row.refs);
      const isHead = tokens.some((token) => token === "HEAD" || token.startsWith("HEAD ->"));
      map.set(row.hash, {
        refs: tokens,
        isHead,
        authorColorValue: authorColor(row.author),
        authorInitialValue: authorInitial(row.author),
        formattedDate: formatGraphDate(row.dateTimestamp),
        dimmed,
      });
    }
    return map;
  }, [graphRows, historyHighlightMode, graphMetadata?.gitUserName, panelState?.currentBranch]);

  const graphLaneWidth = useMemo(() => {
    let maxX = 0;
    for (const row of graphRows) {
      const nodeX = row.nodeColumn * GRAPH_LANE_PX + GRAPH_LANE_PX / 2 + 4;
      if (nodeX > maxX) maxX = nodeX;
      for (const segment of row.segments) {
        if (segment.xTop > maxX) maxX = segment.xTop;
        if (segment.xBottom > maxX) maxX = segment.xBottom;
      }
      for (const arrow of row.arrows) {
        if (arrow.x > maxX) maxX = arrow.x;
      }
    }
    return Math.max(Math.ceil(maxX + GRAPH_LANE_PX / 2), GRAPH_LANE_MIN_W);
  }, [graphRows]);

  const historySelectedIndex = useMemo(() => {
    if (!historySelectedHash) return -1;
    return graphRows.findIndex((row) => row.hash === historySelectedHash);
  }, [graphRows, historySelectedHash]);

  const loadGraphRowsRef = useRef(loadGraphRows);
  loadGraphRowsRef.current = loadGraphRows;

  const handleHistoryListScroll = useCallback((event: ReactUIEvent<HTMLDivElement>) => {
    const target = event.currentTarget;
    // Prefetch when the user is still ~2 viewports away from the bottom.
    // Fires eagerly; loadGraphRows is re-entrant-safe (guards on
    // historyLoadingMore / historyHasMore), so spurious extra calls during
    // a fast scroll are dropped.
    const distanceToBottom = target.scrollHeight - (target.scrollTop + target.clientHeight);
    if (distanceToBottom < target.clientHeight * 2) {
      void loadGraphRowsRef.current(false);
    }
  }, []);

  // Safety net: if the current rows don't even fill the viewport (e.g. a
  // narrow filter returned a short first page), auto-load the next page so
  // the user doesn't get stranded on "End of history" despite there being
  // more to fetch.
  useEffect(() => {
    if (!historyHasMore || historyLoading || historyLoadingMore) return;
    const el = historyListRef.current;
    if (!el) return;
    if (el.scrollHeight < el.clientHeight * 2) {
      void loadGraphRowsRef.current(false);
    }
  }, [graphRows.length, historyHasMore, historyLoading, historyLoadingMore]);

  if (!browserPath) {
    return <div className="git-panel git-panel--loading"><PanelSkeleton variant="rows" rows={10} /></div>;
  }

  // Sidebar is parked on the drive picker ("This PC"). There's no real
  // directory to inspect, so show a clear empty state instead of
  // rendering stale Git data from a previously-visited repo.
  if (!isBrowsableRepoPath(browserPath)) {
    return (
      <div className="git-panel">
        <GitEmptyState
          icon={Folder}
          title={t("No directory selected")}
          description={t("Pick a directory in the left sidebar to see its Git status.")}
        />
      </div>
    );
  }

  return (
    <div className="git-panel" ref={panelRef}>
      <div className="git-panel__chrome">
        <div className="git-tabs git-tabs--chrome">
          {navigationTabs.map((tab) => {
            const Icon = tab.icon;
            const active = selectedTab === tab.key;
            return (
              <button
                key={tab.key}
                className={active ? "git-tab git-tab--active" : "git-tab"}
                onClick={() => startTransition(() => setSelectedTab(tab.key))}
                type="button"
              >
                <Icon size={12} />
                <span>{tab.label}</span>
                {tab.badge ? <span className="git-tab__badge">{tab.badge}</span> : null}
              </button>
            );
          })}
          <div className="git-tabs__spacer" />
          <GitIconButton
            aria-label={t("Refresh")}
            className="git-tabs__action"
            disabled={busy}
            icon={RefreshCw}
            onClick={() => void refreshAfterMutation({ stash: selectedTab === "stash", conflicts: selectedTab === "conflicts" })}
          />
        </div>

        {gitReady ? (
          <section className="git-panel__branch-card">
            <div className="git-panel__branch-row">
              <button
                className="git-panel__branch-pill git-panel__branch-pill--card"
                onClick={(event) => void openBranchMenu(event)}
                title={panelState?.currentBranch || t("Detached")}
              >
                <GitBranch size={12} />
                <ChevronDown size={10} />
                <span className="git-panel__branch-name">{panelState?.currentBranch || t("Detached")}</span>
              </button>

              {panelState?.trackingBranch ? (
                <span className="git-panel__branch-tracking mono" title={panelState.trackingBranch}>
                  <ArrowRight size={10} />
                  <span>{panelState.trackingBranch}</span>
                </span>
              ) : null}

              {panelState?.behindCount ? (
                <span className="git-panel__branch-count git-panel__branch-count--behind mono" title={t("Behind")}>
                  <ArrowDown size={10} />
                  {panelState.behindCount}
                </span>
              ) : null}
              {panelState?.aheadCount ? (
                <span className="git-panel__branch-count git-panel__branch-count--ahead mono" title={t("Ahead")}>
                  <ArrowUp size={10} />
                  {panelState.aheadCount}
                </span>
              ) : null}

              <div className="git-panel__branch-spacer" />

              <div className="git-panel__branch-tools">
                <GitIconButton
                  aria-label={t("Branches")}
                  icon={GitBranch}
                  onClick={(event) => openBranchManager(event)}
                  title={t("Branches")}
                />
                <GitIconButton aria-label={t("Tags")} icon={Tag} onClick={(event) => void openTagManager(event)} />
                <GitIconButton aria-label={t("Remotes")} icon={Network} onClick={(event) => void openRemoteManager(event)} />
                <GitIconButton aria-label={t("Submodules")} icon={Layers} onClick={(event) => void openSubmoduleManager(event)} />
                <GitIconButton aria-label={t("Interactive rebase")} icon={GitMerge} onClick={(event) => void openRebaseManager(event)} />
                <GitIconButton aria-label={t("Config")} icon={Settings2} onClick={(event) => void openConfigManager(event)} />
                <GitIconButton
                  aria-label={t("Fetch")}
                  disabled={busy}
                  icon={RefreshCw}
                  onClick={() => void runGitAction(() => cmd.gitFetchRemote(currentRepoPath, null), { remotes: true })}
                />
              </div>

              <div className="git-panel__branch-divider" />

              <GitIconButton
                aria-label={t("Unpushed commits")}
                className={"git-panel__branch-sync" + (panelState?.aheadCount ? " git-panel__branch-sync--active" : "")}
                disabled={!panelState?.aheadCount}
                icon={Upload}
                onClick={openUnpushedCommits}
                title={panelState?.aheadCount ? `${panelState.aheadCount} ${t("unpushed commits")}` : t("Unpushed commits")}
              />
              <GitIconButton
                aria-label={t("Pull")}
                className={panelState?.behindCount ? "git-panel__branch-sync git-panel__branch-sync--active" : "git-panel__branch-sync"}
                disabled={!panelState?.behindCount || busy}
                icon={ArrowDownCircle}
                onClick={() => void runGitAction(() => cmd.gitPull(currentRepoPath))}
              />
              <GitIconButton
                aria-label={t("Push")}
                className={panelState?.aheadCount ? "git-panel__branch-sync git-panel__branch-sync--active" : "git-panel__branch-sync"}
                disabled={!panelState?.aheadCount || busy}
                icon={ArrowUpCircle}
                onClick={() => void runGitAction(() => cmd.gitPush(currentRepoPath))}
              />
            </div>
          </section>
        ) : null}
      </div>

      {banner ? (
        <div className={`git-banner git-banner--${banner.success ? "success" : "error"}`}>
          <div className="git-banner__dot" />
          <div className="git-banner__message">{banner.message}</div>
          <button className="git-banner__close" onClick={() => setBanner(null)} type="button">
            <X size={12} />
          </button>
        </div>
      ) : null}

      {!gitReady ? (
        <div className="git-panel__body">
          <GitEmptyState
            accent="var(--accent)"
            action={
              <GitButton
                tone="primary"
                disabled={busy}
                onClick={() =>
                  void runGitAction(() => cmd.gitInitRepo(browserPath), {
                    refresh: true,
                    successMessage: `Initialized a Git repository in ${repoName}.`,
                  })
                }
              >
                {t("Initialize Git")}
              </GitButton>
            }
            description={gitError || t("This folder is not initialized as a Git repository yet.")}
            icon={Folder}
            title={t("No repository")}
          />
        </div>
      ) : (
        <div className="git-panel__body">
          {selectedTab === "changes" ? (
            <div className="git-changes-wrap">
              <PanelGroup
                className="git-panel-group git-changes-group"
                defaultLayout={changesLayout}
                onLayoutChanged={persistChangesLayout}
                orientation="vertical"
              >
                <Panel
                  className="git-changes-pane"
                  collapsedSize={6}
                  collapsible
                  defaultSize={panelState?.stagedFiles.length ? 30 : 6}
                  id="staged"
                  minSize={6}
                >
                  <section className="git-surface git-file-section git-file-section--staged">
                    <div className="git-file-section__header">
                      <div className="git-file-section__title-wrap">
                        <span className="git-file-section__dot git-file-section__dot--success" />
                        <span className="git-file-section__title">{t("Staged")}</span>
                        <span className="git-file-section__count">{panelState?.stagedFiles.length ?? 0}</span>
                        <span className="git-file-section__help">{t("Files ready to commit")}</span>
                      </div>
                      {panelState?.stagedFiles.length ? (
                        <GitButton
                          compact
                          disabled={busy}
                          onClick={() => void runGitAction(() => cmd.gitUnstageAll(currentRepoPath))}
                        >
                          {t("Unstage all")}
                        </GitButton>
                      ) : null}
                    </div>
                    {panelState?.stagedFiles.length ? (
                      <div className="git-file-list">
                        {panelState.stagedFiles.map((file) => {
                          const active =
                            diffTarget?.kind === "working" && diffTarget.path === file.path && diffTarget.staged === true;
                          return (
                            <button
                              key={`staged-${file.path}`}
                              className={active ? "git-file-row git-file-row--active" : "git-file-row"}
                              onClick={() => openWorkingDiff({ path: file.path, staged: true, untracked: false })}
                              onContextMenu={(event) => openChangeFileMenu(event, file, true)}
                              type="button"
                            >
                              <span className={`git-status-badge git-status-badge--${statusToneFromCode(file.status)}`}>{file.status}</span>
                              <div className="git-file-row__copy">
                                <span className="git-file-row__name" title={file.fileName}>{file.fileName}</span>
                                {parentPathLabel(file.path) ? <span className="git-file-row__path" title={file.path}>{parentPathLabel(file.path)}</span> : null}
                              </div>
                              <GitFileDelta additions={file.additions} deletions={file.deletions} />
                              <button
                                className="git-file-row__action git-file-row__action--unstage"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  void runGitAction(() => cmd.gitUnstagePaths(currentRepoPath, [file.path]));
                                }}
                                type="button"
                              >
                                <Minus size={11} />
                              </button>
                            </button>
                          );
                        })}
                      </div>
                    ) : (
                      <div className="git-file-section__empty">
                        <span>{t("Stage changes to commit")}</span>
                      </div>
                    )}
                  </section>
                </Panel>

                <PanelResizeHandle className="git-split-handle git-split-handle--vertical" />

                <Panel
                  className="git-changes-pane"
                  defaultSize={45}
                  id="working"
                  minSize={20}
                >
                  <section className="git-surface git-file-section git-file-section--working">
                    <div className="git-file-section__header">
                      <div className="git-file-section__title-wrap">
                        <span className="git-file-section__dot git-file-section__dot--warning" />
                        <span className="git-file-section__title">{t("Working tree")}</span>
                        {panelState?.unstagedFiles.length ? <span className="git-file-section__count">{panelState.unstagedFiles.length}</span> : null}
                        <span className="git-file-section__help">{t("Modified and untracked files")}</span>
                      </div>
                      {panelState?.unstagedFiles.length ? (
                        <GitButton
                          compact
                          disabled={busy}
                          onClick={() => void runGitAction(() => cmd.gitStageAll(currentRepoPath))}
                        >
                          {t("Stage all")}
                        </GitButton>
                      ) : null}
                    </div>
                    {panelState?.unstagedFiles.length ? (
                      <div className="git-file-list">
                        {panelState.unstagedFiles.map((file) => {
                          const active =
                            diffTarget?.kind === "working" && diffTarget.path === file.path && diffTarget.staged === false;
                          return (
                            <button
                              key={`unstaged-${file.path}`}
                              className={active ? "git-file-row git-file-row--active" : "git-file-row"}
                              onClick={() =>
                                openWorkingDiff({
                                  path: file.path,
                                  staged: false,
                                  untracked: file.status === "?",
                                })
                              }
                              onContextMenu={(event) => openChangeFileMenu(event, file, false)}
                              type="button"
                            >
                              <span className={`git-status-badge git-status-badge--${statusToneFromCode(file.status)}`}>{file.status}</span>
                              <div className="git-file-row__copy">
                                <span className="git-file-row__name" title={file.fileName}>{file.fileName}</span>
                                {parentPathLabel(file.path) ? <span className="git-file-row__path" title={file.path}>{parentPathLabel(file.path)}</span> : null}
                              </div>
                              <GitFileDelta additions={file.additions} deletions={file.deletions} />
                              <button
                                className="git-file-row__action git-file-row__action--stage"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  void runGitAction(() => cmd.gitStagePaths(currentRepoPath, [file.path]));
                                }}
                                type="button"
                              >
                                <Plus size={11} />
                              </button>
                              {file.status !== "?" ? (
                                <button
                                  className="git-file-row__discard"
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    void runGitAction(() => cmd.gitDiscardPaths(currentRepoPath, [file.path]));
                                  }}
                                  type="button"
                                >
                                  <X size={11} />
                                </button>
                              ) : null}
                            </button>
                          );
                        })}
                      </div>
                    ) : (
                      <div className="git-file-section__empty git-file-section__empty--clean">
                        <Check size={11} />
                        <span>{t("Working tree clean")}</span>
                      </div>
                    )}
                  </section>
                </Panel>

                <PanelResizeHandle className="git-split-handle git-split-handle--vertical" />

                <Panel
                  className="git-changes-pane"
                  defaultSize={25}
                  id="commit"
                  minSize={18}
                >
                  <section className="git-surface git-commit-surface">
                    <GitSectionHeader
                      subtitle={
                        panelState?.stagedFiles.length
                          ? `${panelState.stagedFiles.length} ${t("staged file(s) ready to commit")}`
                          : t("Stage changes to enable commit")
                      }
                      title={t("Commit")}
                    />
                    <textarea
                      className="git-textarea git-textarea--mono git-commit-message"
                      onChange={(event) => setCommitMessage(event.currentTarget.value)}
                      placeholder={t("Write a focused commit message…")}
                      rows={3}
                      value={commitMessage}
                    />
                    <div className="git-commit-actions">
                      <label className="git-commit-check">
                        <input
                          type="checkbox"
                          checked={commitAmend}
                          onChange={(event) => setCommitAmend(event.currentTarget.checked)}
                        />
                        <span>{t("Amend")}</span>
                      </label>
                      <div className="git-commit-actions__spacer" />
                      <GitButton
                        disabled={!commitMessage.trim() || (!commitAmend && !panelState?.stagedFiles.length) || busy}
                        onClick={() =>
                          void runGitAction(() =>
                            cmd.gitCommit(currentRepoPath, commitMessage.trim(), {
                              amend: commitAmend,
                              sign: useSettingsStore.getState().gitCommitSigning,
                            }),
                          ).then(() => {
                            setCommitMessage("");
                            setCommitAmend(false);
                          })
                        }
                      >
                        {t("Commit")}
                      </GitButton>
                      <GitButton
                        tone="primary"
                        disabled={!commitMessage.trim() || (!commitAmend && !panelState?.stagedFiles.length) || busy}
                        onClick={() =>
                          void runGitAction(() =>
                            cmd.gitCommitAndPush(currentRepoPath, commitMessage.trim(), {
                              amend: commitAmend,
                              sign: useSettingsStore.getState().gitCommitSigning,
                            }),
                          ).then(() => {
                            setCommitMessage("");
                            setCommitAmend(false);
                          })
                        }
                      >
                        {t("Commit & Push")}
                      </GitButton>
                    </div>
                  </section>
                </Panel>
              </PanelGroup>
            </div>
          ) : null}

          {selectedTab === "history" ? (
            <div className="git-history">
              <section className="git-history-toolbar">
                <div className="git-history__filters">
                  <label className="git-search git-history__search">
                    <Search size={12} />
                    <input
                      onChange={(event) => setHistorySearchText(event.currentTarget.value)}
                      placeholder={t("Text or hash")}
                      value={historySearchText}
                    />
                    {historySearchText ? <button onClick={() => setHistorySearchText("")} type="button"><X size={11} /></button> : null}
                  </label>

                  <button
                    className={"git-history__filter-chip" + (historyBranchFilter ? " git-history__filter-chip--active" : "")}
                    onClick={openHistoryBranchFilter}
                    title={historyBranchFilter || t("All branches")}
                    type="button"
                  >
                    <GitBranch size={12} />
                    <span className="git-history__filter-label">{historyBranchFilter || t("All branches")}</span>
                    <ChevronDown size={10} />
                  </button>

                  <button
                    className={"git-history__filter-chip" + (historyAuthorFilter ? " git-history__filter-chip--active" : "")}
                    onClick={openHistoryAuthorFilter}
                    title={historyAuthorFilter || t("All authors")}
                    type="button"
                  >
                    <User size={12} />
                    <span className="git-history__filter-label">{historyAuthorFilter || t("User")}</span>
                    <ChevronDown size={10} />
                  </button>

                  <button
                    className={"git-history__filter-chip" + (historyDateFilter !== "all" ? " git-history__filter-chip--active" : "")}
                    onClick={(event) => openPopoverFromElement("historyDateFilter", event.currentTarget, 200)}
                    title={t("Date")}
                    type="button"
                  >
                    <Calendar size={12} />
                    <span className="git-history__filter-label">{historyDateFilterLabel()}</span>
                    <ChevronDown size={10} />
                  </button>

                  <button
                    className={"git-history__filter-chip" + (historyPaths.length ? " git-history__filter-chip--active" : "")}
                    onClick={openHistoryPathFilter}
                    title={historyPathSummary()}
                    type="button"
                  >
                    <Folder size={12} />
                    <span className="git-history__filter-label">{historyPathSummary()}</span>
                    <ChevronDown size={10} />
                  </button>

                  {historyPaths.length ? (
                    <GitIconButton className="git-history__toolbar-icon" aria-label={t("Clear path filter")} icon={X} onClick={() => setHistoryPaths([])} />
                  ) : null}
                  <GitIconButton
                    className="git-history__toolbar-icon"
                    active={popover?.kind === "historyOptions"}
                    aria-label={t("History options")}
                    icon={Settings2}
                    onClick={(event) => openPopoverFromElement("historyOptions", event.currentTarget, 228)}
                  />
                  <GitIconButton className="git-history__toolbar-icon" aria-label={t("Reload graph")} icon={RefreshCw} onClick={() => void loadGraphRows()} />
                </div>
              </section>

              <section
                className={
                  "git-surface git-history-list-surface" +
                  (historyLoading && graphRows.length ? " is-reloading" : "")
                }
                ref={(el) => { historyListSurfaceRef.current = el; }}
              >
                {graphRows.length ? (
                  <>
                    <GitHistoryVirtualList
                      className="git-history-list"
                      scrollRef={historyListRef}
                      onScroll={handleHistoryListScroll}
                      rows={graphRows}
                      rowHeight={GRAPH_ROW_H}
                      selectedIndex={historySelectedIndex}
                      detailNode={activeCommitDetail ? renderHistoryInlineDetail(activeCommitDetail) : null}
                      renderRow={(row, index) => {
                        const active = row.hash === historySelectedHash;
                        const meta = rowMetaByHash.get(row.hash);
                        const refs = meta?.refs ?? [];
                        const dimmed = meta?.dimmed ?? false;
                        const zebraStripe = historyShowZebraStripes && index % 2 === 1;
                        return (
                          <HistoryRow
                            row={row}
                            active={active}
                            dimmed={dimmed}
                            zebraStripe={zebraStripe}
                            refs={refs}
                            isHead={meta?.isHead ?? false}
                            graphLaneWidth={graphLaneWidth}
                            authorColorValue={meta?.authorColorValue ?? ""}
                            authorInitialValue={meta?.authorInitialValue ?? ""}
                            formattedDate={meta?.formattedDate ?? ""}
                            showAuthor={historyShowAuthor}
                            showDate={historyShowDate}
                            showHash={historyShowHash}
                            titleText={`${row.shortHash} · ${row.message}`}
                            surfaceRef={historyListSurfaceRef}
                            colWidthsRef={historyColWidthsRef}
                            onPersistCol={persistHistoryCol}
                            onSelect={handleHistorySelect}
                            onDoubleClick={handleHistoryDoubleClick}
                            onContextMenu={handleHistoryContextMenu}
                          />
                        );
                      }}
                      footer={
                        historyLoadingMore ? (
                          <div className="git-history-list__loading-more mono">{t("Loading more…")}</div>
                        ) : !historyHasMore && graphRows.length > 0 ? (
                          <div className="git-history-list__end mono">{t("End of history")}</div>
                        ) : null
                      }
                    />
                  </>
                ) : historyLoading ? (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={t("Loading commit graph…")}
                    icon={History}
                    title={t("Loading")}
                  />
                ) : (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={t("Adjust branch, author, date, path, or message filters to load commit graph data.")}
                    icon={History}
                    title={t("No history matches")}
                  />
                )}
              </section>
            </div>
          ) : null}

          <GitPopover kind="branchManager" onClose={() => setPopover(null)} popover={popover}>
            <div className="git-branches-view git-branches-view--popover">
              <div className="git-branches-view__header">
                <GitSectionHeader
                  actions={
                    <>
                      <GitIconButton
                        active={branchCreateExpanded}
                        aria-label={branchCreateExpanded ? t("Hide composer") : t("New branch")}
                        icon={branchCreateExpanded ? X : Plus}
                        onClick={() => setBranchCreateExpanded((value) => !value)}
                      />
                      <GitIconButton aria-label={t("Reload branches")} icon={RefreshCw} onClick={() => void loadGraphMetadata()} />
                    </>
                  }
                  title={t("Local & remote")}
                />
                <div className="git-segmented">
                  <button className={branchManagerMode === "local" ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setBranchManagerMode("local")} type="button">{t("Local")}</button>
                  <button className={branchManagerMode === "remote" ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setBranchManagerMode("remote")} type="button">{t("Remote")}</button>
                </div>
                <label className="git-search">
                  <Search size={12} />
                  <input
                    onChange={(event) => setBranchManagerSearchText(event.currentTarget.value)}
                    placeholder={t("Filter branches")}
                    value={branchManagerSearchText}
                  />
                  {branchManagerSearchText ? <button onClick={() => setBranchManagerSearchText("")} type="button"><X size={11} /></button> : null}
                </label>
              </div>

              <div className="git-branches-view__body">
                {branchManagerMode === "local" && branchCreateExpanded ? (
                  <div className="git-card git-card--inset">
                    <GitSectionHeader subtitle={t("Create a local branch from the current HEAD")} title={t("Create branch")} />
                    <div className="git-inline-form">
                      <input className="git-input" onChange={(event) => setBranchDraftName(event.currentTarget.value)} placeholder={t("Branch name")} value={branchDraftName} />
                      <GitButton
                        tone="primary"
                        compact
                        disabled={!branchDraftName.trim() || busy}
                        onClick={() =>
                          void runGitAction(() => cmd.gitCreateBranch(currentRepoPath, branchDraftName.trim())).then(() =>
                            setBranchDraftName(""),
                          )
                        }
                      >
                        {t("Create")}
                      </GitButton>
                    </div>
                    <div className="git-manager__divider" />
                    <GitSectionHeader subtitle={t("Set or remove upstream for a local branch")} title={t("Tracking")} />
                    <div className="git-inline-form">
                      <select className="git-select" onChange={(event) => setTrackingBranchTarget(event.currentTarget.value)} value={trackingBranchTarget}>
                        {localBranches.map((branch) => (
                          <option key={branch} value={branch}>{branch}</option>
                        ))}
                      </select>
                      <select className="git-select" onChange={(event) => setTrackingUpstreamTarget(event.currentTarget.value)} value={trackingUpstreamTarget}>
                        {remoteBranches.map((branch) => (
                          <option key={branch} value={branch}>{branch}</option>
                        ))}
                      </select>
                    </div>
                    <div className="git-inline-form">
                      <GitButton compact disabled={!trackingBranchTarget || busy} onClick={() => void runGitAction(() => cmd.gitUnsetBranchTracking(currentRepoPath, trackingBranchTarget))}>
                        {t("Unset")}
                      </GitButton>
                      <div className="git-commit-actions__spacer" />
                      <GitButton
                        tone="primary"
                        compact
                        disabled={!trackingBranchTarget || !trackingUpstreamTarget || busy}
                        onClick={() =>
                          void runGitAction(() => cmd.gitSetBranchTracking(currentRepoPath, trackingBranchTarget, trackingUpstreamTarget))
                        }
                      >
                        {t("Set tracking")}
                      </GitButton>
                    </div>
                  </div>
                ) : null}

                {branchManagerMode === "local" ? (
                  <>
                    <GitSectionHeader subtitle={`${filteredManagerLocalBranches.length} ${t("branches")}`} title={t("Local branches")} />
                    <div className="git-manager-list">
                      {filteredManagerLocalBranches.length ? (
                        filteredManagerLocalBranches.map((branch) => {
                          const current = branch === panelState?.currentBranch;
                          const renameMode = branchRenameSource === branch;
                          return (
                            <div className="git-manager-row" key={branch}>
                              <span className={`git-manager-row__dot ${current ? "git-manager-row__dot--success" : ""}`} />
                              <div className="git-manager-row__copy">
                                {renameMode ? (
                                  <div className="git-inline-form">
                                    <input className="git-input" onChange={(event) => setBranchRenameTarget(event.currentTarget.value)} placeholder={t("Rename branch")} value={branchRenameTarget} />
                                    <GitButton compact onClick={() => { setBranchRenameSource(""); setBranchRenameTarget(""); }}>{t("Cancel")}</GitButton>
                                    <GitButton
                                      tone="primary"
                                      compact
                                      disabled={!branchRenameTarget.trim()}
                                      onClick={() =>
                                        void runGitAction(() => cmd.gitRenameBranch(currentRepoPath, branch, branchRenameTarget.trim())).then(() => {
                                          setBranchRenameSource("");
                                          setBranchRenameTarget("");
                                        })
                                      }
                                    >
                                      {t("Save")}
                                    </GitButton>
                                  </div>
                                ) : (
                                  <>
                                    <div className="git-manager-row__title">{branch}</div>
                                    {current && panelState?.trackingBranch ? <div className="git-manager-row__subtitle">{`${t("Tracking")} ${panelState.trackingBranch}`}</div> : null}
                                  </>
                                )}
                              </div>
                              {current ? <GitPill tone="success">{t("Current")}</GitPill> : null}
                              {!renameMode ? (
                                <div className="git-manager-row__actions">
                                  {!current ? (
                                    <GitIconButton aria-label={t("Switch")} icon={ArrowRight} onClick={() => void runGitAction(() => cmd.gitCheckoutBranch(currentRepoPath, branch))} title={t("Switch to this branch")} />
                                  ) : null}
                                  {!current ? (
                                    <GitIconButton aria-label={t("Merge")} icon={GitMerge} onClick={() => void runGitAction(() => cmd.gitMergeBranch(currentRepoPath, branch))} title={t("Merge into current branch")} />
                                  ) : null}
                                  <GitIconButton aria-label={t("Rename")} icon={Pencil} onClick={() => { setBranchRenameSource(branch); setBranchRenameTarget(branch); }} title={t("Rename branch")} />
                                  {!current ? (
                                    <GitIconButton aria-label={t("Delete")} className="is-danger" icon={Trash2} onClick={() => void runGitAction(() => cmd.gitDeleteBranch(currentRepoPath, branch))} title={t("Delete branch")} />
                                  ) : null}
                                </div>
                              ) : null}
                            </div>
                          );
                        })
                      ) : (
                        <GitEmptyState accent="var(--accent)" description={t("Create a branch to start parallel workstreams.")} icon={GitBranch} title={t("No local branches")} />
                      )}
                    </div>
                  </>
                ) : (
                  <>
                    <GitSectionHeader subtitle={`${filteredManagerRemoteBranches.length} ${t("refs")}`} title={t("Remote branches")} />
                    <div className="git-manager-list">
                      {filteredManagerRemoteBranches.length ? (
                        filteredManagerRemoteBranches.map((branch) => {
                          const renameMode = branchRenameSource === branch;
                          return (
                            <div className="git-manager-row" key={branch}>
                              <span className="git-manager-row__dot git-manager-row__dot--accent" />
                              <div className="git-manager-row__copy">
                                {renameMode ? (
                                  <div className="git-inline-form">
                                    <input className="git-input" onChange={(event) => setBranchRenameTarget(event.currentTarget.value)} placeholder={t("Rename branch")} value={branchRenameTarget} />
                                    <GitButton compact onClick={() => { setBranchRenameSource(""); setBranchRenameTarget(""); }}>{t("Cancel")}</GitButton>
                                    <GitButton
                                      tone="primary"
                                      compact
                                      disabled={!branchRenameTarget.trim()}
                                      onClick={() => {
                                        const parts = branch.split("/");
                                        const remoteName = parts.shift() || "origin";
                                        const remoteBranch = parts.join("/");
                                        void runGitAction(() =>
                                          cmd.gitRenameRemoteBranch(currentRepoPath, remoteName, remoteBranch, branchRenameTarget.trim()),
                                        ).then(() => {
                                          setBranchRenameSource("");
                                          setBranchRenameTarget("");
                                        });
                                      }}
                                    >
                                      {t("Save")}
                                    </GitButton>
                                  </div>
                                ) : (
                                  <div className="git-manager-row__title">{branch}</div>
                                )}
                              </div>
                              {!renameMode ? (
                                <div className="git-manager-row__actions">
                                  <GitIconButton
                                    aria-label={t("Checkout")}
                                    icon={Download}
                                    onClick={() =>
                                      void runGitAction(() => cmd.gitCheckoutTarget(currentRepoPath, branch.replace(/^[^/]+\//, ""), branch))
                                    }
                                    title={t("Checkout local copy")}
                                  />
                                  <GitIconButton
                                    aria-label={t("Rename")}
                                    icon={Pencil}
                                    onClick={() => { setBranchRenameSource(branch); setBranchRenameTarget(branch.replace(/^[^/]+\//, "")); }}
                                    title={t("Rename remote branch")}
                                  />
                                  <GitIconButton
                                    aria-label={t("Delete")}
                                    className="is-danger"
                                    icon={Trash2}
                                    onClick={() => {
                                      const parts = branch.split("/");
                                      const remoteName = parts.shift() || "origin";
                                      const remoteBranch = parts.join("/");
                                      void runGitAction(() => cmd.gitDeleteRemoteBranch(currentRepoPath, remoteName, remoteBranch));
                                    }}
                                    title={t("Delete remote branch")}
                                  />
                                </div>
                              ) : null}
                            </div>
                          );
                        })
                      ) : (
                        <GitEmptyState accent="var(--accent)" description={t("Remote refs will appear here after fetch or clone.")} icon={GitBranch} title={t("No remote branches")} />
                      )}
                    </div>
                  </>
                )}
              </div>
            </div>
          </GitPopover>

          {selectedTab === "stash" ? (
            <div className="git-stash-view">
              <section className="git-card git-card--inset git-stash-composer">
                <GitSectionHeader
                  subtitle={stashes.length ? `${stashes.length} ${t("entries")}` : t("Snapshot unfinished work")}
                  title={t("Stash")}
                />
                <div className="git-inline-form git-stash-composer__form">
                  <input
                    className="git-input git-stash-composer__input"
                    onChange={(event) => setStashMessage(event.currentTarget.value)}
                    placeholder={t("Optional stash label")}
                    value={stashMessage}
                  />
                  <GitButton
                    compact
                    disabled={workingTreeClean || busy}
                    onClick={() =>
                      void runGitAction(() => cmd.gitStashPush(currentRepoPath, stashMessage), { stash: true }).then(() =>
                        setStashMessage(""),
                      )
                    }
                  >
                    {t("Stash")}
                  </GitButton>
                </div>
              </section>
              <section className="git-surface git-stash-list-surface">
                <div className="git-file-section__header">
                  <div className="git-file-section__title-wrap">
                    <span className="git-file-section__dot git-file-section__dot--accent" />
                    <span className="git-file-section__title">{t("Saved stashes")}</span>
                    {stashes.length ? <span className="git-file-section__count">{stashes.length}</span> : null}
                    <span className="git-file-section__help">{t("Apply, pop, or drop a snapshot")}</span>
                  </div>
                </div>
                {stashes.length ? (
                  <div className="git-stash-list">
                    {stashes.map((stash) => {
                      const editing = stashEditingIndex === stash.index;
                      const beginEdit = () => {
                        setStashEditingIndex(stash.index);
                        setStashEditingDraft(stash.message);
                      };
                      const commitEdit = () => {
                        const next = stashEditingDraft.trim();
                        const original = stash.message.trim();
                        setStashEditingIndex(null);
                        if (!next || next === original) return;
                        void runGitAction(
                          () => cmd.gitStashReword(currentRepoPath, stash.index, next),
                          { stash: true },
                        );
                      };
                      return (
                        <div
                          key={stash.index}
                          className={"git-stash-row" + (editing ? " is-editing" : "")}
                          onContextMenu={(event) => {
                            event.preventDefault();
                            openPopoverAt("stashMenu", event.clientX, event.clientY, 188, stash);
                          }}
                        >
                          <div className="git-stash-row__copy">
                            {editing ? (
                              <input
                                autoFocus
                                className="git-input git-stash-row__edit"
                                onBlur={commitEdit}
                                onChange={(event) => setStashEditingDraft(event.currentTarget.value)}
                                onKeyDown={(event) => {
                                  if (event.key === "Enter") {
                                    event.preventDefault();
                                    commitEdit();
                                  } else if (event.key === "Escape") {
                                    event.preventDefault();
                                    setStashEditingIndex(null);
                                  }
                                }}
                                placeholder={t("Stash message")}
                                value={stashEditingDraft}
                              />
                            ) : (
                              <button
                                className="git-stash-row__message"
                                onDoubleClick={beginEdit}
                                title={t("Double-click to rename")}
                                type="button"
                              >
                                {stash.message || "WIP"}
                              </button>
                            )}
                            <div className="git-stash-row__meta">{`stash@{${stash.index}} · ${stash.relativeDate}`}</div>
                          </div>
                          <div className="git-stash-row__actions">
                            <GitButton compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitStashApply(currentRepoPath, stash.index), { stash: true })}>
                              {t("Apply")}
                            </GitButton>
                            <GitButton compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitStashPop(currentRepoPath, stash.index), { stash: true })}>
                              {t("Pop")}
                            </GitButton>
                            <GitButton tone="destructive" compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitStashDrop(currentRepoPath, stash.index), { stash: true })}>
                              {t("Drop")}
                            </GitButton>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                ) : (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={t("Use stash to park unfinished work without leaving the current branch.")}
                    icon={Archive}
                    title={t("No stashes")}
                  />
                )}
              </section>
            </div>
          ) : null}

          {selectedTab === "conflicts" ? (
            <div className="git-conflicts">
              <section className="git-card git-card--inset git-conflicts-summary">
                <GitSectionHeader
                  actions={
                    <>
                      <GitPill tone={conflicts.length ? "warning" : "success"}>
                        {conflicts.length ? `${conflicts.length} ${t("open")}` : t("Clean")}
                      </GitPill>
                      <GitIconButton aria-label={t("Reload conflicts")} icon={RefreshCw} onClick={() => void loadConflicts()} />
                    </>
                  }
                  subtitle={
                    conflicts.length
                      ? `${conflicts.length} ${t("conflicted file(s)")}`
                      : t("Files requiring manual merge resolution")
                  }
                  title={t("Conflicts")}
                />
              </section>
              <section className="git-surface git-conflicts-surface">
                {conflicts.length ? (
                  <PanelGroup className="git-panel-group" orientation="horizontal">
                    <Panel defaultSize={36} minSize={28}>
                      <div className="git-conflict-files">
                        <div className="git-conflict-files__head">
                          <GitSectionHeader subtitle={`${conflicts.length} ${t("open")}`} title={t("Files")} />
                        </div>
                        <div className="git-conflict-files__list">
                          {conflicts.map((file) => (
                            <button
                              key={file.path}
                              className={file.path === selectedConflictFile?.path ? "git-conflict-file git-conflict-file--active" : "git-conflict-file"}
                              onClick={() => {
                                setSelectedConflictPath(file.path);
                                openWorkingDiff({ path: file.path, staged: false, untracked: false });
                              }}
                              type="button"
                            >
                              <span className="git-conflict-file__dot" />
                              <div className="git-conflict-file__copy">
                                <span className="git-conflict-file__name" title={file.name}>{file.name}</span>
                                <span className="git-conflict-file__path" title={file.path}>{parentPathLabel(file.path) || file.path}</span>
                              </div>
                              <GitPill tone="warning">{file.conflictCount}</GitPill>
                              <GitButton
                                compact
                                onClick={(event) => {
                                  event.stopPropagation();
                                  void runGitAction(() => cmd.gitStagePaths(currentRepoPath, [file.path]), { conflicts: true });
                                }}
                              >
                                {t("Stage")}
                              </GitButton>
                            </button>
                          ))}
                        </div>
                      </div>
                    </Panel>
                    <PanelResizeHandle className="git-split-handle git-split-handle--horizontal" />
                    <Panel defaultSize={64} minSize={36}>
                      <div className="git-conflict-detail">
                        {selectedConflictFile ? (
                          <>
                            <section className="git-conflict-detail__header">
                              <GitSectionHeader
                                actions={<GitPill tone="warning">{`${selectedConflictHunks.length} ${t("hunks")}`}</GitPill>}
                                subtitle={selectedConflictFile.path}
                                title={selectedConflictFile.name || t("Resolution")}
                              />
                            </section>

                            <div className="git-conflict-detail__actions">
                              <GitButton
                                compact
                                onClick={() =>
                                  openWorkingDiff({ path: selectedConflictFile.path, staged: false, untracked: false })
                                }
                              >
                                {t("Diff")}
                              </GitButton>
                              <GitButton
                                compact
                                disabled={busy}
                                onClick={() =>
                                  void runGitAction(() => cmd.gitConflictAcceptAll(currentRepoPath, selectedConflictFile.path, "ours"), {
                                    conflicts: true,
                                  })
                                }
                              >
                                {t("Accept all ours")}
                              </GitButton>
                              <GitButton
                                compact
                                disabled={busy}
                                onClick={() =>
                                  void runGitAction(() => cmd.gitConflictAcceptAll(currentRepoPath, selectedConflictFile.path, "theirs"), {
                                    conflicts: true,
                                  })
                                }
                              >
                                {t("Accept all theirs")}
                              </GitButton>
                              <div className="git-commit-actions__spacer" />
                              <GitButton
                                tone="primary"
                                compact
                                disabled={busy}
                                onClick={() =>
                                  void runGitAction(
                                    () =>
                                      cmd.gitConflictMarkResolved(
                                        currentRepoPath,
                                        selectedConflictFile.path,
                                        selectedConflictHunks,
                                      ),
                                    { conflicts: true },
                                  )
                                }
                              >
                                {t("Mark resolved")}
                              </GitButton>
                            </div>

                            {selectedConflictHunks.length > 0 &&
                            selectedConflictHunks.every((h) => !h.hasBase) ? (
                              <div
                                className="git-inline-note"
                                style={{
                                  margin: "var(--sp-2) 0",
                                  display: "flex",
                                  alignItems: "center",
                                  gap: "var(--sp-2)",
                                }}
                              >
                                <span style={{ flex: 1 }}>
                                  {t("Base column hidden — enable diff3 style to see the common ancestor in future conflicts.")}
                                </span>
                                <GitButton
                                  compact
                                  onClick={() =>
                                    void runGitAction(
                                      () =>
                                        cmd.gitSetConfigValue(
                                          currentRepoPath,
                                          "merge.conflictStyle",
                                          "diff3",
                                          false,
                                        ),
                                      { config: true, successMessage: t("Enabled diff3 for this repo") },
                                    )
                                  }
                                >
                                  {t("Enable diff3")}
                                </GitButton>
                              </div>
                            ) : null}

                            <div className="git-conflict-hunks">
                              {selectedConflictHunks.map((hunk, index) => (
                                <div key={`${selectedConflictFile.path}-hunk-${index}`} className="git-card git-card--inset git-conflict-hunk">
                                  <GitSectionHeader
                                    actions={
                                      hunk.resolution ? (
                                        <GitPill
                                          tone={
                                            hunk.resolution === "theirs"
                                              ? "info"
                                              : hunk.resolution === "both"
                                                ? "warning"
                                                : hunk.resolution === "base"
                                                  ? "info"
                                                  : "success"
                                          }
                                        >
                                          {hunk.resolution === "theirs"
                                            ? t("Theirs")
                                            : hunk.resolution === "both"
                                              ? t("Both")
                                              : hunk.resolution === "base"
                                                ? t("Base")
                                                : t("Ours")}
                                        </GitPill>
                                      ) : null
                                    }
                                    subtitle={
                                      hunk.resolution
                                        ? `${t("Selected")}: ${hunk.resolution}`
                                        : t("Choose a resolution for this hunk")
                                    }
                                    title={`${t("Conflict")} ${index + 1}`}
                                  />
                                  <div
                                    className={
                                      hunk.hasBase
                                        ? "git-conflict-hunk__columns git-conflict-hunk__columns--with-base"
                                        : "git-conflict-hunk__columns"
                                    }
                                  >
                                    <div className="git-conflict-hunk__side git-conflict-hunk__side--ours">
                                      <div className="git-conflict-hunk__label">{t("Ours")}</div>
                                      {hunk.oursLines.map((line, lineIndex) => (
                                        <div key={`ours-${lineIndex}-${line}`} className="git-conflict-hunk__line">
                                          {line || " "}
                                        </div>
                                      ))}
                                    </div>
                                    {hunk.hasBase ? (
                                      <div className="git-conflict-hunk__side git-conflict-hunk__side--base">
                                        <div className="git-conflict-hunk__label">{t("Base")}</div>
                                        {hunk.baseLines.length === 0 ? (
                                          <div className="git-conflict-hunk__line text-muted">
                                            {t("(no lines in common ancestor)")}
                                          </div>
                                        ) : (
                                          hunk.baseLines.map((line, lineIndex) => (
                                            <div
                                              key={`base-${lineIndex}-${line}`}
                                              className="git-conflict-hunk__line"
                                            >
                                              {line || " "}
                                            </div>
                                          ))
                                        )}
                                      </div>
                                    ) : null}
                                    <div className="git-conflict-hunk__side git-conflict-hunk__side--theirs">
                                      <div className="git-conflict-hunk__label">{t("Theirs")}</div>
                                      {hunk.theirsLines.map((line, lineIndex) => (
                                        <div key={`theirs-${lineIndex}-${line}`} className="git-conflict-hunk__line">
                                          {line || " "}
                                        </div>
                                      ))}
                                    </div>
                                  </div>
                                  <div className="git-conflict-hunk__actions">
                                    <GitButton
                                      compact
                                      onClick={() =>
                                        setConflictDrafts((current) => {
                                          const next = { ...current };
                                          const items = [...(next[selectedConflictFile.path] || selectedConflictFile.conflicts)];
                                          items[index] = { ...items[index], resolution: "ours" };
                                          next[selectedConflictFile.path] = items;
                                          return next;
                                        })
                                      }
                                    >
                                      {t("Accept ours")}
                                    </GitButton>
                                    <GitButton
                                      compact
                                      onClick={() =>
                                        setConflictDrafts((current) => {
                                          const next = { ...current };
                                          const items = [...(next[selectedConflictFile.path] || selectedConflictFile.conflicts)];
                                          items[index] = { ...items[index], resolution: "theirs" };
                                          next[selectedConflictFile.path] = items;
                                          return next;
                                        })
                                      }
                                    >
                                      {t("Accept theirs")}
                                    </GitButton>
                                    <GitButton
                                      compact
                                      onClick={() =>
                                        setConflictDrafts((current) => {
                                          const next = { ...current };
                                          const items = [...(next[selectedConflictFile.path] || selectedConflictFile.conflicts)];
                                          items[index] = { ...items[index], resolution: "both" };
                                          next[selectedConflictFile.path] = items;
                                          return next;
                                        })
                                      }
                                    >
                                      {t("Accept both")}
                                    </GitButton>
                                    {hunk.hasBase ? (
                                      <GitButton
                                        compact
                                        onClick={() =>
                                          setConflictDrafts((current) => {
                                            const next = { ...current };
                                            const items = [
                                              ...(next[selectedConflictFile.path] || selectedConflictFile.conflicts),
                                            ];
                                            items[index] = { ...items[index], resolution: "base" };
                                            next[selectedConflictFile.path] = items;
                                            return next;
                                          })
                                        }
                                      >
                                        {t("Accept base")}
                                      </GitButton>
                                    ) : null}
                                  </div>
                                </div>
                              ))}
                            </div>
                          </>
                        ) : (
                          <GitEmptyState
                            accent="var(--status-warning)"
                            description={t("Choose a conflicted file to inspect ours, theirs, and apply a resolution.")}
                            icon={GitMerge}
                            title={t("Select a conflict")}
                          />
                        )}
                      </div>
                    </Panel>
                  </PanelGroup>
                ) : (
                  <GitEmptyState
                    accent="var(--status-success)"
                    description={t("Conflicted files will appear here when Git requires manual resolution.")}
                    icon={Check}
                    title={t("No merge conflicts")}
                  />
                )}
              </section>
            </div>
          ) : null}
        </div>
      )}

      <GitPopover kind="branchMenu" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-list">
          {branchMenuBranches.map((branch) => (
            <GitMenuItem
              active={branch === panelState?.currentBranch}
              key={branch}
              onClick={() => {
                setPopover(null);
                void runGitAction(() => cmd.gitCheckoutBranch(currentRepoPath, branch));
              }}
            >
              {branch}
            </GitMenuItem>
          ))}
        </div>
      </GitPopover>

      <GitPopover kind="historyBranchFilter" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-list git-popover-list--scroll">
          <GitMenuItem
            active={!historyBranchFilter}
            onClick={() => { setHistoryBranchFilter(""); setPopover(null); }}
          >
            {t("All branches")}
          </GitMenuItem>
          {(graphMetadata?.branches || []).map((branch) => (
            <GitMenuItem
              active={historyBranchFilter === branch}
              key={branch}
              onClick={() => { setHistoryBranchFilter(branch); setPopover(null); }}
            >
              {branch}
            </GitMenuItem>
          ))}
        </div>
      </GitPopover>

      <GitPopover kind="historyAuthorFilter" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-list git-popover-list--scroll">
          <GitMenuItem
            active={!historyAuthorFilter}
            onClick={() => { setHistoryAuthorFilter(""); setPopover(null); }}
          >
            {t("All authors")}
          </GitMenuItem>
          {(graphMetadata?.authors || []).map((author) => (
            <GitMenuItem
              active={historyAuthorFilter === author}
              key={author}
              onClick={() => { setHistoryAuthorFilter(author); setPopover(null); }}
            >
              {author}
            </GitMenuItem>
          ))}
        </div>
      </GitPopover>

      <GitPopover kind="historyDateFilter" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-list">
          {[
            { value: "all", label: t("Any time") },
            { value: "7d", label: t("Last 7 days") },
            { value: "30d", label: t("Last 30 days") },
            { value: "90d", label: t("Last 90 days") },
            { value: "365d", label: t("Last year") },
          ].map((option) => (
            <GitMenuItem
              active={historyDateFilter === option.value}
              key={option.value}
              onClick={() => { setHistoryDateFilter(option.value); setPopover(null); }}
            >
              {option.label}
            </GitMenuItem>
          ))}
        </div>
      </GitPopover>

      <GitPopover kind="historyOptions" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Sort")}</div>
          <GitMenuItem checkable active={historySortMode === "topo"} onClick={() => setHistorySortMode("topo")}>{t("Topology order")}</GitMenuItem>
          <GitMenuItem checkable active={historySortMode === "date"} onClick={() => setHistorySortMode("date")}>{t("Date order")}</GitMenuItem>
        </div>
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Graph options")}</div>
          <GitMenuItem checkable active={historyFirstParent} onClick={() => setHistoryFirstParent((value) => !value)}>{t("First parent only")}</GitMenuItem>
          <GitMenuItem checkable active={historyNoMerges} onClick={() => setHistoryNoMerges((value) => !value)}>{t("Hide merge commits")}</GitMenuItem>
          <GitMenuItem checkable active={historyShowLongEdges} onClick={() => setHistoryShowLongEdges((value) => !value)}>{t("Expand long edges")}</GitMenuItem>
        </div>
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Highlight")}</div>
          <GitMenuItem checkable active={historyHighlightMode === "none"} onClick={() => setHistoryHighlightMode("none")}>{t("No highlight")}</GitMenuItem>
          <GitMenuItem checkable active={historyHighlightMode === "mine"} onClick={() => setHistoryHighlightMode("mine")}>{t("My commits")}</GitMenuItem>
          <GitMenuItem checkable active={historyHighlightMode === "merge"} onClick={() => setHistoryHighlightMode("merge")}>{t("Merge commits")}</GitMenuItem>
          <GitMenuItem checkable active={historyHighlightMode === "branch"} onClick={() => setHistoryHighlightMode("branch")}>{t("Current branch")}</GitMenuItem>
        </div>
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Display")}</div>
          <GitMenuItem checkable active={historyShowZebraStripes} onClick={() => setHistoryShowZebraStripes((value) => !value)}>{t("Zebra stripes")}</GitMenuItem>
          <GitMenuItem checkable active={historyShowHash} onClick={() => setHistoryShowHash((value) => !value)}>{t("Show hash column")}</GitMenuItem>
          <GitMenuItem checkable active={historyShowAuthor} onClick={() => setHistoryShowAuthor((value) => !value)}>{t("Show author column")}</GitMenuItem>
          <GitMenuItem checkable active={historyShowDate} onClick={() => setHistoryShowDate((value) => !value)}>{t("Show date column")}</GitMenuItem>
        </div>
        <div className="git-popover-section">
          <GitMenuItem
            onClick={() => {
              setPopover(null);
              setReflogDialogOpen(true);
              setReflogLoading(true);
              setReflogError("");
              void cmd
                .gitReflogList(currentRepoPath, 200)
                .then((entries) => {
                  setReflogEntries(entries);
                })
                .catch((err) => {
                  setReflogEntries([]);
                  setReflogError(String(err));
                })
                .finally(() => setReflogLoading(false));
            }}
          >
            {t("Show reflog")}
          </GitMenuItem>
        </div>
      </GitPopover>

      <GitPopover kind="changeFileMenu" onClose={() => setPopover(null)} popover={popover}>
        {popover?.kind === "changeFileMenu" ? (
          <div className="git-popover-list">
            <GitMenuItem
              onClick={() => {
                const { file, staged } = popover.data as ChangeFileMenuState;
                setPopover(null);
                openWorkingDiff({ path: file.path, staged, untracked: !staged && file.status === "?" });
              }}
            >
              {t("Show diff")}
            </GitMenuItem>
            <GitMenuItem
              onClick={() => {
                const { file } = popover.data as ChangeFileMenuState;
                setPopover(null);
                setBlameDialogOpen(true);
                setBlameFilePath(file.path);
                void cmd
                  .gitBlameFile(currentRepoPath, file.path)
                  .then((next) => setBlameLines(next))
                  .catch(() => setBlameLines([]));
              }}
            >
              {t("Blame")}
            </GitMenuItem>
            <div className="git-popover-divider" />
            {(popover.data as ChangeFileMenuState).staged ? (
              <GitMenuItem
                onClick={() => {
                  const { file } = popover.data as ChangeFileMenuState;
                  setPopover(null);
                  void runGitAction(() => cmd.gitUnstagePaths(currentRepoPath, [file.path]));
                }}
              >
                {t("Unstage")}
              </GitMenuItem>
            ) : (
              <>
                <GitMenuItem
                  onClick={() => {
                    const { file } = popover.data as ChangeFileMenuState;
                    setPopover(null);
                    void runGitAction(() => cmd.gitStagePaths(currentRepoPath, [file.path]));
                  }}
                >
                  {t("Stage")}
                </GitMenuItem>
                {(popover.data as ChangeFileMenuState).file.status !== "?" ? (
                  <GitMenuItem
                    destructive
                    onClick={() => {
                      const { file } = popover.data as ChangeFileMenuState;
                      setPopover(null);
                      void runGitAction(() => cmd.gitDiscardPaths(currentRepoPath, [file.path]));
                    }}
                  >
                    {t("Discard changes")}
                  </GitMenuItem>
                ) : null}
              </>
            )}
          </div>
        ) : null}
      </GitPopover>

      <GitPopover kind="historyCommit" onClose={() => setPopover(null)} popover={popover}>
        {historyContextCommit ? (
          <div className="git-popover-list">
            <GitMenuItem onClick={() => void copyText(historyContextCommit.hash)}>{t("Copy hash")}</GitMenuItem>
            <GitMenuItem
              onClick={() => {
                setPopover(null);
                void runGitAction(() => cmd.gitCheckoutTarget(currentRepoPath, historyContextCommit.hash));
              }}
            >
              {t("Checkout this revision")}
            </GitMenuItem>
            {historyContextCheckoutTargets(historyContextCommit).slice(1).map((target) => (
              <GitMenuItem
                key={`${target.target}-${target.tracking || ""}`}
                onClick={() => {
                  setPopover(null);
                  void runGitAction(() => cmd.gitCheckoutTarget(currentRepoPath, target.target, target.tracking || null));
                }}
              >
                {target.label}
              </GitMenuItem>
            ))}
            <GitMenuItem
              onClick={() => {
                setPopover(null);
                setComparisonBaseHash(historyContextCommit.hash);
                void cmd
                  .gitComparisonFiles(currentRepoPath, historyContextCommit.hash)
                  .then((files) => {
                    setComparisonFiles(files);
                    setHistoryCompareDialogOpen(true);
                  })
                  .catch((error) => showBanner(false, extractErrorMessage(error, t)));
              }}
            >
              {t("Compare with local")}
            </GitMenuItem>
            <GitMenuItem
              disabled={!browserUrlForCommit(historyContextCommit.hash)}
              onClick={() => {
                setPopover(null);
                void openCommitInBrowser(historyContextCommit.hash);
              }}
            >
              {t("Open in browser")}
            </GitMenuItem>
            <div className="git-popover-divider" />
            <GitMenuItem
              onClick={() => {
                setPopover(null);
                setHistoryBranchDraftName("");
                setHistoryBranchDialogOpen(true);
              }}
            >
              {t("Create branch from commit")}
            </GitMenuItem>
            <GitMenuItem
              onClick={() => {
                setPopover(null);
                setHistoryTagDraftName("");
                setHistoryTagDraftMessage("");
                setHistoryTagDialogOpen(true);
              }}
            >
              {t("Create tag from commit")}
            </GitMenuItem>
            <GitMenuItem
              onClick={() => {
                setPopover(null);
                setHistoryResetDialogOpen(true);
              }}
            >
              {t("Reset current branch")}
            </GitMenuItem>
            <GitMenuItem
              disabled={!historyContextIsHead(historyContextCommit) || !historyContextParentHash(historyContextCommit) || busy}
              onClick={() => {
                setPopover(null);
                void runGitAction(() =>
                  cmd.gitResetToCommit(currentRepoPath, historyContextParentHash(historyContextCommit), "soft"),
                );
              }}
            >
              {t("Undo commit")}
            </GitMenuItem>
            <GitMenuItem
              disabled={!historyContextIsHead(historyContextCommit) || busy}
              onClick={() => {
                setPopover(null);
                setHistoryAmendMessage(activeCommitDetail?.message || historyContextCommit.message || "");
                setHistoryEditDialogOpen(true);
              }}
            >
              {t("Edit commit message")}
            </GitMenuItem>
            <GitMenuItem
              disabled={busy}
              onClick={() => {
                setPopover(null);
                setHistoryDropDialogOpen(true);
              }}
            >
              {t("Drop commit")}
            </GitMenuItem>
            <div className="git-popover-divider" />
            <GitMenuItem
              disabled={busy}
              onClick={() => {
                const hash = historyContextCommit?.hash || "";
                setPopover(null);
                if (!hash) return;
                void runGitAction(() => cmd.gitRevertCommit(currentRepoPath, hash));
              }}
            >
              {t("Revert commit")}
            </GitMenuItem>
            <GitMenuItem
              disabled={busy}
              onClick={() => {
                const hash = historyContextCommit?.hash || "";
                setPopover(null);
                if (!hash) return;
                void runGitAction(() => cmd.gitCherryPickCommit(currentRepoPath, hash));
              }}
            >
              {t("Cherry-pick onto current branch")}
            </GitMenuItem>
          </div>
        ) : null}
      </GitPopover>

      <GitPopover kind="stashMenu" onClose={() => setPopover(null)} popover={popover}>
        {popover?.kind === "stashMenu" ? (
          <div className="git-popover-list">
            <GitMenuItem
              onClick={() => {
                const stash = popover.data as GitStashEntry;
                setPopover(null);
                void runGitAction(() => cmd.gitStashApply(currentRepoPath, stash.index), { stash: true });
              }}
            >
              {t("Apply")}
            </GitMenuItem>
            <GitMenuItem
              onClick={() => {
                const stash = popover.data as GitStashEntry;
                setPopover(null);
                void runGitAction(() => cmd.gitStashPop(currentRepoPath, stash.index), { stash: true });
              }}
            >
              {t("Pop")}
            </GitMenuItem>
            <div className="git-popover-divider" />
            <GitMenuItem
              destructive
              onClick={() => {
                const stash = popover.data as GitStashEntry;
                setPopover(null);
                void runGitAction(() => cmd.gitStashDrop(currentRepoPath, stash.index), { stash: true });
              }}
            >
              {t("Drop")}
            </GitMenuItem>
          </div>
        ) : null}
      </GitPopover>
      <GitPopover kind="tagManager" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager">
          <GitSectionHeader
            actions={
              <>
                <GitIconButton active={tagCreateExpanded} aria-label={tagCreateExpanded ? t("Hide composer") : t("New tag")} icon={tagCreateExpanded ? X : Plus} onClick={() => setTagCreateExpanded((value) => !value)} />
                <GitButton compact disabled={!tags.length || busy} onClick={() => void runGitAction(() => cmd.gitPushAllTags(currentRepoPath), { tags: true })}>{t("Push all")}</GitButton>
                <GitIconButton aria-label={t("Reload tags")} icon={RefreshCw} onClick={() => void loadTags()} />
              </>
            }
            subtitle={t("Create, push, and delete release markers")}
            title={t("Tags")}
          />
          {tagCreateExpanded ? (
            <div className="git-card git-card--inset">
              <input className="git-input" onChange={(event) => setTagDraftName(event.currentTarget.value)} placeholder={t("Tag name")} value={tagDraftName} />
              <input className="git-input" onChange={(event) => setTagDraftMessage(event.currentTarget.value)} placeholder={t("Tag message (optional)")} value={tagDraftMessage} />
              <div className="git-inline-form">
                <div className="git-commit-actions__spacer" />
                <GitButton
                  tone="primary"
                  compact
                  disabled={!tagDraftName.trim() || busy}
                  onClick={() =>
                    void runGitAction(() => cmd.gitCreateTag(currentRepoPath, tagDraftName.trim(), tagDraftMessage.trim()), {
                      tags: true,
                    }).then(() => {
                      setTagDraftName("");
                      setTagDraftMessage("");
                    })
                  }
                >
                  {t("Create tag")}
                </GitButton>
              </div>
            </div>
          ) : null}
          <label className="git-search">
            <Search size={12} />
            <input onChange={(event) => setTagSearchText(event.currentTarget.value)} placeholder={t("Filter tags")} value={tagSearchText} />
            {tagSearchText ? <button onClick={() => setTagSearchText("")} type="button"><X size={11} /></button> : null}
          </label>
          <div className="git-manager-list">
            {filteredTagEntries.length ? (
              filteredTagEntries.map((tag) => (
                <div className="git-manager-row" key={tag.name}>
                  <span className="git-manager-row__dot git-manager-row__dot--tag" />
                  <div className="git-manager-row__copy">
                    <div className="git-manager-row__title">{tag.name}</div>
                    {tag.message ? <div className="git-manager-row__subtitle">{tag.message}</div> : null}
                  </div>
                  <span className="git-manager-row__meta">{tag.hash}</span>
                  <div className="git-manager-row__actions">
                    <GitIconButton aria-label={t("Push")} icon={Upload} onClick={() => void runGitAction(() => cmd.gitPushTag(currentRepoPath, tag.name), { tags: true })} title={t("Push tag to remote")} />
                    <GitIconButton aria-label={t("Copy hash")} icon={Copy} onClick={() => void copyText(tag.hash)} title={t("Copy commit hash")} />
                    <GitIconButton aria-label={t("Delete")} className="is-danger" icon={Trash2} onClick={() => void runGitAction(() => cmd.gitDeleteTag(currentRepoPath, tag.name), { tags: true })} title={t("Delete tag")} />
                  </div>
                </div>
              ))
            ) : (
              <GitEmptyState accent="var(--warn)" description={t("Create release or checkpoint tags for this repository.")} icon={Tag} title={t("No tags")} />
            )}
          </div>
        </div>
      </GitPopover>

      <GitPopover kind="remoteManager" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager">
          <GitSectionHeader
            actions={
              <>
                <GitIconButton
                  active={remoteComposerExpanded || !!remoteEditSourceName}
                  aria-label={remoteComposerExpanded || !!remoteEditSourceName ? t("Hide composer") : t("Add remote")}
                  icon={remoteComposerExpanded || !!remoteEditSourceName ? X : Plus}
                  onClick={() => {
                    if (remoteEditSourceName) clearRemoteDraft();
                    else setRemoteComposerExpanded((value) => !value);
                  }}
                />
                <GitIconButton aria-label={t("Reload remotes")} icon={RefreshCw} onClick={() => void loadRemotes()} />
                <GitButton compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitFetchRemote(currentRepoPath, null), { remotes: true })}>
                  {t("Fetch all")}
                </GitButton>
              </>
            }
            subtitle={remoteEditSourceName ? `${t("Update fetch/push URL for")} ${remoteEditSourceName}` : t("Manage upstream repository endpoints")}
            title={t("Remotes")}
          />
          {remoteComposerExpanded || remoteEditSourceName ? (
            <div className="git-card git-card--inset">
              {remoteEditSourceName ? <div className="git-inline-note">{`${t("Editing remote")} ${remoteEditSourceName}.`}</div> : null}
              <input className="git-input" disabled={!!remoteEditSourceName} onChange={(event) => setRemoteDraftName(event.currentTarget.value)} placeholder={t("Remote name")} value={remoteDraftName} />
              <input className="git-input" onChange={(event) => setRemoteDraftUrl(event.currentTarget.value)} placeholder={t("Remote URL")} value={remoteDraftUrl} />
              <div className="git-inline-form">
                {remoteEditSourceName ? <GitButton compact onClick={() => clearRemoteDraft()}>{t("Cancel edit")}</GitButton> : null}
                <div className="git-commit-actions__spacer" />
                <GitButton
                  tone="primary"
                  compact
                  disabled={!remoteDraftName.trim() || !remoteDraftUrl.trim() || busy}
                  onClick={() => {
                    const action = remoteEditSourceName
                      ? cmd.gitSetRemoteUrl(currentRepoPath, remoteEditSourceName, remoteDraftUrl.trim())
                      : cmd.gitAddRemote(currentRepoPath, remoteDraftName.trim(), remoteDraftUrl.trim());
                    void runGitAction(() => action, { remotes: true }).then(() => clearRemoteDraft());
                  }}
                >
                  {remoteEditSourceName ? t("Update remote") : t("Add remote")}
                </GitButton>
              </div>
            </div>
          ) : null}
          <label className="git-search">
            <Search size={12} />
            <input onChange={(event) => setRemoteSearchText(event.currentTarget.value)} placeholder={t("Filter remotes")} value={remoteSearchText} />
            {remoteSearchText ? <button onClick={() => setRemoteSearchText("")} type="button"><X size={11} /></button> : null}
          </label>
          <div className="git-manager-list">
            {filteredRemoteEntries.length ? (
              filteredRemoteEntries.map((remote) => (
                <div className="git-manager-row" key={remote.name}>
                  <span className="git-manager-row__dot git-manager-row__dot--accent" />
                  <div className="git-manager-row__copy">
                    <div className="git-manager-row__title">{remote.name}</div>
                    <div className="git-manager-row__subtitle">{remote.fetchUrl || remote.pushUrl}</div>
                  </div>
                  <div className="git-manager-row__actions">
                    <GitIconButton aria-label={t("Fetch")} icon={Download} onClick={() => void runGitAction(() => cmd.gitFetchRemote(currentRepoPath, remote.name), { remotes: true })} title={t("Fetch from this remote")} />
                    <GitIconButton aria-label={t("Edit")} icon={Pencil} onClick={() => beginRemoteEdit(remote)} title={t("Edit remote URL")} />
                    <GitIconButton aria-label={t("Remove")} className="is-danger" icon={Trash2} onClick={() => void runGitAction(() => cmd.gitRemoveRemote(currentRepoPath, remote.name), { remotes: true })} title={t("Remove remote")} />
                  </div>
                </div>
              ))
            ) : (
              <GitEmptyState accent="var(--accent)" description={t("Add an origin or upstream remote to enable pull and push.")} icon={Network} title={t("No remotes")} />
            )}
          </div>
        </div>
      </GitPopover>

      <GitPopover kind="configManager" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager git-popover-form">
          <GitSectionHeader
            actions={
              <>
                <GitIconButton
                  active={configComposerExpanded}
                  aria-label={configComposerExpanded ? t("Hide composer") : t("Add setting")}
                  icon={configComposerExpanded ? X : Plus}
                  onClick={() => {
                    setConfigComposerExpanded((value) => !value);
                    setConfigDraftKey("");
                    setConfigDraftValue("");
                  }}
                  title={configComposerExpanded ? t("Hide composer") : t("Add new config setting")}
                />
                <GitIconButton aria-label={t("Reload config")} icon={RefreshCw} onClick={() => void loadConfigEntries()} title={t("Reload config")} />
              </>
            }
            subtitle={t("View and edit local or global Git configuration")}
            title={t("Config")}
          />
          {configComposerExpanded ? (
            <div className="git-card git-card--inset">
              <div className="git-segmented">
                <button className={!configDraftGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigDraftGlobal(false)} type="button">{t("Local")}</button>
                <button className={configDraftGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigDraftGlobal(true)} type="button">{t("Global")}</button>
              </div>
              <input className="git-input" onChange={(event) => setConfigDraftKey(event.currentTarget.value)} placeholder={t("Config key (e.g. user.email)")} value={configDraftKey} />
              <input className="git-input" onChange={(event) => setConfigDraftValue(event.currentTarget.value)} placeholder={t("Value")} value={configDraftValue} />
              <div className="git-inline-form">
                <div className="git-commit-actions__spacer" />
                <GitButton
                  tone="primary"
                  compact
                  disabled={!configDraftKey.trim() || busy}
                  onClick={() =>
                    void runGitAction(
                      () => cmd.gitSetConfigValue(currentRepoPath, configDraftKey.trim(), configDraftValue, configDraftGlobal),
                      { config: true },
                    ).then(() => {
                      setConfigDraftKey("");
                      setConfigDraftValue("");
                      setConfigComposerExpanded(false);
                    })
                  }
                >
                  {t("Add")}
                </GitButton>
              </div>
            </div>
          ) : null}
          <label className="git-search">
            <Search size={12} />
            <input onChange={(event) => setConfigSearchText(event.currentTarget.value)} placeholder={t("Filter key or value")} value={configSearchText} />
            {configSearchText ? <button onClick={() => setConfigSearchText("")} type="button"><X size={11} /></button> : null}
          </label>
          <div className="git-segmented">
            <button className={!configSelectedGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigSelectedGlobal(false)} type="button">{t("Local")}</button>
            <button className={configSelectedGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigSelectedGlobal(true)} type="button">{t("Global")}</button>
          </div>
          <div className="git-popover-form__body">
            {(() => {
              const filtered = configEntries
                .filter((entry) => entry.scope === (configSelectedGlobal ? "global" : "local"))
                .filter((entry) => {
                  const needle = configSearchText.trim().toLowerCase();
                  if (!needle) return true;
                  return entry.key.toLowerCase().includes(needle) || entry.value.toLowerCase().includes(needle);
                });
              if (!filtered.length) {
                return (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={
                      configSelectedGlobal
                        ? t("Set global Git configuration values that apply across repositories.")
                        : t("Set repository-specific Git configuration values for this project.")
                    }
                    icon={Settings2}
                    title={t("No config entries")}
                  />
                );
              }
              return (
                <div className="git-manager-list">
                  {filtered.map((entry) => {
                    const editingId = `${entry.scope}:${entry.key}`;
                    const editing = configEditingKey === editingId;
                    return (
                      <div className={"git-manager-row" + (editing ? " is-editing" : "")} key={editingId}>
                        <div className="git-manager-row__copy">
                          <div className="git-manager-row__title mono">{entry.key}</div>
                          {editing ? (
                            <input
                              autoFocus
                              className="git-input git-manager-row__edit"
                              onBlur={() => commitConfigEdit(entry)}
                              onChange={(event) => setConfigEditingDraft(event.currentTarget.value)}
                              onKeyDown={(event) => {
                                if (event.key === "Enter") {
                                  event.preventDefault();
                                  commitConfigEdit(entry);
                                } else if (event.key === "Escape") {
                                  event.preventDefault();
                                  setConfigEditingKey(null);
                                }
                              }}
                              value={configEditingDraft}
                            />
                          ) : (
                            <div className="git-manager-row__subtitle" onDoubleClick={() => beginConfigEdit(entry)} title={t("Double-click to edit")}>
                              {entry.value || <span className="text-dim">{t("(empty)")}</span>}
                            </div>
                          )}
                        </div>
                        <div className="git-manager-row__actions">
                          <GitIconButton aria-label={t("Edit")} icon={Pencil} onClick={() => beginConfigEdit(entry)} title={t("Edit value")} />
                          <GitIconButton aria-label={t("Copy")} icon={Copy} onClick={() => void copyText(entry.value)} title={t("Copy value")} />
                          <GitIconButton aria-label={t("Unset")} className="is-danger" icon={Trash2} onClick={() => void runGitAction(() => cmd.gitUnsetConfigValue(currentRepoPath, entry.key, configSelectedGlobal), { config: true })} title={t("Unset config entry")} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              );
            })()}
          </div>
        </div>
      </GitPopover>

      <GitPopover kind="rebaseManager" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager">
          <GitSectionHeader
            actions={<GitIconButton aria-label={t("Reload rebase plan")} icon={RefreshCw} onClick={() => void loadRebase()} />}
            subtitle={rebasePlan.inProgress ? t("Continue or abort the active rebase session") : t("Reorder, squash, or drop recent commits")}
            title={t("Interactive rebase")}
          />
          {rebasePlan.inProgress ? (
            <>
              <div className="git-banner git-banner--warning">
                <div className="git-banner__dot" />
                <div className="git-banner__message">{t("Git reports that an interactive rebase is already in progress.")}</div>
              </div>
              <div className="git-inline-form">
                <GitButton compact onClick={() => void runGitAction(() => cmd.gitAbortRebase(currentRepoPath), { rebase: true })}>{t("Abort")}</GitButton>
                <GitButton tone="primary" compact onClick={() => void runGitAction(() => cmd.gitContinueRebase(currentRepoPath), { rebase: true })}>{t("Continue")}</GitButton>
              </div>
            </>
          ) : (
            <div className="git-card git-card--inset">
              <div className="git-inline-form">
                <select className="git-select git-select--narrow" onChange={(event) => setRebaseCommitCount(Number(event.currentTarget.value))} value={rebaseCommitCount}>
                  <option value={10}>10</option>
                  <option value={20}>20</option>
                  <option value={50}>50</option>
                </select>
                <span className="git-inline-note">{t("Recent commits")}</span>
                <div className="git-commit-actions__spacer" />
                <GitButton
                  tone="primary"
                  compact
                  disabled={!rebaseDraftItems.length || busy}
                  onClick={() =>
                    void runGitAction(
                      () => cmd.gitExecuteRebase(currentRepoPath, rebaseDraftItems, rebaseDraftItems.length ? `${rebaseDraftItems[rebaseDraftItems.length - 1].hash}~1` : null),
                      { rebase: true, refresh: true },
                    )
                  }
                >
                  {t("Execute")}
                </GitButton>
              </div>
              <div className="git-manager-list">
                {rebaseDraftItems.length ? (
                  rebaseDraftItems.map((item, index) => (
                    <div className="git-manager-row" key={`${item.hash}-${index}`}>
                      <select
                        className="git-select git-select--action"
                        onChange={(event) =>
                          setRebaseDraftItems((current) => {
                            const next = [...current];
                            next[index] = { ...next[index], action: event.currentTarget.value };
                            return next;
                          })
                        }
                        value={item.action}
                      >
                        <option value="pick">{t("Pick")}</option>
                        <option value="reword">{t("Reword")}</option>
                        <option value="edit">{t("Edit")}</option>
                        <option value="squash">{t("Squash")}</option>
                        <option value="fixup">{t("Fixup")}</option>
                        <option value="drop">{t("Drop")}</option>
                      </select>
                      <span className="git-manager-row__meta git-manager-row__meta--accent">{item.shortHash}</span>
                      <div className="git-manager-row__copy">
                        <div className="git-manager-row__title">{item.message}</div>
                      </div>
                      <div className="git-manager-row__actions">
                        <GitButton
                          compact
                          disabled={index === 0}
                          onClick={() =>
                            setRebaseDraftItems((current) => {
                              const next = [...current];
                              [next[index - 1], next[index]] = [next[index], next[index - 1]];
                              return next;
                            })
                          }
                        >
                          ↑
                        </GitButton>
                        <GitButton
                          compact
                          disabled={index === rebaseDraftItems.length - 1}
                          onClick={() =>
                            setRebaseDraftItems((current) => {
                              const next = [...current];
                              [next[index], next[index + 1]] = [next[index + 1], next[index]];
                              return next;
                            })
                          }
                        >
                          ↓
                        </GitButton>
                      </div>
                    </div>
                  ))
                ) : (
                  <GitEmptyState accent="var(--accent)" description={t("Load recent commits to start an interactive rebase.")} icon={GitMerge} title={t("No rebase plan")} />
                )}
              </div>
            </div>
          )}
        </div>
      </GitPopover>

      <GitPopover kind="submoduleManager" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager">
          <GitSectionHeader
            actions={<GitIconButton aria-label={t("Reload submodules")} icon={RefreshCw} onClick={() => void loadSubmodules()} />}
            subtitle={t("Inspect and update nested repositories")}
            title={t("Submodules")}
          />
          <div className="git-inline-form">
            <GitButton compact onClick={() => void runGitAction(() => cmd.gitInitSubmodules(currentRepoPath), { submodules: true })}>{t("Init")}</GitButton>
            <GitButton compact onClick={() => void runGitAction(() => cmd.gitUpdateSubmodules(currentRepoPath, true), { submodules: true })}>{t("Update")}</GitButton>
            <GitButton compact onClick={() => void runGitAction(() => cmd.gitSyncSubmodules(currentRepoPath), { submodules: true })}>{t("Sync")}</GitButton>
          </div>
          <label className="git-search">
            <Search size={12} />
            <input onChange={(event) => setSubmoduleSearchText(event.currentTarget.value)} placeholder={t("Filter submodules")} value={submoduleSearchText} />
            {submoduleSearchText ? <button onClick={() => setSubmoduleSearchText("")} type="button"><X size={11} /></button> : null}
          </label>
          <div className="git-manager-list">
            {filteredSubmodules.length ? (
              filteredSubmodules.map((submodule) => (
                <div className="git-manager-row" key={submodule.path}>
                  <span className={`git-manager-row__dot git-manager-row__dot--${submodule.status}`} />
                  <div className="git-manager-row__copy">
                    <div className="git-manager-row__title">{submodule.path}</div>
                    {submodule.url ? <div className="git-manager-row__subtitle">{submodule.url}</div> : null}
                  </div>
                  <span className="git-manager-row__meta">{submodule.shortHash}</span>
                  <div className="git-manager-row__actions">
                    <GitIconButton aria-label={t("Copy path")} icon={Folder} onClick={() => void copyText(submodule.path)} title={t("Copy submodule path")} />
                    {submodule.url ? <GitIconButton aria-label={t("Copy URL")} icon={Copy} onClick={() => void copyText(submodule.url)} title={t("Copy submodule URL")} /> : null}
                  </div>
                </div>
              ))
            ) : (
              <GitEmptyState accent="var(--accent)" description={t("Nested repositories will appear here after you add or initialize them.")} icon={Layers} title={t("No submodules")} />
            )}
          </div>
        </div>
      </GitPopover>

      <GitPopover kind="unpushedCommits" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager git-popover-form">
          <GitSectionHeader
            actions={
              <GitIconButton
                aria-label={t("Reload")}
                icon={RefreshCw}
                onClick={() => void loadUnpushedCommits()}
                title={t("Reload unpushed commits")}
              />
            }
            subtitle={
              unpushedCommits.length
                ? `${unpushedCommits.length} ${t("commits ahead of upstream")}`
                : t("Nothing waiting to be pushed")
            }
            title={t("Unpushed commits")}
          />
          <div className="git-popover-form__body">
            {unpushedCommits.length ? (
              <div className="git-manager-list">
                {unpushedCommits.map((commit) => {
                  const editing = unpushedEditingHash === commit.hash;
                  const beginEdit = () => {
                    setUnpushedEditingHash(commit.hash);
                    setUnpushedEditingDraft(commit.message);
                  };
                  const commitEdit = () => {
                    // Guard: keydown (Cmd+Enter) and the resulting
                    // blur on the unmounted textarea would otherwise
                    // both dispatch reword, racing two parallel git
                    // operations against the same commit.
                    if (unpushedSavingRef.current === commit.hash) return;
                    const next = unpushedEditingDraft.trim();
                    setUnpushedEditingHash(null);
                    if (!next || next === commit.message.trim()) return;
                    unpushedSavingRef.current = commit.hash;
                    // git_reword_unpushed_commit handles HEAD via amend
                    // and older commits via interactive rebase. We
                    // reload the list in finally — even when the git
                    // operation errors out (dirty tree, hook reject)
                    // the popover should reflect the actual repo state
                    // rather than keep showing stale optimistic data.
                    void runGitAction(
                      () => cmd.gitRewordUnpushedCommit(currentRepoPath, commit.hash, next),
                    )
                      .catch(() => {})
                      .finally(() => {
                        unpushedSavingRef.current = null;
                        void loadUnpushedCommits();
                      });
                  };
                  const subject = (commit.message.split("\n", 1)[0] || "").trim();
                  const body = commit.message.slice(subject.length).replace(/^\n+/, "");
                  return (
                    <div className={"git-manager-row git-unpushed-row" + (editing ? " is-editing" : "")} key={commit.hash}>
                      <span className={`git-manager-row__dot ${commit.isHead ? "git-manager-row__dot--success" : "git-manager-row__dot--accent"}`} />
                      <span className="git-manager-row__meta git-manager-row__meta--accent">{commit.shortHash}</span>
                      <div className="git-manager-row__copy">
                        {editing ? (
                          <>
                            <textarea
                              autoFocus
                              className="git-textarea git-textarea--mono git-unpushed-row__edit"
                              onBlur={commitEdit}
                              onChange={(event) => setUnpushedEditingDraft(event.currentTarget.value)}
                              onKeyDown={(event) => {
                                if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
                                  event.preventDefault();
                                  commitEdit();
                                } else if (event.key === "Escape") {
                                  event.preventDefault();
                                  setUnpushedEditingHash(null);
                                }
                              }}
                              rows={Math.min(8, Math.max(3, unpushedEditingDraft.split("\n").length))}
                              value={unpushedEditingDraft}
                            />
                            <div className="git-unpushed-row__hint mono">
                              {t("Cmd/Ctrl+Enter to save · Esc to cancel")}
                            </div>
                          </>
                        ) : (
                          <>
                            <div className="git-manager-row__title">{subject || "(no message)"}</div>
                            {body ? <div className="git-unpushed-row__body mono">{body}</div> : null}
                            <div className="git-manager-row__subtitle">{commit.author} · {commit.relativeDate}</div>
                          </>
                        )}
                      </div>
                      <div className="git-manager-row__actions">
                        <GitIconButton
                          aria-label={t("Copy hash")}
                          icon={Copy}
                          onClick={() => void copyText(commit.hash)}
                          title={t("Copy commit hash")}
                        />
                        <GitIconButton
                          aria-label={t("Edit message")}
                          icon={Pencil}
                          onClick={beginEdit}
                          title={commit.isHead ? t("Reword HEAD commit") : t("Reword commit (interactive rebase)")}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : (
              <GitEmptyState
                accent="var(--accent)"
                description={t("Local commits will appear here once HEAD moves ahead of the upstream tracking branch.")}
                icon={Upload}
                title={t("Up to date")}
              />
            )}
          </div>
          <div className="git-popover-form__foot">
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="primary"
              compact
              disabled={!panelState?.aheadCount || busy}
              onClick={() => {
                setPopover(null);
                void runGitAction(() => cmd.gitPush(currentRepoPath));
              }}
            >
              {t("Push")}
            </GitButton>
          </div>
        </div>
      </GitPopover>

      <GitPopover kind="historyPathFilter" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-manager git-popover-form">
          <GitSectionHeader
            subtitle={t("Filter commit graph to specific repository paths")}
            title={t("Tracked files")}
          />
          <label className="git-search">
            <Search size={12} />
            <input
              onChange={(event) => setHistoryPathSearchText(event.currentTarget.value)}
              placeholder={t("Search tracked files")}
              value={historyPathSearchText}
            />
            {historyPathSearchText ? (
              <button onClick={() => setHistoryPathSearchText("")} type="button"><X size={11} /></button>
            ) : null}
          </label>
          <div className="git-popover-form__body">
            {filteredHistoryPathTree.length ? (
              <div className="git-path-list git-path-tree">
                {renderHistoryPathTree(filteredHistoryPathTree)}
              </div>
            ) : (
              <GitEmptyState
                accent="var(--accent)"
                description={t("Try a different search or refresh repository metadata.")}
                icon={Folder}
                title={t("No tracked files")}
              />
            )}
          </div>
          <div className="git-popover-form__foot">
            <GitButton compact onClick={() => setHistoryPathSelection([])}>{t("Clear")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton compact onClick={() => setPopover(null)}>{t("Cancel")}</GitButton>
            <GitButton
              tone="primary"
              compact
              onClick={() => {
                setHistoryPaths(historyPathSelection);
                setPopover(null);
              }}
            >
              {t("Apply")}
            </GitButton>
          </div>
        </div>
      </GitPopover>

      <GitDialog
        footer={
          <GitButton compact onClick={() => {
            setHistoryCompareDialogOpen(false);
            setComparisonFiles([]);
            setComparisonDiff("");
            setComparisonSelectedPath("");
            setComparisonExpandedPaths([]);
          }}>{t("Close")}</GitButton>
        }
        onClose={() => {
          setHistoryCompareDialogOpen(false);
          setComparisonFiles([]);
          setComparisonDiff("");
          setComparisonSelectedPath("");
          setComparisonExpandedPaths([]);
        }}
        open={historyCompareDialogOpen}
        subtitle={comparisonBaseHash || t("Commit comparison")}
        title={t("Compare with local")}
        wide
        tall
      >
        <PanelGroup className="git-panel-group" orientation="horizontal">
          <Panel defaultSize={32} minSize={22}>
            <div className="git-card git-card--inset git-card--fill git-compare-pane">
              <div className="git-diff__header git-compare-pane__header">
                <div className="git-compare-pane__title-wrap">
                  <div className="git-diff__title">{t("Changed files")}</div>
                  <span className="git-file-section__count">{comparisonFiles.length}</span>
                </div>
              </div>
              {comparisonFiles.length ? (
                <div className="git-compare-file-list git-compare-file-list--tree">
                  {renderComparisonTree(comparisonPathTree)}
                </div>
              ) : (
                <GitEmptyState accent="var(--accent)" description={t("This commit matches local HEAD, or there are no comparable files.")} icon={GitBranch} title={t("No local diff")} />
              )}
            </div>
          </Panel>
          <PanelResizeHandle className="git-split-handle git-split-handle--horizontal" />
          <Panel defaultSize={68} minSize={40}>
            <div className="git-card git-card--inset git-card--fill git-compare-pane">
              <div className="git-diff__header git-compare-pane__header">
                <div className="git-compare-pane__title-wrap git-compare-pane__title-wrap--diff">
                  <div className="git-diff__title">{`${comparisonBaseHash.slice(0, 8)} ↔ ${t("Working tree")}`}</div>
                  {comparisonSelectedPath ? (
                    <div className="git-compare-pane__path" title={comparisonSelectedPath}>{comparisonSelectedPath}</div>
                  ) : null}
                </div>
              </div>
              {comparisonDiff ? (
                <GitDiffCode text={comparisonDiff} />
              ) : (
                <GitEmptyState accent="var(--accent)" description={t("Select a changed file to inspect the diff against local HEAD.")} icon={FileText} title={t("Select a changed file")} />
              )}
            </div>
          </Panel>
        </PanelGroup>
      </GitDialog>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryBranchDialogOpen(false)}>{t("Cancel")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="primary"
              compact
              disabled={!historyBranchDraftName.trim() || !historyContextCommit?.hash || busy}
              onClick={() =>
                void runGitAction(() => cmd.gitCreateBranchAt(currentRepoPath, historyBranchDraftName.trim(), historyContextCommit?.hash || null)).then(() => {
                  setHistoryBranchDraftName("");
                  setHistoryBranchDialogOpen(false);
                })
              }
            >
              {t("Create branch")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryBranchDialogOpen(false)}
        open={historyBranchDialogOpen}
        subtitle={t("Create a branch that starts at this commit")}
        title={t("Create branch from commit")}
      >
        <div className="git-card git-card--inset">
          <GitSectionHeader subtitle={historyContextCommit?.message || ""} title={historyContextCommit?.shortHash || t("Commit")} />
          <input className="git-input" onChange={(event) => setHistoryBranchDraftName(event.currentTarget.value)} placeholder={t("Branch name")} value={historyBranchDraftName} />
        </div>
      </GitDialog>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryTagDialogOpen(false)}>{t("Cancel")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="primary"
              compact
              disabled={!historyTagDraftName.trim() || !historyContextCommit?.hash || busy}
              onClick={() =>
                void runGitAction(
                  () =>
                    cmd.gitCreateTagAt(
                      currentRepoPath,
                      historyTagDraftName.trim(),
                      historyContextCommit?.hash || null,
                      historyTagDraftMessage.trim(),
                    ),
                  { tags: true },
                ).then(() => {
                  setHistoryTagDraftName("");
                  setHistoryTagDraftMessage("");
                  setHistoryTagDialogOpen(false);
                })
              }
            >
              {t("Create tag")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryTagDialogOpen(false)}
        open={historyTagDialogOpen}
        subtitle={t("Create a lightweight or annotated tag at this commit")}
        title={t("Create tag from commit")}
      >
        <div className="git-card git-card--inset">
          <GitSectionHeader subtitle={historyContextCommit?.message || ""} title={historyContextCommit?.shortHash || t("Commit")} />
          <input className="git-input" onChange={(event) => setHistoryTagDraftName(event.currentTarget.value)} placeholder={t("Tag name")} value={historyTagDraftName} />
          <textarea className="git-textarea" onChange={(event) => setHistoryTagDraftMessage(event.currentTarget.value)} placeholder={t("Annotated tag message (optional)")} rows={5} value={historyTagDraftMessage} />
        </div>
      </GitDialog>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryResetDialogOpen(false)}>{t("Cancel")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="primary"
              compact
              disabled={!historyContextCommit?.hash || busy}
              onClick={() =>
                void runGitAction(() => cmd.gitResetToCommit(currentRepoPath, historyContextCommit?.hash || "", historyResetMode)).then(() => {
                  setHistoryResetDialogOpen(false);
                })
              }
            >
              {t("Apply reset")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryResetDialogOpen(false)}
        open={historyResetDialogOpen}
        title={t("Reset current branch")}
        subtitle={t("Move the current branch pointer to this commit")}
      >
        <div className="git-card git-card--inset">
          <GitSectionHeader subtitle={t("Soft keeps changes staged, mixed keeps changes unstaged, hard discards working tree changes.")} title={t("Reset mode")} />
          <div className="git-segmented">
            <button className={historyResetMode === "soft" ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setHistoryResetMode("soft")} type="button">{t("Soft")}</button>
            <button className={historyResetMode === "mixed" ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setHistoryResetMode("mixed")} type="button">{t("Mixed")}</button>
            <button className={historyResetMode === "hard" ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setHistoryResetMode("hard")} type="button">{t("Hard")}</button>
          </div>
          <div className={`git-banner git-banner--${historyResetMode === "hard" ? "warning" : "info"}`}>
            <div className="git-banner__dot" />
            <div className="git-banner__message">
              {historyResetMode === "hard"
                ? t("Hard reset will discard working tree changes.")
                : historyResetMode === "soft"
                  ? t("Soft reset keeps all changes staged for recommit.")
                  : t("Mixed reset keeps changes in the working tree but unstaged.")}
            </div>
          </div>
        </div>
      </GitDialog>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryEditDialogOpen(false)}>{t("Cancel")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="primary"
              compact
              disabled={!historyContextCommit?.hash || !historyAmendMessage.trim() || busy}
              onClick={() =>
                void runGitAction(
                  () => cmd.gitAmendHeadCommitMessage(currentRepoPath, historyContextCommit?.hash || "", historyAmendMessage.trim()),
                ).then(() => setHistoryEditDialogOpen(false))
              }
            >
              {t("Edit message")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryEditDialogOpen(false)}
        open={historyEditDialogOpen}
        subtitle={t("Amend the HEAD commit message")}
        title={t("Edit commit message")}
      >
        <div className="git-card git-card--inset">
          <div className="git-banner git-banner--info">
            <div className="git-banner__dot" />
            <div className="git-banner__message">{t("The HEAD commit will be amended with the message below.")}</div>
          </div>
          <textarea className="git-textarea" onChange={(event) => setHistoryAmendMessage(event.currentTarget.value)} placeholder={t("Update commit message")} rows={8} value={historyAmendMessage} />
        </div>
      </GitDialog>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryDropDialogOpen(false)}>{t("Cancel")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton
              tone="destructive"
              compact
              disabled={!historyContextCommit?.hash || busy}
              onClick={() =>
                void runGitAction(
                  () => cmd.gitDropCommit(currentRepoPath, historyContextCommit?.hash || "", historyContextParentHash(historyContextCommit) || null),
                ).then(() => setHistoryDropDialogOpen(false))
              }
            >
              {t("Drop")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryDropDialogOpen(false)}
        open={historyDropDialogOpen}
        subtitle={t("Remove this commit from history")}
        title={t("Drop commit")}
      >
        <div className="git-card git-card--inset">
          <div className="git-banner git-banner--warning">
            <div className="git-banner__dot" />
            <div className="git-banner__message">{t("This will permanently rewrite Git history for the current branch.")}</div>
          </div>
          <div className="git-inline-note">
            {historyContextIsHead(historyContextCommit)
              ? t("The current HEAD commit will be removed by resetting to its parent.")
              : t("This non-HEAD commit will be removed using rebase --onto.")}
          </div>
        </div>
      </GitDialog>

      <GitDialog
        footer={<GitButton compact onClick={() => setReflogDialogOpen(false)}>{t("Close")}</GitButton>}
        onClose={() => setReflogDialogOpen(false)}
        open={reflogDialogOpen}
        subtitle={t("Local history of HEAD movements. Use to recover dropped commits.")}
        title={t("Reflog")}
      >
        <div className="git-card git-card--inset">
          {reflogLoading ? (
            <GitEmptyState icon={History} title={t("Loading")} description={t("Loading reflog…")} />
          ) : reflogError ? (
            <div className="git-banner git-banner--error">
              <div className="git-banner__dot" />
              <div className="git-banner__message">{reflogError}</div>
            </div>
          ) : reflogEntries.length === 0 ? (
            <GitEmptyState icon={History} title={t("No reflog entries")} description={t("This repository has no reflog yet.")} />
          ) : (
            <div className="git-conn-list" style={{ maxHeight: 420, overflowY: "auto" }}>
              {reflogEntries.map((entry, idx) => (
                <div key={`${entry.hash}-${idx}`} className="git-conn-row">
                  <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
                    <div style={{ fontFamily: "var(--mono)", fontSize: "var(--ui-fs-sm)", color: "var(--ink-2)" }}>
                      {entry.shortHash} · {entry.refName}
                    </div>
                    <div style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {entry.subject}
                    </div>
                    <div style={{ fontSize: "var(--ui-fs-sm)", color: "var(--muted)" }}>{entry.relativeDate}</div>
                  </div>
                  <button
                    className="mini-button"
                    type="button"
                    onClick={() => {
                      void writeClipboardText(entry.hash);
                    }}
                  >
                    {t("Copy hash")}
                  </button>
                  <button
                    className="mini-button mini-button--destructive"
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      setReflogDialogOpen(false);
                      void runGitAction(() => cmd.gitResetToCommit(currentRepoPath, entry.hash, "hard"));
                    }}
                  >
                    {t("Reset hard here")}
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </GitDialog>

      <GitDialog
        footer={<GitButton compact onClick={() => setBlameDialogOpen(false)}>{t("Close")}</GitButton>}
        onClose={() => setBlameDialogOpen(false)}
        open={blameDialogOpen}
        subtitle={blameFilePath || t("Line ownership")}
        title={t("Blame")}
        wide
        tall
      >
        <div className="git-card git-card--inset git-card--fill">
          {blameLines.length ? (
            <div className="git-blame-list ux-selectable">
              {blameLines.map((line) => (
                <div className="git-blame-row" key={`${line.lineNumber}-${line.hash}-${line.content}`}>
                  <span className="git-blame-row__line">{line.lineNumber}</span>
                  <span className="git-blame-row__hash">{line.shortHash}</span>
                  <span className="git-blame-row__author">{line.author}</span>
                  <span className="git-blame-row__date">{line.date}</span>
                  <span className="git-blame-row__content">{line.content}</span>
                </div>
              ))}
            </div>
          ) : (
            <GitEmptyState accent="var(--accent)" description={t("Select a file diff and run blame to inspect line ownership.")} icon={FileText} title={t("No blame data")} />
          )}
        </div>
      </GitDialog>

      <DiffDialog
        open={workingDiffOpen}
        onClose={() => setWorkingDiffOpen(false)}
        files={workingDiffFiles}
        activeId={workingDiffActiveId}
        onSelectFile={(id) => openWorkingDiffById(id)}
        actions={
          diffTarget?.kind === "working" && diffTarget.path ? (
            <GitButton
              compact
              disabled={busy}
              onClick={() => {
                setBlameDialogOpen(true);
                setBlameFilePath(diffTarget.path);
                void cmd
                  .gitBlameFile(currentRepoPath, diffTarget.path)
                  .then((next) => setBlameLines(next))
                  .catch(() => setBlameLines([]));
              }}
            >
              {t("Blame")}
            </GitButton>
          ) : null
        }
      />

      <DiffDialog
        open={commitDiffOpen}
        onClose={() => setCommitDiffOpen(false)}
        files={
          activeCommitDetail && activeCommitDetail.hash === commitDiffHash
            ? activeCommitDetail.changedFiles.map((file) => ({
                id: file.path,
                path: file.path,
                status: commitDiffStatus(file),
                diffText: commitDiffCache[file.path] ?? null,
                additions: file.additions,
                deletions: file.deletions,
              }))
            : []
        }
        activeId={commitDiffActivePath}
        onSelectFile={(id) => {
          setCommitDiffActivePath(id);
          if (commitDiffCache[id] == null) void ensureCommitDiff(commitDiffHash, id);
        }}
      />
    </div>
  );
}
