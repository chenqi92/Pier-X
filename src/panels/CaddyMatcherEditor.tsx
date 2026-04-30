import { useMemo, useState } from "react";
import { AlertTriangle, AtSign, Plus, Trash2 } from "lucide-react";
import type { CaddyNode } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import {
  collectScopes,
  getBlockAt,
  isDirective,
  newDirective,
  replaceBlockAt,
  type Scope,
} from "./caddyFeatures";

/** A named matcher in a Caddy site block — `@name <type> <args…>` or
 *  `@name { … }` for compound matchers. We track both forms so edits
 *  preserve the user's original shape. */
type MatcherRef = {
  /** Index inside the parent block. The matcher always lives at the
   *  top level of its scope's block (Caddy doesn't nest @-defs). */
  index: number;
  /** The `@xxx` token, including the leading `@`. */
  name: string;
  /** Inline form: type token (`path`, `host`, `method`, …). Empty
   *  string when the matcher uses the block form. */
  inlineType: string;
  /** Inline form: positional args after the type (the matcher value).
   *  Empty when block form. */
  inlineArgs: string[];
  /** Block form: the inner directives, each one is a sub-condition.
   *  `null` for the inline form. */
  blockBody: CaddyNode[] | null;
  /** How many other directives in the same scope reference this name
   *  as an argument (e.g. `handle @api { … }`). Helps users gauge the
   *  blast radius of a rename / delete. */
  usageCount: number;
};

const MATCHER_TYPE_OPTIONS = [
  { value: "path", label: "path" },
  { value: "path_regexp", label: "path_regexp" },
  { value: "host", label: "host" },
  { value: "method", label: "method" },
  { value: "header", label: "header" },
  { value: "header_regexp", label: "header_regexp" },
  { value: "query", label: "query" },
  { value: "protocol", label: "protocol" },
  { value: "remote_ip", label: "remote_ip" },
  { value: "expression", label: "expression" },
  { value: "not", label: "not" },
];

/** Walk one block and pull out every `@name` matcher together with
 *  its inline-or-block form and a count of how many other directives
 *  in the same block reference it. Naming-collision is left to Caddy
 *  itself — we never silently rename. */
function collectMatchers(block: CaddyNode[]): MatcherRef[] {
  const out: MatcherRef[] = [];
  for (let i = 0; i < block.length; i++) {
    const n = block[i];
    if (!isDirective(n)) continue;
    if (!n.name.startsWith("@")) continue;
    const blockBody = n.block;
    const inlineType = blockBody === null ? n.args[0] ?? "" : "";
    const inlineArgs = blockBody === null ? n.args.slice(1) : [];
    out.push({
      index: i,
      name: n.name,
      inlineType,
      inlineArgs,
      blockBody,
      usageCount: 0,
    });
  }
  // Second pass: count cross-references inside the same block.
  for (const ref of out) {
    let usage = 0;
    for (let i = 0; i < block.length; i++) {
      if (i === ref.index) continue;
      const n = block[i];
      if (!isDirective(n)) continue;
      if (n.args.includes(ref.name)) usage += 1;
    }
    ref.usageCount = usage;
  }
  return out;
}

/** Replace one matcher directive in a block. Caller has already
 *  built the new directive shape; we just splice it in. */
function setMatcherAt(
  block: CaddyNode[],
  index: number,
  next: CaddyNode,
): CaddyNode[] {
  if (index < 0 || index >= block.length) return block;
  const out = block.slice();
  out[index] = next;
  return out;
}

function removeMatcherAt(block: CaddyNode[], index: number): CaddyNode[] {
  if (index < 0 || index >= block.length) return block;
  const out = block.slice();
  out.splice(index, 1);
  return out;
}

/** Insert a new matcher directive at the top of the block, above any
 *  reverse_proxy/handle/redir lines that might reference it. */
function prependMatcher(block: CaddyNode[], d: CaddyNode): CaddyNode[] {
  return [d, ...block];
}

/** Pretty label for the matcher list — shows type + first value, or
 *  a "{N conditions}" hint for compound block-form matchers. */
