import { useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileText, Hash } from "lucide-react";
import * as cmd from "../lib/commands";
import type { ApacheNode, ApacheParseResult } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";

// Read-only structured tree view for an Apache config. Renders
// `<Section>` containers as collapsible cards and inline directives
// as leaf rows. Re-parses on every content change because the parser
// is fast and the round-trip keeps the tree perfectly in sync with
// the dirty buffer.

type Props = {
  /** Current text contents of the active Apache config file. */
  content: string;
};

export default function ApacheTreeView({ content }: Props) {
  const { t } = useI18n();
  const [parse, setParse] = useState<ApacheParseResult | null>(null);
  const [parseError, setParseError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setParseError("");
    cmd
      .apacheParse(content)
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
        <div className="status-note mono">{t("Parsing Apache config…")}</div>
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
        <div className="status-note mono">{t("(empty config)")}</div>
      )}
      <div className="ws-tree__nodes">
        {parse.nodes.map((n, i) => (
          <NodeView key={i} node={n} depth={0} t={t} />
        ))}
      </div>
    </div>
  );
}

function NodeView({
  node,
  depth,
  t,
}: {
  node: ApacheNode;
  depth: number;
  t: (s: string) => string;
}) {
  if (node.kind === "comment") {
    return (
      <div className="ws-tree-card ws-tree-card--comment">
        <div className="ws-tree-card__head mono">
          <Hash size={10} /> {node.text}
        </div>
      </div>
    );
  }
  return <DirectiveCard directive={node} depth={depth} t={t} />;
}

function DirectiveCard({
  directive,
  depth,
  t,
}: {
  directive: Extract<ApacheNode, { kind: "directive" }>;
  depth: number;
  t: (s: string) => string;
}) {
  const isSection = directive.section !== null;
  // Top-level sections (vhosts, IfModule wrappers) default open;
  // deeper nested sections (Directory, Location inside vhost) default
  // collapsed at depth ≥ 2.
  const [expanded, setExpanded] = useState(depth < 2);

  const title = useMemo(() => {
    if (isSection) return `<${directive.name}>`;
    return directive.name;
  }, [directive.name, isSection]);

  const argSummary =
    directive.args.length > 0 ? directive.args.join(" ") : "";

  return (
    <div
      className={`ws-tree-card ${isSection ? "ws-tree-card--block" : ""} ${
        expanded ? "is-expanded" : ""
      }`}
    >
      <button
        type="button"
        className="ws-tree-card__head"
        onClick={() => isSection && setExpanded(!expanded)}
      >
        <span className="ws-tree-card__chevron">
          {isSection ? (
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
      {isSection && expanded && (
        <div className="ws-tree-card__body">
          {directive.section!.length === 0 && (
            <div className="status-note mono">{t("(empty section)")}</div>
          )}
          {directive.section!.map((child, i) => (
            <NodeView key={i} node={child} depth={depth + 1} t={t} />
          ))}
        </div>
      )}
    </div>
  );
}
