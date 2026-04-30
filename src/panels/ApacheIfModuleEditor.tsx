import { useMemo, useState } from "react";
import {
  AlertTriangle,
  GitBranch,
  Layers,
  Plus,
  Trash2,
} from "lucide-react";
import type { ApacheNode } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { isDirective, newSection } from "./apacheFeatures";

type IfModuleRef = {
  /** Path of indices into nested `section` arrays leading to the
   *  IfModule directive itself. Last index is into the parent block. */
  path: number[];
  moduleArg: string;
  negated: boolean;
  baseModule: string;
  /** Human-readable breadcrumb of the parent block — `(top-level)`,
   *  `<VirtualHost *:80>`, `<IfModule mod_ssl.c>`, etc. */
  parentLabel: string;
  childCount: number;
  /** First few directive names from the body, for a one-line preview
   *  so users can identify the block at a glance. */
  childPreview: string[];
};

function describeContainer(d: { name: string; args: string[] }): string {
  return d.args.length
    ? `<${d.name} ${d.args.join(" ")}>`
    : `<${d.name}>`;
}

/** Walk the tree and collect every `<IfModule>` directive, including
 *  nested ones. The path is captured as we descend so the caller can
 *  apply mutations without re-walking. */
function collectIfModules(nodes: ApacheNode[]): IfModuleRef[] {
  const out: IfModuleRef[] = [];
  const walk = (block: ApacheNode[], path: number[], parentLabel: string) => {
    for (let i = 0; i < block.length; i++) {
      const n = block[i];
      if (!isDirective(n) || n.section === null) continue;
      const here = [...path, i];
      const lname = n.name.toLowerCase();
      if (lname === "ifmodule") {
        const arg = n.args[0] ?? "";
        const negated = arg.startsWith("!");
        const baseModule = negated ? arg.slice(1) : arg;
        const childPreview: string[] = [];
        let childCount = 0;
        for (const c of n.section) {
          if (!isDirective(c)) continue;
          childCount += 1;
          if (childPreview.length < 5) {
            childPreview.push(
              c.section !== null ? `<${c.name}…>` : c.name,
            );
          }
        }
        out.push({
          path: here,
          moduleArg: arg,
          negated,
          baseModule,
          parentLabel,
          childCount,
          childPreview,
        });
      }
      walk(n.section, here, describeContainer(n));
    }
  };
  walk(nodes, [], "(top-level)");
  return out;
}

/** Replace the IfModule directive at `path` with one whose first arg
 *  is `newArg`. Pure — returns a fresh tree, leaves the input alone. */
function setIfModuleArg(
  nodes: ApacheNode[],
  path: number[],
  newArg: string,
): ApacheNode[] {
  if (path.length === 0) return nodes;
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || !isDirective(target)) return nodes;
  const out = nodes.slice();
  if (rest.length === 0) {
    out[head] = { ...target, args: [newArg] };
    return out;
  }
  if (target.section === null) return nodes;
  out[head] = {
    ...target,
    section: setIfModuleArg(target.section, rest, newArg),
  };
  return out;
}

/** Delete the IfModule directive at `path` and splice its children
 *  into the parent block in its place — an "unwrap" operation. */
function unwrapIfModuleAt(
  nodes: ApacheNode[],
  path: number[],
): ApacheNode[] {
  if (path.length === 0) return nodes;
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || !isDirective(target) || target.section === null) return nodes;
  const out = nodes.slice();
  if (rest.length === 0) {
    out.splice(head, 1, ...target.section);
    return out;
  }
  out[head] = {
    ...target,
    section: unwrapIfModuleAt(target.section, rest),
  };
  return out;
}

/** Append a new empty `<IfModule moduleArg>` to top-level. Users
 *  can drag directives into it via the raw editor afterwards — the
 *  point of this helper is just to scaffold the wrapper. */
function appendIfModule(
  nodes: ApacheNode[],
  moduleArg: string,
): ApacheNode[] {
  const wrapper = newSection("IfModule", [moduleArg], []);
  return [...nodes, wrapper];
}