function describeRef(ref: MatcherRef, t: (s: string, p?: any) => string): string {
  if (ref.blockBody !== null) {
    const conds = ref.blockBody.filter((c) => isDirective(c)).length;
    return t("{n} conditions (block form)", { n: conds });
  }
  if (!ref.inlineType) return "(empty matcher)";
  const tail = ref.inlineArgs.join(" ");
  return tail ? `${ref.inlineType} ${tail}` : ref.inlineType;
}

export default function CaddyMatcherEditor({
  nodes,
  onChange,
}: {
  nodes: CaddyNode[];
  onChange: (next: CaddyNode[]) => void;
}) {
  const { t } = useI18n();

  // Filter scopes to those where named matchers actually live —
  // Caddy permits `@name` in site blocks and snippets but not the
  // global config block.
  const allScopes = useMemo(() => collectScopes(nodes), [nodes]);
  const scopes = useMemo(
    () => allScopes.filter((s) => s.kind !== "global"),
    [allScopes],
  );
  const [scopeIdx, setScopeIdx] = useState(0);
  const safeIdx = scopeIdx < scopes.length ? scopeIdx : 0;
  const scope = scopes[safeIdx];

  const block = useMemo(
    () => (scope ? getBlockAt(nodes, scope.path) ?? [] : []),
    [scope, nodes],
  );
  const matchers = useMemo(() => collectMatchers(block), [block]);

  const applyBlock = (newBlock: CaddyNode[]) => {
    if (!scope) return;
    onChange(replaceBlockAt(nodes, scope.path, newBlock));
  };

  const handleAddInline = (name: string, type: string, value: string) => {
    if (!scope) return;
    const trimmedName = name.trim();
    const directiveName = trimmedName.startsWith("@")
      ? trimmedName
      : `@${trimmedName}`;
    const args = [type.trim(), ...value.split(/\s+/).filter(Boolean)];
    applyBlock(prependMatcher(block, newDirective(directiveName, args)));
  };

  return (
    <div className="caddy-mat">
      {scopes.length > 1 && (
        <ScopeBar scopes={scopes} active={safeIdx} onPick={setScopeIdx} />
      )}
      {!scope && (
        <div className="status-note mono">
          {t("(no site or snippet blocks in this file)")}
        </div>
      )}
      {scope && (
        <>
          <NewMatcherForm onAdd={handleAddInline} />
          {matchers.length === 0 && (
            <div className="status-note mono">
              {t("(no named matchers in this scope)")}
            </div>
          )}
          {matchers.map((ref) => (
            <MatcherCard
              key={`${ref.index}-${ref.name}`}
              ref0={ref}
              describe={(r) => describeRef(r, t)}
              onRename={(nextName) => {
                const next = block[ref.index];
                if (!isDirective(next)) return;
                applyBlock(
                  setMatcherAt(block, ref.index, { ...next, name: nextName }),
                );
              }}
              onArgsChange={(type, value) => {
                const next = block[ref.index];
                if (!isDirective(next)) return;
                if (next.block !== null) return; // block-form matchers — leave alone
                const nextArgs = [
                  type,
                  ...value.split(/\s+/).filter(Boolean),
                ];
                applyBlock(
                  setMatcherAt(block, ref.index, { ...next, args: nextArgs }),
                );
              }}
              onDelete={() => applyBlock(removeMatcherAt(block, ref.index))}
            />
          ))}
        </>
      )}
    </div>
  );
}

function ScopeBar({
  scopes,
  active,
  onPick,
}: {
  scopes: Scope[];
  active: number;
  onPick: (i: number) => void;
}) {
  return (
    <div className="caddy-mat__scopes" role="tablist">
      {scopes.map((s, i) => (
        <button
          key={`${s.kind}-${s.path.join("-")}`}
          type="button"
          role="tab"
          aria-selected={i === active}
          className={`ngx-fc__scope ${i === active ? "is-active" : ""}`}
          onClick={() => onPick(i)}
          title={s.label}
        >
          <span className="mono">{s.label}</span>
        </button>
      ))}
    </div>
  );
}

