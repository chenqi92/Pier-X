import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import { useI18n } from "../../i18n/useI18n";
import type { RedisKeyEntry } from "../../lib/types";
import RedisTypeBadge from "./RedisTypeBadge";

type Props = {
  keys: RedisKeyEntry[];
  /** Currently selected key — used to colour the matching row. */
  selected: string | null;
  /** Type of the selected key (preferred over the per-row scan kind, since detail inspection is authoritative). */
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  /** Whether more keys remain on the server — drives the "Load more" button. */
  hasMore?: boolean;
  /** Click handler for "Load more". When omitted, the button is hidden. */
  onLoadMore?: () => void;
  /** Disabled flag for "Load more" while a request is in flight. */
  loadMoreBusy?: boolean;
  /** When true, group keys by their `:` separator into a
   *  collapsible tree. Only common namespaces collapse; leaf
   *  rows keep the same per-row chip layout as the flat view. */
  treeMode?: boolean;
  /** Separator for the tree split. Defaults to ":" — Redis
   *  convention. Pass a different glyph for projects that use
   *  another delimiter. */
  separator?: string;
};

/**
 * Key list. Defaults to the flat view; when `treeMode` is on,
 * keys with shared prefixes (split on `:`) collapse under
 * folder rows. Type / TTL chips ride on the leaf rows in both
 * modes — the backend's TYPE+PTTL pipeline drives them.
 */
export default function RedisKeyList({
  keys,
  selected,
  selectedKind,
  onSelect,
  hasMore,
  onLoadMore,
  loadMoreBusy,
  treeMode,
  separator = ":",
}: Props) {
  const { t } = useI18n();

  if (keys.length === 0) {
    return (
      <div className="rds-detail-empty" style={{ padding: "var(--sp-6) var(--sp-4)" }}>
        <div>{t("No keys match this pattern.")}</div>
      </div>
    );
  }

  if (!treeMode) {
    return (
      <div className="rds-keys">
        {keys.map((entry) => (
          <KeyRow
            key={entry.key}
            entry={entry}
            depth={0}
            selected={selected}
            selectedKind={selectedKind}
            onSelect={onSelect}
            t={t}
          />
        ))}
        {hasMore && onLoadMore && (
          <button
            type="button"
            className="btn is-ghost is-compact rds-load-more"
            onClick={onLoadMore}
            disabled={loadMoreBusy}
          >
            {loadMoreBusy ? t("Loading...") : t("Load more")}
          </button>
        )}
      </div>
    );
  }

  return (
    <KeyTree
      keys={keys}
      separator={separator}
      selected={selected}
      selectedKind={selectedKind}
      onSelect={onSelect}
      hasMore={hasMore}
      onLoadMore={onLoadMore}
      loadMoreBusy={loadMoreBusy}
      t={t}
    />
  );
}

type TreeNode = {
  /** Last segment shown in the row — `users` in `app:users:42`. */
  segment: string;
  /** Full path so far, including `segment`. Used as a stable
   *  React key and as the open-state map key. */
  path: string;
  /** When the node represents an actual key (i.e. the full path
   *  is itself a key on the server), this carries the entry. */
  entry?: RedisKeyEntry;
  /** Child folders / leaves keyed by their immediate segment. */
  children: Map<string, TreeNode>;
};

function buildTree(keys: RedisKeyEntry[], separator: string): TreeNode {
  const root: TreeNode = { segment: "", path: "", children: new Map() };
  for (const entry of keys) {
    const parts = entry.key.split(separator);
    let node = root;
    let acc = "";
    for (let i = 0; i < parts.length; i += 1) {
      const seg = parts[i];
      acc = acc === "" ? seg : `${acc}${separator}${seg}`;
      let next = node.children.get(seg);
      if (!next) {
        next = { segment: seg, path: acc, children: new Map() };
        node.children.set(seg, next);
      }
      node = next;
    }
    node.entry = entry;
  }
  return root;
}

