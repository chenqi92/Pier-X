import { Check, Lock, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { NginxNode } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import {
  collectScopes,
  FEATURES,
  GROUP_ORDER,
  GROUP_TITLES,
  getBlockAt,
  replaceBlockAt,
  type Feature,
  type FieldValue,
  type FieldValues,
  type Scope,
  type ScopeKind,
} from "./nginxFeatures";

const SCOPE_LABELS: Record<ScopeKind, string> = {
  main: "Top-level",
  http: "http",
  server: "server",
};

export default function FeatureCatalog({
  nodes,
  onChange,
}: {
  nodes: NginxNode[];
  onChange: (nodes: NginxNode[]) => void;
}) {
  const { t } = useI18n();
  const scopes = useMemo(() => collectScopes(nodes), [nodes]);
  // Prefer the most specific scope present: server → http → main.
  const defaultScopeIdx = useMemo(() => {
    const i = scopes.findIndex((s) => s.kind === "server");
    if (i >= 0) return i;
    const j = scopes.findIndex((s) => s.kind === "http");
    if (j >= 0) return j;
    return 0;
  }, [scopes]);
  const [scopeIdx, setScopeIdx] = useState(defaultScopeIdx);
  const lastDefaultRef = useRef(defaultScopeIdx);
  useEffect(() => {
    // Re-pick a sensible default when the AST shape changes (e.g. file
    // reload introduces a new server block that wasn't there before).
    if (defaultScopeIdx !== lastDefaultRef.current) {
      setScopeIdx(defaultScopeIdx);
      lastDefaultRef.current = defaultScopeIdx;
    }
  }, [defaultScopeIdx]);

  const safeIdx = scopeIdx < scopes.length ? scopeIdx : 0;
  const scope = scopes[safeIdx] ?? scopes[0];
  const block = scope ? getBlockAt(nodes, scope.path) ?? [] : [];

  const apply = (newBlock: NginxNode[]) => {
    if (!scope) return;
    onChange(replaceBlockAt(nodes, scope.path, newBlock));
  };

  const visibleFeatures = FEATURES.filter((f) =>
    scope ? f.contexts.includes(scope.kind) : false,
  );

  return (
    <div className="ngx-fc">
      <ScopeBar
        scopes={scopes}
        activeIdx={safeIdx}
        onPick={setScopeIdx}
      />
      {!scope && (
        <div className="status-note mono">
          {t("(no editable scope in this file)")}
        </div>
      )}
      {scope &&
        GROUP_ORDER.map((g) => {
          const items = visibleFeatures.filter((f) => f.group === g);
          if (items.length === 0) return null;
          return (
            <div key={g} className="ngx-fc__group">
              <div className="ngx-fc__group-title mono">
                {t(GROUP_TITLES[g])}
              </div>
              <div className="ngx-fc__grid">
                {items.map((f) => (
                  <FeatureCard
                    key={f.id}
                    feature={f}
                    block={block}
                    scopeKind={scope.kind}
                    onApply={apply}
                  />
                ))}
              </div>
            </div>
          );
        })}
      {scope && visibleFeatures.length === 0 && (
        <div className="status-note mono">
          {t("(no features apply to this scope)")}
        </div>
      )}
    </div>
  );
}

function ScopeBar({
  scopes,
  activeIdx,
  onPick,
}: {
  scopes: Scope[];
  activeIdx: number;
  onPick: (idx: number) => void;
}) {
  const { t } = useI18n();
  if (scopes.length <= 1) return null;
  return (
    <div className="ngx-fc__scopes" role="tablist">
      <span className="ngx-fc__scopes-label mono">{t("Scope")}</span>
      {scopes.map((s, i) => (
        <button
          key={i}
          type="button"
          role="tab"
          aria-selected={i === activeIdx}
          className={`ngx-fc__scope ${i === activeIdx ? "is-active" : ""}`}
          onClick={() => onPick(i)}
          title={s.label}
        >
          <span className="ngx-fc__scope-kind mono">
            {t(SCOPE_LABELS[s.kind])}
          </span>
          {s.kind === "server" && (
            <span className="ngx-fc__scope-detail mono">
              {scopeDetail(s.label)}
            </span>
          )}
        </button>
      ))}
    </div>
  );
}

function scopeDetail(label: string): string {
  // Strip the "server { ... }" wrapper for a tighter tab pill.
  const m = /^server\s*\{\s*(.+?)\s*\}$/.exec(label);
  return m ? m[1] : label;
}

function FeatureCard({
  feature,
  block,
  scopeKind,
  onApply,
}: {
  feature: Feature;
  block: NginxNode[];
  scopeKind: ScopeKind;
  onApply: (block: NginxNode[]) => void;
}) {
  const { t } = useI18n();
  const enabled = feature.detect(block);
  const gate = feature.requires?.(block, scopeKind) ?? null;
  const locked = !enabled && !!gate;

  const astValues = useMemo(
    () => (enabled ? feature.read(block) : feature.defaults),
    [enabled, block, feature],
  );
  const astKey = useMemo(() => JSON.stringify(astValues), [astValues]);
  const [draft, setDraft] = useState<FieldValues>(astValues);
  const lastKeyRef = useRef(astKey);
  useEffect(() => {
    if (astKey !== lastKeyRef.current) {
      setDraft(astValues);
      lastKeyRef.current = astKey;
    }
  }, [astKey, astValues]);

  const toggle = () => {
    if (locked) return;
    if (enabled) onApply(feature.disable(block));
    else onApply(feature.enable(block, draft));
  };

  const commit = (key: string, value: FieldValue) => {
    const next = { ...draft, [key]: value };
    setDraft(next);
    if (enabled) onApply(feature.enable(block, next));
  };

  const cls = [
    "ngx-fc-card",
    enabled ? "is-on" : "",
    locked ? "is-locked" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={cls}>
      <button
        type="button"
        className="ngx-fc-card__head"
        onClick={toggle}
        disabled={locked}
        title={
          locked
            ? t(gate!)
            : enabled
              ? t("Click to disable")
              : t("Click to enable")
        }
      >
        <span className={`ngx-fc-card__check ${enabled ? "is-on" : ""}`}>
          {enabled ? (
            <Check size={11} />
          ) : locked ? (
            <Lock size={10} />
          ) : (
            <Plus size={11} />
          )}
        </span>
        <span className="ngx-fc-card__title">{t(feature.title)}</span>
      </button>
      <div className="ngx-fc-card__desc">{t(feature.description)}</div>
      {gate && <div className="ngx-fc-card__gate mono">{t(gate)}</div>}
      {enabled && feature.fields.length > 0 && (
        <div className="ngx-fc-card__fields">
          {feature.fields.map((f) => {
            if (f.type === "text") {
              return (
                <label
                  key={f.key}
                  className="ngx-form__field ngx-form__field--grow"
                >
                  <span className="ngx-form__label">{t(f.label)}</span>
                  <input
                    className="ngx-input mono"
                    value={String(draft[f.key] ?? "")}
                    spellCheck={false}
                    placeholder={f.placeholder}
                    onChange={(e) =>
                      setDraft({ ...draft, [f.key]: e.target.value })
                    }
                    onBlur={(e) => commit(f.key, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter")
                        (e.target as HTMLInputElement).blur();
                    }}
                  />
                </label>
              );
            }
            if (f.type === "textarea") {
              return (
                <label
                  key={f.key}
                  className="ngx-form__field ngx-form__field--grow"
                >
                  <span className="ngx-form__label">{t(f.label)}</span>
                  <textarea
                    className="ngx-input mono ngx-textarea"
                    value={String(draft[f.key] ?? "")}
                    spellCheck={false}
                    placeholder={f.placeholder}
                    rows={f.rows ?? 3}
                    onChange={(e) =>
                      setDraft({ ...draft, [f.key]: e.target.value })
                    }
                    onBlur={(e) => commit(f.key, e.target.value)}
                  />
                </label>
              );
            }
            if (f.type === "select") {
              return (
                <label
                  key={f.key}
                  className="ngx-form__field ngx-form__field--grow"
                >
                  <span className="ngx-form__label">{t(f.label)}</span>
                  <select
                    className="ngx-input mono"
                    value={String(draft[f.key] ?? "")}
                    onChange={(e) => commit(f.key, e.target.value)}
                  >
                    {f.options.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </label>
              );
            }
            return (
              <label key={f.key} className="ngx-form__flag">
                <input
                  type="checkbox"
                  checked={!!draft[f.key]}
                  onChange={(e) => commit(f.key, e.target.checked)}
                />
                {t(f.label)}
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}