export default function ApacheIfModuleEditor({
  nodes,
  onChange,
}: {
  nodes: ApacheNode[];
  onChange: (next: ApacheNode[]) => void;
}) {
  const { t } = useI18n();
  const refs = useMemo(() => collectIfModules(nodes), [nodes]);

  const [newModuleName, setNewModuleName] = useState("");

  const handleAdd = () => {
    const trimmed = newModuleName.trim();
    if (!trimmed) return;
    onChange(appendIfModule(nodes, trimmed));
    setNewModuleName("");
  };

  return (
    <div className="apache-ifm">
      <div className="apache-ifm__add">
        <Plus size={11} />
        <span className="apache-ifm__add-label">{t("New <IfModule>")}</span>
        <input
          className="ngx-input mono"
          value={newModuleName}
          onChange={(e) => setNewModuleName(e.target.value)}
          placeholder="mod_rewrite.c"
          spellCheck={false}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleAdd();
          }}
        />
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={handleAdd}
          disabled={!newModuleName.trim()}
          title={t("Append an empty <IfModule> wrapper at top-level")}
        >
          {t("Add")}
        </button>
      </div>

      {refs.length === 0 && (
        <div className="status-note mono">
          {t("(no <IfModule> blocks in this file)")}
        </div>
      )}

      {refs.map((ref) => (
        <IfModuleCard
          key={ref.path.join("-")}
          ref0={ref}
          onArgChange={(arg) =>
            onChange(setIfModuleArg(nodes, ref.path, arg))
          }
          onUnwrap={() => onChange(unwrapIfModuleAt(nodes, ref.path))}
        />
      ))}
    </div>
  );
}

function IfModuleCard({
  ref0,
  onArgChange,
  onUnwrap,
}: {
  ref0: IfModuleRef;
  onArgChange: (arg: string) => void;
  onUnwrap: () => void;
}) {
  const { t } = useI18n();
  const [draftBase, setDraftBase] = useState(ref0.baseModule);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const commit = (nextBase: string, nextNegated: boolean) => {
    const arg = `${nextNegated ? "!" : ""}${nextBase}`;
    if (arg === ref0.moduleArg) return;
    onArgChange(arg);
  };

  return (
    <div className="apache-ifm__card">
      <div className="apache-ifm__card-head">
        <Layers size={11} />
        <input
          className="ngx-input mono apache-ifm__name"
          value={draftBase}
          spellCheck={false}
          onChange={(e) => setDraftBase(e.target.value)}
          onBlur={() => commit(draftBase.trim(), ref0.negated)}
          onKeyDown={(e) => {
            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          }}
          placeholder="mod_X.c"
        />
        <label
          className="ngx-form__flag apache-ifm__neg"
          title={t("Negate the test (run body when module is NOT loaded)")}
        >
          <input
            type="checkbox"
            checked={ref0.negated}
            onChange={(e) => commit(draftBase.trim(), e.target.checked)}
          />
          <span className="mono">!</span>
        </label>
        <span className="apache-ifm__parent mono" title={ref0.parentLabel}>
          <GitBranch size={10} /> {ref0.parentLabel}
        </span>
        <span className="apache-ifm__count mono">
          {t("{n} directives", { n: ref0.childCount })}
        </span>
        {!confirmRemove ? (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setConfirmRemove(true)}
            title={t(
              "Remove the <IfModule> wrapper, keeping its body in the parent block",
            )}
          >
            <Trash2 size={11} /> {t("Unwrap")}
          </button>
        ) : (
          <span className="apache-ifm__confirm">
            <AlertTriangle size={11} />
            <span className="mono">{t("Unwrap?")}</span>
            <button
              type="button"
              className="btn btn--neg btn--sm"
              onClick={() => {
                setConfirmRemove(false);
                onUnwrap();
              }}
            >
              {t("Yes")}
            </button>
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => setConfirmRemove(false)}
            >
              {t("Cancel")}
            </button>
          </span>
        )}
      </div>
      {ref0.childPreview.length > 0 && (
        <div className="apache-ifm__preview mono">
          {ref0.childPreview.join(" · ")}
          {ref0.childCount > ref0.childPreview.length && (
            <span className="apache-ifm__preview-more">
              {" "}…+{ref0.childCount - ref0.childPreview.length}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
