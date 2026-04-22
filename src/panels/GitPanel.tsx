import {
  ArrowDown,
  ArrowDownCircle,
  ArrowRight,
  ArrowUp,
  ArrowUpCircle,
  Check,
  ChevronDown,
  FileText,
  Folder,
  GitBranch,
  GitMerge,
  HardDrive,
  History,
  Layers,
  Minus,
  Network,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Tag,
  X,
} from "lucide-react";
import type { ComponentType, CSSProperties, MouseEvent as ReactMouseEvent, ReactNode } from "react";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { startTransition, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as cmd from "../lib/commands";
import { writeClipboardText } from "../lib/clipboard";
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
} from "../lib/types";
import { localizeError } from "../i18n/localizeMessage";
import { translate, useI18n, type I18nValue } from "../i18n/useI18n";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useStatusStore } from "../stores/useStatusStore";

type Props = {
  browserPath: string;
  /** True when this panel is the currently-selected right-side tool AND the
   *  right column is expanded. Drives background polling so hidden panels
   *  don't burn IPC on `git_panel_state` every 3s. */
  isActive?: boolean;
};

type PanelTab = "changes" | "history" | "branches" | "stash" | "conflicts";
type PopoverKind =
  | "branchMenu"
  | "historyOptions"
  | "changeFileMenu"
  | "historyCommit"
  | "tagManager"
  | "remoteManager"
  | "configManager"
  | "rebaseManager"
  | "submoduleManager"
  | "stashMenu";

type DiffTarget =
  | { kind: "working"; path: string; staged: boolean; untracked: boolean }
  | null;