function KeyTree({
  keys,
  separator,
  selected,
  selectedKind,
  onSelect,
  hasMore,
  onLoadMore,
  loadMoreBusy,
  t,
}: {
  keys: RedisKeyEntry[];
  separator: string;
  selected: string | null;
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  hasMore?: boolean;
  onLoadMore?: () => void;
  loadMoreBusy?: boolean;
  t: TFn;
}) {
  const root = useMemo(() => buildTree(keys, separator), [keys, separator]);
  // Folders default to open so the user can see their data on
  // first scan. The map only stores the *closed* set so a fresh
  // scan reveals new folders without us having to chase them.
  const [closed, setClosed] = useState<Set<string>>(() => new Set());
  const toggle = (path: string) =>
    setClosed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  return (
    <div className="rds-keys">
      {Array.from(root.children.values()).map((child) => (
        <TreeBranch
          key={child.path}
          node={child}
          depth={0}
          selected={selected}
          selectedKind={selectedKind}
          onSelect={onSelect}
          closed={closed}
          toggle={toggle}
          t={t}
        />
      ))}
      {hasMore && onLoadMore && (
        <button
          type="button"
          className="btn is-ghost is-compact rds-load-more"
          onClick={onLoadMore}
          disabled={loadMoreBusy}
        >
          {loadMoreBusy ? t("Loading...") : t("Load more")}
        </button>
      )}
    </div>
  );
}

function TreeBranch({
  node,
  depth,
  selected,
  selectedKind,
  onSelect,
  closed,
  toggle,
  t,
}: {
  node: TreeNode;
  depth: number;
  selected: string | null;
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  closed: Set<string>;
  toggle: (path: string) => void;
  t: TFn;
}) {
  const hasChildren = node.children.size > 0;
  // Pure-leaf node: render the same row the flat view uses.
  if (!hasChildren && node.entry) {
    return (
      <KeyRow
        entry={node.entry}
        depth={depth}
        selected={selected}
        selectedKind={selectedKind}
        onSelect={onSelect}
        t={t}
      />
    );
  }
  const open = !closed.has(node.path);
  return (
    <>
      <button
        type="button"
        className="rds-row rds-row--folder"
        onClick={() => toggle(node.path)}
        style={{ paddingLeft: `calc(var(--sp-2-5) + ${depth * 12}px)` }}
      >
        <span className="rds-folder-caret" aria-hidden>
          {open ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
        </span>
        <span className="rds-key">{node.segment}</span>
        <span className="rds-meta" />
        <span className="rds-meta" style={{ color: "var(--muted)" }}>
          {node.children.size}
        </span>
      </button>
      {open && (
        <>
          {/* Sibling leaf at this folder path (rare — happens
              when a key is both a prefix and a real key, e.g.
              `users` and `users:1`). */}
          {node.entry && (
            <KeyRow
              entry={node.entry}
              depth={depth + 1}
              selected={selected}
              selectedKind={selectedKind}
              onSelect={onSelect}
              t={t}
            />
          )}
          {Array.from(node.children.values()).map((child) => (
            <TreeBranch
              key={child.path}
              node={child}
              depth={depth + 1}
              selected={selected}
              selectedKind={selectedKind}
              onSelect={onSelect}
              closed={closed}
              toggle={toggle}
              t={t}
            />
          ))}
        </>
      )}
    </>
  );
}

function KeyRow({
  entry,
  depth,
  selected,
  selectedKind,
  onSelect,
  t,
}: {
  entry: RedisKeyEntry;
  depth: number;
  selected: string | null;
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  t: TFn;
}) {
  const isSelected = entry.key === selected;
  const kind = isSelected && selectedKind ? selectedKind : entry.kind;
  return (
    <button
      type="button"
      className={"rds-row" + (isSelected ? " selected" : "")}
      onClick={() => onSelect(entry.key)}
      style={depth > 0 ? { paddingLeft: `calc(var(--sp-2-5) + ${depth * 12}px)` } : undefined}
    >
      <RedisTypeBadge kind={kind} />
      <span className="rds-key">
        {/* In tree mode the row only shows the leaf segment so
            the folder header isn't repeated; the full key is in
            the title for hover. */}
        {depth > 0 ? entry.key.split(":").slice(-1)[0] : entry.key}
      </span>
      <span className="rds-meta" title={ttlTitle(entry.ttlSeconds, t)}>
        {formatTtl(entry.ttlSeconds, t)}
      </span>
      <span className="rds-meta" />
    </button>
  );
}

type TFn = (key: string, vars?: Record<string, string | number | null | undefined>) => string;

function formatTtl(ttl: number, _t: TFn): string {
  if (ttl === -1) return "∞";
  if (ttl < 0) return "—";
  if (ttl < 60) return `${ttl}s`;
  if (ttl < 3600) return `${Math.round(ttl / 60)}m`;
  if (ttl < 86400) return `${Math.round(ttl / 3600)}h`;
  return `${Math.round(ttl / 86400)}d`;
}

function ttlTitle(ttl: number, t: TFn): string {
  if (ttl === -1) return t("No expiry");
  if (ttl < 0) return t("Unknown TTL");
  return t("Expires in {seconds}s", { seconds: ttl });
}
