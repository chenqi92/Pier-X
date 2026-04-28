import { useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Hash,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import * as cmd from "../lib/commands";
import type { CaddyNode, CaddyParseResult } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";

// Editable structured tree view for a parsed Caddyfile.
//
// Reads the dirty editor buffer, parses to AST on every change, and
// when the user edits a node we mutate the AST locally then call
// `caddy_render` to project the result back to the buffer (parent
// receives the new text via `onChange`). Editing is intentionally
// node-level (rename / edit args / add child / remove subtree) — for
// directive-specific form fields the user goes to the Features pane.

type Props = {
  /** Current text contents of the active Caddyfile. */
  content: string;
  /** Called with the new buffer text when an edit is applied. When
   *  omitted, the tree renders read-only. */
  onChange?: (text: string) => void;
};

type Path = number[];

export default function CaddyTreeView({ content, onChange }: Props) {
  const { t } = useI18n();
  const [parse, setParse] = useState<CaddyParseResult | null>(null);
  const [parseError, setParseError] = useState("");
  const [busy, setBusy] = useState(false);

  const editable = !!onChange;

  useEffect(() => {
    let cancelled = false;
    setParseError("");
    cmd
      .caddyParse(content)
      .then((result) => {
        if (cancelled) return;
        setParse(result);
      })
      .catch((e) => {
        if (cancelled) return;
        setParseError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [content]);

  const applyMutation = async (next: CaddyNode[]) => {
    if (!onChange) return;
    setBusy(true);
    try {
      const text = await cmd.caddyRender(next);
      onChange(text);
    } catch (e) {
      setParseError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (parseError) {
    return (
      <div className="ws-tree__empty">
        <div className="status-note mono status-note--error">{parseError}</div>
      </div>
    );
  }

  if (!parse) {
    return (
      <div className="ws-tree__empty">
        <div className="status-note mono">{t("Parsing Caddyfile…")}</div>
      </div>
    );
  }

  return (
    <div className="ws-tree">
      {parse.errors.length > 0 && (
        <div className="ws-tree__warnings">
          <div className="ws-tree__warnings-title mono">
            {t("Parse warnings:")}
          </div>
          {parse.errors.map((err, i) => (
            <div key={i} className="ws-tree__warning mono">
              · {err}
            </div>
          ))}
        </div>
      )}
      {parse.nodes.length === 0 && (
        <div className="status-note mono">{t("(empty Caddyfile)")}</div>
      )}
      <div className="ws-tree__nodes">
        {parse.nodes.map((n, i) => (
          <NodeView
            key={i}
            node={n}
            depth={0}
            path={[i]}
            allNodes={parse.nodes}
            editable={editable}
            busy={busy}
            onMutate={applyMutation}
            t={t}
          />
        ))}
      </div>
      {editable && (
        <button
          type="button"
          className="ws-tree__add"
          disabled={busy}
          onClick={() => {
            const created = newDirective("# new directive", []);
            void applyMutation([...parse.nodes, created]);
          }}
        >
          <Plus size={11} /> {t("Add top-level directive")}
        </button>
      )}
    </div>
  );
}

function NodeView({
  node,
  depth,
  path,
  allNodes,
  editable,
  busy,
  onMutate,
  t,
}: {
  node: CaddyNode;
  depth: number;
  path: Path;
  allNodes: CaddyNode[];
  editable: boolean;
  busy: boolean;
  onMutate: (next: CaddyNode[]) => void | Promise<void>;
  t: (s: string) => string;
}) {
  if (node.kind === "comment") {
    return (
      <div className="ws-tree-card ws-tree-card--comment">
        <div className="ws-tree-card__head mono">
          <Hash size={10} /> {node.text}
        </div>
        {editable && (
          <NodeActions
            allNodes={allNodes}
            path={path}
            kind="comment"
            busy={busy}
            onMutate={onMutate}
            t={t}
          />
        )}
      </div>
    );
  }
  return (
    <DirectiveCard
      directive={node}
      depth={depth}
      path={path}
      allNodes={allNodes}
      editable={editable}
      busy={busy}
      onMutate={onMutate}
      t={t}
    />
  );
}

function DirectiveCard({
  directive,
  depth,
  path,
  allNodes,
  editable,
  busy,
  onMutate,
  t,
}: {
  directive: Extract<CaddyNode, { kind: "directive" }>;
  depth: number;
  path: Path;
  allNodes: CaddyNode[];
  editable: boolean;
  busy: boolean;
  onMutate: (next: CaddyNode[]) => void | Promise<void>;
  t: (s: string) => string;
}) {
  const isBlock = directive.block !== null;
  const [expanded, setExpanded] = useState(depth < 2);
  const [editing, setEditing] = useState(false);

  const title = useMemo(() => {
    if (!directive.name) return t("(global options)");
    return directive.name;
  }, [directive.name, t]);
  const argSummary =
    directive.args.length > 0 ? directive.args.join(" ") : "";

  if (editing) {
    return (
      <div className="ws-tree-card ws-tree-card--editing">
        <DirectiveEditor
          directive={directive}
          path={path}
          allNodes={allNodes}
          busy={busy}
          onCancel={() => setEditing(false)}
          onCommit={async (next) => {
            await onMutate(replaceAt(allNodes, path, next));
            setEditing(false);
          }}
          t={t}
        />
      </div>
    );
  }

  return (
    <div
      className={`ws-tree-card ${isBlock ? "ws-tree-card--block" : ""} ${
        expanded ? "is-expanded" : ""
      }`}
    >
      <div className="ws-tree-card__head-row">
        <button
          type="button"
          className="ws-tree-card__head"
          onClick={() => isBlock && setExpanded(!expanded)}
        >
          <span className="ws-tree-card__chevron">
            {isBlock ? (
              expanded ? (
                <ChevronDown size={11} />
              ) : (
                <ChevronRight size={11} />
              )
            ) : (
              <FileText size={10} />
            )}
          </span>
          <span className="ws-tree-card__title mono">{title}</span>
          {argSummary && (
            <span className="ws-tree-card__args mono">{argSummary}</span>
          )}
          {directive.inlineComment && (
            <span className="ws-tree-card__inline-comment mono">
              # {directive.inlineComment}
            </span>
          )}
        </button>
        {editable && (
          <NodeActions
            allNodes={allNodes}
            path={path}
            kind="directive"
            isBlock={isBlock}
            busy={busy}
            onEdit={() => setEditing(true)}
            onMutate={onMutate}
            t={t}
          />
        )}
      </div>
      {isBlock && expanded && (
        <div className="ws-tree-card__body">
          {directive.block!.length === 0 && (
            <div className="status-note mono">{t("(empty block)")}</div>
          )}
          {directive.block!.map((child, i) => (
            <NodeView
              key={i}
              node={child}
              depth={depth + 1}
              path={[...path, i]}
              allNodes={allNodes}
              editable={editable}
              busy={busy}
              onMutate={onMutate}
              t={t}
            />
          ))}
          {editable && (
            <button
              type="button"
              className="ws-tree-card__add-child"
              disabled={busy}
              onClick={() => {
                const created = newDirective("# new directive", []);
                const newBlock = [...directive.block!, created];
                onMutate(
                  replaceAt(allNodes, path, {
                    ...directive,
                    block: newBlock,
                  }),
                );
              }}
            >
              <Plus size={10} /> {t("Add child")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function NodeActions({
  allNodes,
  path,
  kind,
  isBlock,
  busy,
  onEdit,
  onMutate,
  t,
}: {
  allNodes: CaddyNode[];
  path: Path;
  kind: "comment" | "directive";
  isBlock?: boolean;
  busy: boolean;
  onEdit?: () => void;
  onMutate: (next: CaddyNode[]) => void | Promise<void>;
  t: (s: string) => string;
}) {
  const remove = () => {
    if (busy) return;
    const ok = window.confirm(t("Remove this node?"));
    if (!ok) return;
    onMutate(removeAt(allNodes, path));
  };
  return (
    <div className="ws-tree-card__actions">
      {kind === "directive" && onEdit && (
        <button
          type="button"
          className="ws-tree-card__icon-btn"
          onClick={(e) => {
            e.stopPropagation();
            onEdit();
          }}
          disabled={busy}
          title={isBlock ? t("Edit address / args") : t("Edit directive")}
        >
          <Pencil size={11} />
        </button>
      )}
      <button
        type="button"
        className="ws-tree-card__icon-btn ws-tree-card__icon-btn--danger"
        onClick={(e) => {
          e.stopPropagation();
          remove();
        }}
        disabled={busy}
        title={t("Remove")}
      >
        <Trash2 size={11} />
      </button>
    </div>
  );
}

function DirectiveEditor({
  directive,
  busy,
  onCancel,
  onCommit,
  t,
}: {
  directive: Extract<CaddyNode, { kind: "directive" }>;
  path: Path;
  allNodes: CaddyNode[];
  busy: boolean;
  onCancel: () => void;
  onCommit: (next: Extract<CaddyNode, { kind: "directive" }>) => Promise<void>;
  t: (s: string) => string;
}) {
  const [name, setName] = useState(directive.name);
  const [args, setArgs] = useState(directive.args.join(" "));

  const submit = () => {
    if (busy) return;
    void onCommit({
      ...directive,
      name: name.trim(),
      args: splitArgs(args),
    });
  };

  return (
    <div className="ws-tree-edit">
      <input
        className="ngx-input mono ws-tree-edit__name"
        value={name}
        spellCheck={false}
        autoFocus
        placeholder={t("name (e.g. example.com or reverse_proxy)")}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          else if (e.key === "Escape") onCancel();
        }}
      />
      <input
        className="ngx-input mono ws-tree-edit__args"
        value={args}
        spellCheck={false}
        placeholder={t("args (space-separated)")}
        onChange={(e) => setArgs(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          else if (e.key === "Escape") onCancel();
        }}
      />
      <button
        type="button"
        className="btn btn--primary"
        onClick={submit}
        disabled={busy}
      >
        {t("Save")}
      </button>
      <button
        type="button"
        className="btn btn--ghost"
        onClick={onCancel}
        disabled={busy}
        title={t("Cancel")}
      >
        <X size={11} />
      </button>
    </div>
  );
}

// ── AST helpers ─────────────────────────────────────────────────────

function newDirective(name: string, args: string[]): CaddyNode {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    block: null,
  };
}

/** Replace the node at `path` with `replacement`. Returns a new
 *  top-level array — does not mutate. */
function replaceAt(
  nodes: CaddyNode[],
  path: Path,
  replacement: CaddyNode,
): CaddyNode[] {
  if (path.length === 0) return nodes;
  if (path.length === 1) {
    const out = nodes.slice();
    out[path[0]] = replacement;
    return out;
  }
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || target.kind !== "directive" || target.block === null) {
    return nodes;
  }
  const out = nodes.slice();
  out[head] = {
    ...target,
    block: replaceAt(target.block, rest, replacement),
  };
  return out;
}

/** Remove the node at `path`. Returns a new top-level array. */
function removeAt(nodes: CaddyNode[], path: Path): CaddyNode[] {
  if (path.length === 0) return nodes;
  if (path.length === 1) {
    const out = nodes.slice();
    out.splice(path[0], 1);
    return out;
  }
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || target.kind !== "directive" || target.block === null) {
    return nodes;
  }
  const out = nodes.slice();
  out[head] = {
    ...target,
    block: removeAt(target.block, rest),
  };
  return out;
}

/** Tokenize an args string the same way the Caddyfile lexer does:
 *  whitespace separates, double quotes group, backslash escapes
 *  inside double quotes. Backticks group multi-line literal strings.
 *  Quoted values keep their quotes in the output so the renderer
 *  emits the same form. */
function splitArgs(s: string): string[] {
  const out: string[] = [];
  let cur = "";
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === " " || c === "\t") {
      if (cur) {
        out.push(cur);
        cur = "";
      }
      i++;
      continue;
    }
    if (c === '"') {
      cur += c;
      i++;
      while (i < s.length) {
        const cc = s[i];
        cur += cc;
        i++;
        if (cc === "\\" && i < s.length) {
          cur += s[i];
          i++;
          continue;
        }
        if (cc === '"') break;
      }
      continue;
    }
    if (c === "`") {
      cur += c;
      i++;
      while (i < s.length) {
        const cc = s[i];
        cur += cc;
        i++;
        if (cc === "`") break;
      }
      continue;
    }
    cur += c;
    i++;
  }
  if (cur) out.push(cur);
  return out;
}