type PopoverState = {
  kind: PopoverKind;
  left: number;
  top: number;
  width: number;
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
  const classes = [
    "git-button",
    `git-button--${tone}`,
    compact ? "git-button--compact" : "",
    className,
  ]
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
      className={["git-icon-button", active ? "git-icon-button--active" : "", className].filter(Boolean).join(" ")}
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
    <div className="git-popover-layer" onMouseDown={onClose}>
      <div
        className="git-popover"
        onMouseDown={(event) => event.stopPropagation()}
        style={{ left: popover.left, top: popover.top, width: popover.width }}
      >
        {children}
      </div>
    </div>
  );
}

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
  if (!open) return null;
  const closeLabel = translate(useSettingsStore.getState().locale, "Close");
  return (
    <div className="git-dialog-layer" onMouseDown={onClose}>
      <div
        className={[
          "git-dialog",
          wide ? "git-dialog--wide" : "",
          tall ? "git-dialog--tall" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="git-dialog__header">
          <div>
            <div className="git-dialog__title">{title}</div>
            {subtitle ? <div className="git-dialog__subtitle">{subtitle}</div> : null}
          </div>
          <GitIconButton aria-label={closeLabel} icon={X} onClick={onClose} />
        </div>
        <div className="git-dialog__body">{children}</div>
        {footer ? <div className="git-dialog__footer">{footer}</div> : null}
      </div>
    </div>
  );
}

function GitGraphLane({ row }: { row: GitGraphRowView }) {
  return (
    <svg className="git-graph-lane" viewBox="0 0 74 24" preserveAspectRatio="none" aria-hidden="true">
      {row.segments.map((segment, index) => (
        <line
          key={`${row.hash}-segment-${index}`}
          x1={segment.xTop}
          y1={segment.yTop}
          x2={segment.xBottom}
          y2={segment.yBottom}
          stroke={graphColor(segment.colorIndex)}
          strokeWidth="1.6"
          strokeLinecap="butt"
        />
      ))}
      {row.arrows.map((arrow, index) => {
        const x = arrow.x;
        const y = arrow.y;
        const points = arrow.isDown
          ? `${x - 3},${y - 2} ${x + 3},${y - 2} ${x},${y + 3}`
          : `${x - 3},${y + 2} ${x + 3},${y + 2} ${x},${y - 3}`;
        return <polygon key={`${row.hash}-arrow-${index}`} points={points} fill={graphColor(arrow.colorIndex)} />;
      })}
      <circle
        cx={row.nodeColumn * 12 + 10}
        cy={12}
        r="4.2"
        fill={graphColor(row.colorIndex)}
        stroke="var(--bg-panel)"
        strokeWidth="1"
      />
    </svg>
  );
}

function GitDiffCode({ text }: { text: string }) {
  const lines = text.split("\n");
  return (
    <pre className="git-diff-code">
      {lines.map((line, index) => (
        <div key={`${index}-${line}`} className={`git-diff-code__line git-diff-code__line--${diffLineTone(line)}`}>
          {line || " "}
        </div>
      ))}
    </pre>
  );
}

function GitMenuItem({
  active = false,
  destructive = false,
  children,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  active?: boolean;
  destructive?: boolean;
}) {
  return (
    <button
      {...props}
      className={[
        "git-menu-item",
        active ? "git-menu-item--active" : "",
        destructive ? "git-menu-item--destructive" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      type={props.type ?? "button"}
    >
      {children}
    </button>
  );
}

export default function GitPanel({ browserPath, isActive = true }: Props) {
  const { t } = useI18n();
  const panelRef = useRef<HTMLDivElement>(null);

  const [panelState, setPanelState] = useState<GitPanelState | null>(null);
  const setGitStatus = useStatusStore((s) => s.setGitStatus);
  const clearGitStatus = useStatusStore((s) => s.clearGitStatus);
  const [gitReady, setGitReady] = useState(false);
  const [gitError, setGitError] = useState("");
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<BannerState>(null);
  const [selectedTab, setSelectedTab] = useState<PanelTab>("changes");
  const [branchMenuBranches, setBranchMenuBranches] = useState<string[]>([]);
  const [graphMetadata, setGraphMetadata] = useState<GitGraphMetadata | null>(null);
  const [graphRows, setGraphRows] = useState<GitGraphRowView[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
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
  const [historyPathDialogOpen, setHistoryPathDialogOpen] = useState(false);
  const [historyPathSearchText, setHistoryPathSearchText] = useState("");
  const [historyPathSelection, setHistoryPathSelection] = useState<string[]>([]);
  const [historyPathExpanded, setHistoryPathExpanded] = useState<string[]>([]);
  const [historyBranchDialogOpen, setHistoryBranchDialogOpen] = useState(false);
  const [historyTagDialogOpen, setHistoryTagDialogOpen] = useState(false);
  const [historyResetDialogOpen, setHistoryResetDialogOpen] = useState(false);
  const [historyEditDialogOpen, setHistoryEditDialogOpen] = useState(false);
  const [historyDropDialogOpen, setHistoryDropDialogOpen] = useState(false);
  const [historyCompareDialogOpen, setHistoryCompareDialogOpen] = useState(false);
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
  const [commitSignoff, setCommitSignoff] = useState(false);
  const [commitAmend, setCommitAmend] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [commitDiffOpen, setCommitDiffOpen] = useState(false);
  const [commitDiffHash, setCommitDiffHash] = useState("");
  const [commitDiffActivePath, setCommitDiffActivePath] = useState("");
  const [commitDiffCache, setCommitDiffCache] = useState<Record<string, string | null>>({});
  const [stashMessage, setStashMessage] = useState("");
  const [popover, setPopover] = useState<PopoverState>(null);

  const deferredHistorySearch = useDeferredValue(historySearchText);
  const deferredHistoryPathSearch = useDeferredValue(historyPathSearchText);
  const currentRepoPath = panelState?.repoPath || browserPath;
  const repoName = repoNameFromPath(currentRepoPath);

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
      {
        key: "branches" as PanelTab,
        label: t("Branches"),
        icon: GitBranch,
        badge: localBranches.length ? String(localBranches.length) : "",
      },
      { key: "stash" as PanelTab, label: t("Stash"), icon: HardDrive, badge: stashes.length ? String(stashes.length) : "" },
      { key: "conflicts" as PanelTab, label: t("Conflicts"), icon: Layers, badge: conflicts.length ? String(conflicts.length) : "" },
    ],
    [panelState?.totalChanges, stashes.length, conflicts.length, localBranches.length, t],
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
    const rect = element.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const estHeight = Math.min(vh * 0.82, 720);
    const preferBelow = rect.bottom + 4 + estHeight <= vh - 8;
    const left = Math.max(8, Math.min(vw - width - 8, rect.right - width));
    const top = preferBelow
      ? rect.bottom + 4
      : Math.max(8, rect.top - 4 - Math.min(estHeight, rect.top - 8));
    setPopover({ kind, left, top, width, data });
  }

  function openPopoverAt(kind: PopoverKind, clientX: number, clientY: number, width: number, data?: unknown) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const left = Math.max(8, Math.min(vw - width - 8, clientX));
    const top = Math.max(8, Math.min(vh - 16, clientY));
    setPopover({ kind, left, top, width, data });
  }

  function openChangeFileMenu(event: ReactMouseEvent<HTMLButtonElement>, file: GitPanelState["stagedFiles"][number], staged: boolean) {
    event.preventDefault();
    openPopoverAt("changeFileMenu", event.clientX, event.clientY, 196, {
      file,
      staged,
    } satisfies ChangeFileMenuState);
  }

  async function loadPanelState() {
    try {
      const next = await cmd.gitPanelState(browserPath);
      setPanelState(next);
      setGitReady(true);
      setGitError("");
    } catch (error) {
      setPanelState(null);
      setGitReady(false);
      setGitError(extractErrorMessage(error, t));
    }
  }

  async function loadGraphMetadata() {
    if (!gitReady) return;
    try {
      setGraphMetadata(await cmd.gitGraphMetadata(currentRepoPath));
    } catch {
      setGraphMetadata(null);
    }
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

  async function loadGraphRows() {
    if (!gitReady) return;
    setHistoryLoading(true);
    try {
      const rows = await cmd.gitGraphHistory({
        path: currentRepoPath,
        limit: 180,
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
    } catch (error) {
      showBanner(false, extractErrorMessage(error, t));
      setGraphRows([]);
    } finally {
      setHistoryLoading(false);
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
  // then poll every 3s. Hidden panels sit idle — the RightSidebar keep-
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
    const timer = window.setInterval(tick, 3000);
    const onVisibility = () => {
      if (document.visibilityState === "visible") tick();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [isActive, browserPath]);

  useEffect(() => {
    if (!banner) return undefined;
    const timer = window.setTimeout(() => setBanner(null), 2800);
    return () => window.clearTimeout(timer);
  }, [banner]);

  useEffect(() => {
    if (!gitReady) return;
    void loadGraphMetadata();
    void loadTags();
    void loadRemotes();
    void loadConfigEntries();
    void loadSubmodules();
  }, [gitReady, currentRepoPath]);

  useEffect(() => {
    if (!gitReady) return;
    if (selectedTab === "history") {
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
    if (selectedTab === "branches") {
      void loadGraphMetadata();
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
    historyPaths.join("\n"),
    historySortMode,
    historyShowLongEdges,
  ]);

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
    setHistorySelectedHash((current) => (graphRows.some((row) => row.hash === current) ? current : graphRows[0].hash));
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

  function historyPathSelectionState(node: RepoPathTreeNode) {
    if (historyPathSelection.includes(node.path)) return "selected";
    if (node.kind === "directory" && historyPathSelection.some((path) => path.startsWith(`${node.path}/`))) {
      return "partial";
    }
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
            <span className="git-path-row__icon">{node.kind === "directory" ? <Folder size={12} /> : <FileText size={12} />}</span>
            <span className="git-path-row__text">{node.name}</span>
            {node.kind === "directory" && node.children.length ? (
              <span className="git-path-row__meta">{node.children.length}</span>
            ) : null}
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

  function historyRowShouldDim(row: GitGraphRowView) {
    switch (historyHighlightMode) {
      case "mine":
        return !!row.author && row.author !== (graphMetadata?.gitUserName || "");
      case "merge":
        return !historyRowIsMerge(row);
      case "branch":
        return !row.refs.includes(panelState?.currentBranch || "") && !row.refs.includes("HEAD");
      default:
        return false;
    }
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
          <span className="git-history-inline__hash">{detail.shortHash}</span>
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
              <button
                key={`${detail.hash}-${file.path}`}
                type="button"
                className="git-history-inline__file"
                onClick={() => openCommitMultiDiff(detail, file.path)}
                title={t("Open diff") + " · " + file.path}
              >
                <span className="git-history-inline__file-delta mono">
                  {file.additions > 0 ? <span className="git-file-row__delta-add">+{file.additions}</span> : null}
                  {file.deletions > 0 ? <span className="git-file-row__delta-del">−{file.deletions}</span> : null}
                </span>
                <span className="git-history-inline__file-path mono" title={file.path}>{file.path}</span>
              </button>
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
    setConfigComposerExpanded(true);
    setConfigDraftKey(entry.key);
    setConfigDraftValue(entry.value);
    setConfigDraftGlobal(entry.scope === "global");
  }

  async function openBranchMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadBranchesMenu();
    openPopoverFromElement("branchMenu", event.currentTarget, 224);
  }

  async function openTagManager(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadTags();
    openPopoverFromElement("tagManager", event.currentTarget, 344);
  }

  async function openRemoteManager(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadRemotes();
    openPopoverFromElement("remoteManager", event.currentTarget, 372);
  }

  async function openConfigManager(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadConfigEntries();
    openPopoverFromElement("configManager", event.currentTarget, 372);
  }

  async function openRebaseManager(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadRebase();
    openPopoverFromElement("rebaseManager", event.currentTarget, 432);
  }

  async function openSubmoduleManager(event: ReactMouseEvent<HTMLButtonElement>) {
    await loadSubmodules();
    openPopoverFromElement("submoduleManager", event.currentTarget, 392);
  }

  const workingTreeClean = panelState?.workingTreeClean ?? true;

  if (!browserPath) {
    return <div className="git-panel git-panel--loading">{t("Loading Git panel…")}</div>;
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
              <div className="git-changes">
                  {panelState?.stagedFiles.length ? (
                    <section className="git-surface git-file-section git-file-section--staged">
                      <div className="git-file-section__header">
                        <div className="git-file-section__title-wrap">
                          <span className="git-file-section__dot git-file-section__dot--success" />
                          <span className="git-file-section__title">{t("Staged")}</span>
                          <span className="git-file-section__count">{panelState.stagedFiles.length}</span>
                          <span className="git-file-section__help">{t("Files ready to commit")}</span>
                        </div>
                        <GitButton
                          compact
                          disabled={busy}
                          onClick={() => void runGitAction(() => cmd.gitUnstageAll(currentRepoPath))}
                        >
                          {t("Unstage all")}
                        </GitButton>
                      </div>
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
                    </section>
                  ) : null}

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
                          checked={commitSignoff}
                          onChange={(event) => setCommitSignoff(event.currentTarget.checked)}
                        />
                        <span>{t("Sign off")}</span>
                      </label>
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
                              signoff: commitSignoff,
                              amend: commitAmend,
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
                              signoff: commitSignoff,
                              amend: commitAmend,
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
              </div>
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
                      placeholder={t("Search commit message or hash")}
                      value={historySearchText}
                    />
                    {historySearchText ? <button onClick={() => setHistorySearchText("")} type="button"><X size={11} /></button> : null}
                  </label>

                  <select
                    className="git-select git-history__select git-history__select--branch"
                    onChange={(event) => setHistoryBranchFilter(event.currentTarget.value)}
                    value={historyBranchFilter}
                  >
                    <option value="">{t("All branches")}</option>
                    {(graphMetadata?.branches || []).map((branch) => (
                      <option key={branch} value={branch}>
                        {branch}
                      </option>
                    ))}
                  </select>

                  <select
                    className="git-select git-history__select git-history__select--author"
                    onChange={(event) => setHistoryAuthorFilter(event.currentTarget.value)}
                    value={historyAuthorFilter}
                  >
                    <option value="">{t("All authors")}</option>
                    {(graphMetadata?.authors || []).map((author) => (
                      <option key={author} value={author}>
                        {author}
                      </option>
                    ))}
                  </select>

                  <select
                    className="git-select git-history__select git-history__select--date"
                    onChange={(event) => setHistoryDateFilter(event.currentTarget.value)}
                    value={historyDateFilter}
                  >
                    <option value="all">{t("Any time")}</option>
                    <option value="7d">{t("Last 7 days")}</option>
                    <option value="30d">{t("Last 30 days")}</option>
                    <option value="90d">{t("Last 90 days")}</option>
                    <option value="365d">{t("Last year")}</option>
                  </select>

                  <GitButton
                    compact
                    className="git-history__path-summary git-history__toolbar-button"
                    onClick={() => {
                      setHistoryPathSelection(historyPaths);
                      setHistoryPathSearchText("");
                      setHistoryPathExpanded(defaultExpandedHistoryPaths(graphMetadata?.repoFiles || [], historyPaths));
                      setHistoryPathDialogOpen(true);
                    }}
                  >
                    <Folder size={11} />
                    {historyPathSummary()}
                    <ChevronDown size={10} />
                  </GitButton>

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

              <section className="git-surface git-history-list-surface">
                {historyLoading ? (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={t("Loading commit graph…")}
                    icon={History}
                    title={t("Loading")}
                  />
                ) : graphRows.length ? (
                  <>
                    <div className="git-history-columns">
                      <div className="git-history-columns__graph" />
                      <div className="git-history-columns__subject">{t("Subject")}</div>
                      {historyShowAuthor ? <div className="git-history-columns__author">{t("Author")}</div> : null}
                      {historyShowDate ? <div className="git-history-columns__date">{t("Date")}</div> : null}
                      {historyShowHash ? <div className="git-history-columns__hash">{t("Hash")}</div> : null}
                    </div>
                    <div className="git-history-list">
                      {graphRows.map((row, index) => {
                        const active = row.hash === historySelectedHash;
                        const dimmed = historyRowShouldDim(row);
                        const refs = refTokens(row.refs);
                        return (
                          <div
                            key={row.hash}
                            className={[
                              "git-history-entry",
                              active ? "git-history-entry--active" : "",
                              dimmed ? "git-history-entry--dimmed" : "",
                              historyShowZebraStripes && index % 2 === 1 ? "git-history-entry--zebra" : "",
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
                              onClick={() => setHistorySelectedHash(active ? "" : row.hash)}
                              onDoubleClick={() => {
                                setHistorySelectedHash(row.hash);
                                void loadCommitDetail(row.hash).then((detail) => {
                                  if (detail) openCommitMultiDiff(detail);
                                });
                              }}
                              onContextMenu={(event) => {
                                event.preventDefault();
                                setHistoryContextCommit(row);
                                openPopoverAt("historyCommit", event.clientX, event.clientY, 232, row);
                              }}
                              type="button"
                              title={`${row.shortHash} · ${row.message}`}
                            >
                              <GitGraphLane row={row} />
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
                                {historyShowAuthor ? (
                                  <span className="git-history-row__author" title={row.author}>
                                    <span
                                      className="git-history-row__avatar"
                                      style={{ background: authorColor(row.author) }}
                                      aria-hidden="true"
                                    >
                                      {authorInitial(row.author)}
                                    </span>
                                    <span className="git-history-row__author-name">{row.author}</span>
                                  </span>
                                ) : null}
                                {historyShowDate ? <span className="git-history-row__date" title={formatGraphDate(row.dateTimestamp)}>{formatGraphDate(row.dateTimestamp)}</span> : null}
                                {historyShowHash ? <span className="git-history-row__hash" title={row.shortHash}>{row.shortHash}</span> : null}
                              </div>
                            </button>
                            {active && activeCommitDetail ? renderHistoryInlineDetail(activeCommitDetail) : null}
                          </div>
                        );
                      })}
                    </div>
                  </>
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

          {selectedTab === "branches" ? (
            <div className="git-branches-view">
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
                  subtitle={t("Create, switch, rename, and manage tracking")}
                  title={t("Branches")}
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
                                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitCheckoutBranch(currentRepoPath, branch))}>
                                      {t("Switch")}
                                    </GitButton>
                                  ) : null}
                                  {!current ? (
                                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitMergeBranch(currentRepoPath, branch))}>
                                      {t("Merge")}
                                    </GitButton>
                                  ) : null}
                                  <GitButton compact onClick={() => { setBranchRenameSource(branch); setBranchRenameTarget(branch); }}>
                                    {t("Rename")}
                                  </GitButton>
                                  {!current ? (
                                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitDeleteBranch(currentRepoPath, branch))}>
                                      {t("Delete")}
                                    </GitButton>
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
                                  <GitButton
                                    compact
                                    onClick={() =>
                                      void runGitAction(() => cmd.gitCheckoutTarget(currentRepoPath, branch.replace(/^[^/]+\//, ""), branch))
                                    }
                                  >
                                    {t("Checkout")}
                                  </GitButton>
                                  <GitButton compact onClick={() => { setBranchRenameSource(branch); setBranchRenameTarget(branch.replace(/^[^/]+\//, "")); }}>
                                    {t("Rename")}
                                  </GitButton>
                                  <GitButton
                                    compact
                                    onClick={() => {
                                      const parts = branch.split("/");
                                      const remoteName = parts.shift() || "origin";
                                      const remoteBranch = parts.join("/");
                                      void runGitAction(() => cmd.gitDeleteRemoteBranch(currentRepoPath, remoteName, remoteBranch));
                                    }}
                                  >
                                    {t("Delete")}
                                  </GitButton>
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
          ) : null}

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
                    {stashes.map((stash) => (
                      <div
                        key={stash.index}
                        className="git-stash-row"
                        onContextMenu={(event) => {
                          event.preventDefault();
                          openPopoverAt("stashMenu", event.clientX, event.clientY, 188, stash);
                        }}
                      >
                        <div className="git-stash-row__copy">
                          <div className="git-stash-row__message">{stash.message || "WIP"}</div>
                          <div className="git-stash-row__meta">{`stash@{${stash.index}} · ${stash.relativeDate}`}</div>
                        </div>
                        <div className="git-stash-row__actions">
                          <GitButton compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitStashApply(currentRepoPath, stash.index), { stash: true })}>
                            {t("Apply")}
                          </GitButton>
                          <GitButton compact disabled={busy} onClick={() => void runGitAction(() => cmd.gitStashPop(currentRepoPath, stash.index), { stash: true })}>
                            {t("Pop")}
                          </GitButton>
                        </div>
                      </div>
                    ))}
                  </div>
                ) : (
                  <GitEmptyState
                    accent="var(--accent)"
                    description={t("Use stash to park unfinished work without leaving the current branch.")}
                    icon={HardDrive}
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

                            <div className="git-conflict-hunks">
                              {selectedConflictHunks.map((hunk, index) => (
                                <div key={`${selectedConflictFile.path}-hunk-${index}`} className="git-card git-card--inset git-conflict-hunk">
                                  <GitSectionHeader
                                    actions={
                                      hunk.resolution ? (
                                        <GitPill tone={hunk.resolution === "theirs" ? "info" : hunk.resolution === "both" ? "warning" : "success"}>
                                          {hunk.resolution === "theirs" ? t("Theirs") : hunk.resolution === "both" ? t("Both") : t("Ours")}
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
                                  <div className="git-conflict-hunk__columns">
                                    <div className="git-conflict-hunk__side git-conflict-hunk__side--ours">
                                      <div className="git-conflict-hunk__label">{t("Ours")}</div>
                                      {hunk.oursLines.map((line, lineIndex) => (
                                        <div key={`ours-${lineIndex}-${line}`} className="git-conflict-hunk__line">
                                          {line || " "}
                                        </div>
                                      ))}
                                    </div>
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

      <GitPopover kind="historyOptions" onClose={() => setPopover(null)} popover={popover}>
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Sort")}</div>
          <GitMenuItem active={historySortMode === "topo"} onClick={() => setHistorySortMode("topo")}>{t("Topology order")}</GitMenuItem>
          <GitMenuItem active={historySortMode === "date"} onClick={() => setHistorySortMode("date")}>{t("Date order")}</GitMenuItem>
        </div>
        <div className="git-popover-divider" />
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Graph options")}</div>
          <GitMenuItem active={historyFirstParent} onClick={() => setHistoryFirstParent((value) => !value)}>{t("First parent only")}</GitMenuItem>
          <GitMenuItem active={historyNoMerges} onClick={() => setHistoryNoMerges((value) => !value)}>{t("Hide merge commits")}</GitMenuItem>
          <GitMenuItem active={historyShowLongEdges} onClick={() => setHistoryShowLongEdges((value) => !value)}>{t("Expand long edges")}</GitMenuItem>
        </div>
        <div className="git-popover-divider" />
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Highlight")}</div>
          <GitMenuItem active={historyHighlightMode === "none"} onClick={() => setHistoryHighlightMode("none")}>{t("No highlight")}</GitMenuItem>
          <GitMenuItem active={historyHighlightMode === "mine"} onClick={() => setHistoryHighlightMode("mine")}>{t("My commits")}</GitMenuItem>
          <GitMenuItem active={historyHighlightMode === "merge"} onClick={() => setHistoryHighlightMode("merge")}>{t("Merge commits")}</GitMenuItem>
          <GitMenuItem active={historyHighlightMode === "branch"} onClick={() => setHistoryHighlightMode("branch")}>{t("Current branch")}</GitMenuItem>
        </div>
        <div className="git-popover-divider" />
        <div className="git-popover-section">
          <div className="git-popover-label">{t("Display")}</div>
          <GitMenuItem active={historyShowZebraStripes} onClick={() => setHistoryShowZebraStripes((value) => !value)}>{t("Zebra stripes")}</GitMenuItem>
          <GitMenuItem active={historyShowHash} onClick={() => setHistoryShowHash((value) => !value)}>{t("Show hash column")}</GitMenuItem>
          <GitMenuItem active={historyShowAuthor} onClick={() => setHistoryShowAuthor((value) => !value)}>{t("Show author column")}</GitMenuItem>
          <GitMenuItem active={historyShowDate} onClick={() => setHistoryShowDate((value) => !value)}>{t("Show date column")}</GitMenuItem>
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
                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitPushTag(currentRepoPath, tag.name), { tags: true })}>{t("Push")}</GitButton>
                    <GitButton compact onClick={() => void copyText(tag.hash)}>{t("Copy hash")}</GitButton>
                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitDeleteTag(currentRepoPath, tag.name), { tags: true })}>{t("Delete")}</GitButton>
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
                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitFetchRemote(currentRepoPath, remote.name), { remotes: true })}>{t("Fetch")}</GitButton>
                    <GitButton compact onClick={() => beginRemoteEdit(remote)}>{t("Edit")}</GitButton>
                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitRemoveRemote(currentRepoPath, remote.name), { remotes: true })}>{t("Remove")}</GitButton>
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
        <div className="git-manager">
          <GitSectionHeader
            actions={
              <>
                <GitIconButton active={configComposerExpanded} aria-label={configComposerExpanded ? t("Hide composer") : t("Add setting")} icon={configComposerExpanded ? X : Plus} onClick={() => {
                  setConfigComposerExpanded((value) => !value);
                  if (configComposerExpanded) {
                    setConfigDraftKey("");
                    setConfigDraftValue("");
                    setConfigDraftGlobal(false);
                  }
                }} />
                <GitIconButton aria-label={t("Reload config")} icon={RefreshCw} onClick={() => void loadConfigEntries()} />
              </>
            }
            subtitle={t("View and edit local or global Git configuration")}
            title={t("Config")}
          />
          {configComposerExpanded ? (
            <div className="git-card git-card--inset">
              {configDraftKey ? <div className="git-inline-note">{`${t("Editing")} ${configDraftKey}`}</div> : null}
              <div className="git-segmented">
                <button className={!configDraftGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigDraftGlobal(false)} type="button">{t("Local")}</button>
                <button className={configDraftGlobal ? "git-segmented__item git-segmented__item--active" : "git-segmented__item"} onClick={() => setConfigDraftGlobal(true)} type="button">{t("Global")}</button>
              </div>
              <input className="git-input" onChange={(event) => setConfigDraftKey(event.currentTarget.value)} placeholder={t("Config key")} value={configDraftKey} />
              <input className="git-input" onChange={(event) => setConfigDraftValue(event.currentTarget.value)} placeholder={t("Config value")} value={configDraftValue} />
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
                    )
                  }
                >
                  {t("Set value")}
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
          <div className="git-manager-list">
            {configEntries
              .filter((entry) => entry.scope === (configSelectedGlobal ? "global" : "local"))
              .filter((entry) => {
                const needle = configSearchText.trim().toLowerCase();
                if (!needle) return true;
                return entry.key.toLowerCase().includes(needle) || entry.value.toLowerCase().includes(needle);
              })
              .map((entry) => (
                <div className="git-manager-row" key={`${entry.scope}-${entry.key}`}>
                  <span className="git-manager-row__dot git-manager-row__dot--neutral" />
                  <div className="git-manager-row__copy">
                    <div className="git-manager-row__title">{entry.key}</div>
                    <div className="git-manager-row__subtitle">{entry.value}</div>
                  </div>
                  <span className="git-manager-row__meta">{entry.scope}</span>
                  <div className="git-manager-row__actions">
                    <GitButton compact onClick={() => beginConfigEdit(entry)}>{t("Edit")}</GitButton>
                    <GitButton compact onClick={() => void copyText(entry.value)}>{t("Copy")}</GitButton>
                    <GitButton compact onClick={() => void runGitAction(() => cmd.gitUnsetConfigValue(currentRepoPath, entry.key, configSelectedGlobal), { config: true })}>{t("Unset")}</GitButton>
                  </div>
                </div>
              ))}
            {!configEntries.filter((entry) => entry.scope === (configSelectedGlobal ? "global" : "local")).length ? (
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
            ) : null}
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
                    <GitButton compact onClick={() => void copyText(submodule.path)}>{t("Copy path")}</GitButton>
                    {submodule.url ? <GitButton compact onClick={() => void copyText(submodule.url)}>{t("Copy URL")}</GitButton> : null}
                  </div>
                </div>
              ))
            ) : (
              <GitEmptyState accent="var(--accent)" description={t("Nested repositories will appear here after you add or initialize them.")} icon={Layers} title={t("No submodules")} />
            )}
          </div>
        </div>
      </GitPopover>

      <GitDialog
        footer={
          <>
            <GitButton compact onClick={() => setHistoryPathSelection([])}>{t("Clear")}</GitButton>
            <div className="git-commit-actions__spacer" />
            <GitButton compact onClick={() => setHistoryPathDialogOpen(false)}>{t("Cancel")}</GitButton>
            <GitButton
              tone="primary"
              compact
              onClick={() => {
                setHistoryPaths(historyPathSelection);
                setHistoryPathDialogOpen(false);
              }}
            >
              {t("Apply")}
            </GitButton>
          </>
        }
        onClose={() => setHistoryPathDialogOpen(false)}
        open={historyPathDialogOpen}
        subtitle={t("Filter commit graph to specific repository paths")}
        title={t("Tracked files")}
      >
        <label className="git-search">
          <Search size={12} />
          <input onChange={(event) => setHistoryPathSearchText(event.currentTarget.value)} placeholder={t("Search tracked files")} value={historyPathSearchText} />
          {historyPathSearchText ? <button onClick={() => setHistoryPathSearchText("")} type="button"><X size={11} /></button> : null}
        </label>
        <div className="git-card git-card--inset git-card--fill">
          {filteredHistoryPathTree.length ? (
            <div className="git-path-list git-path-tree">
              {renderHistoryPathTree(filteredHistoryPathTree)}
            </div>
          ) : (
            <GitEmptyState accent="var(--accent)" description={t("Try a different search or refresh repository metadata.")} icon={Folder} title={t("No tracked files")} />
          )}
        </div>
      </GitDialog>

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