function NewMatcherForm({
  onAdd,
}: {
  onAdd: (name: string, type: string, value: string) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  const [type, setType] = useState("path");
  const [value, setValue] = useState("");
  const valid = !!name.trim() && !!type.trim();

  const submit = () => {
    if (!valid) return;
    onAdd(name, type, value);
    setName("");
    setValue("");
  };

  return (
    <div className="caddy-mat__add">
      <Plus size={11} />
      <span className="caddy-mat__add-label">{t("New named matcher")}</span>
      <span className="caddy-mat__at mono">@</span>
      <input
        className="ngx-input mono caddy-mat__name-input"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="api"
        spellCheck={false}
      />
      <select
        className="ngx-input mono"
        value={type}
        onChange={(e) => setType(e.target.value)}
      >
        {MATCHER_TYPE_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <input
        className="ngx-input mono caddy-mat__value-input"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder="/api/*"
        spellCheck={false}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
        }}
      />
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        onClick={submit}
        disabled={!valid}
      >
        {t("Add")}
      </button>
    </div>
  );
}

function MatcherCard({
  ref0,
  describe,
  onRename,
  onArgsChange,
  onDelete,
}: {
  ref0: MatcherRef;
  describe: (r: MatcherRef) => string;
  onRename: (next: string) => void;
  onArgsChange: (type: string, value: string) => void;
  onDelete: () => void;
}) {
  const { t } = useI18n();
  const [draftName, setDraftName] = useState(ref0.name);
  const [draftType, setDraftType] = useState(ref0.inlineType);
  const [draftValue, setDraftValue] = useState(ref0.inlineArgs.join(" "));
  const [confirmDelete, setConfirmDelete] = useState(false);
  const isBlockForm = ref0.blockBody !== null;

  const commitName = () => {
    let v = draftName.trim();
    if (!v.startsWith("@")) v = `@${v}`;
    if (v === ref0.name) return;
    onRename(v);
  };

  const commitArgs = (typeNext: string, valueNext: string) => {
    if (typeNext === ref0.inlineType && valueNext === ref0.inlineArgs.join(" "))
      return;
    onArgsChange(typeNext, valueNext);
  };

  return (
    <div className="caddy-mat__card">
      <div className="caddy-mat__card-head">
        <AtSign size={11} />
        <input
          className="ngx-input mono caddy-mat__name-input"
          value={draftName}
          spellCheck={false}
          onChange={(e) => setDraftName(e.target.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
        />
        {!isBlockForm ? (
          <>
            <select
              className="ngx-input mono"
              value={draftType}
              onChange={(e) => {
                setDraftType(e.target.value);
                commitArgs(e.target.value, draftValue);
              }}
            >
              {MATCHER_TYPE_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
              {draftType && !MATCHER_TYPE_OPTIONS.some((o) => o.value === draftType) && (
                <option key={draftType} value={draftType}>
                  {draftType}
                </option>
              )}
            </select>
            <input
              className="ngx-input mono caddy-mat__value-input"
              value={draftValue}
              spellCheck={false}
              onChange={(e) => setDraftValue(e.target.value)}
              onBlur={() => commitArgs(draftType, draftValue)}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
            />
          </>
        ) : (
          <span className="caddy-mat__blockform mono">{describe(ref0)}</span>
        )}
        <span className="caddy-mat__usage mono" title={t("References within this scope")}>
          {t("× {n}", { n: ref0.usageCount })}
        </span>
        {!confirmDelete ? (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setConfirmDelete(true)}
            title={
              ref0.usageCount > 0
                ? t("Delete — {n} references will dangle", { n: ref0.usageCount })
                : t("Delete this matcher")
            }
          >
            <Trash2 size={11} />
          </button>
        ) : (
          <span className="caddy-mat__confirm">
            <AlertTriangle size={11} />
            {ref0.usageCount > 0 && (
              <span className="caddy-mat__warn mono">
                {t("{n} references", { n: ref0.usageCount })}
              </span>
            )}
            <button
              type="button"
              className="btn btn--neg btn--sm"
              onClick={() => {
                setConfirmDelete(false);
                onDelete();
              }}
            >
              {t("Delete")}
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => setConfirmDelete(false)}
            >
              {t("Cancel")}
            </button>
          </span>
        )}
      </div>
      {isBlockForm && ref0.blockBody && ref0.blockBody.length > 0 && (
        <div className="caddy-mat__block-preview mono">
          {ref0.blockBody
            .filter(isDirective)
            .slice(0, 6)
            .map((d, i) => (
              <span key={i} className="caddy-mat__block-line">
                {d.name} {d.args.join(" ")}
              </span>
            ))}
        </div>
      )}
    </div>
  );
}
