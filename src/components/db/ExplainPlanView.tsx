import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Code2, X } from "lucide-react";

import type { PlanNode } from "../../lib/explainPlan";
import { useI18n } from "../../i18n/useI18n";

type Props = {
  plan: PlanNode;
  /** Optional close button — when set, lets the user dismiss the
   *  plan view to return to the data grid. */
  onClose?: () => void;
  /** Header chip — usually "EXPLAIN ANALYZE · 12 ms" or similar. */
  meta?: string;
  /** When set, each node is annotated with delta chips comparing
   *  actual rows / actual time to the matching node in `prevPlan`
   *  (paired by tree position). Useful for tracking regressions
   *  between two runs of the same query. */
  prevPlan?: PlanNode | null;
};

/**
 * Hierarchical plan tree. Each node shows its operator label, an
 * optional detail line (table / index / condition), and a row of
 * stat chips (rows / cost / actual time / buffers) when those are
 * present in the underlying parsed plan.
 *
 * Children render under their parent with a vertical indent. Heavy
 * branches (≥ 5 children, or a child whose subtree contains another
 * heavy branch) start collapsed so the user isn't drowned by a 30-
 * line plan; everything else is expanded by default.
 *
 * Per-row tint is driven by `actualTime.total / sumActualTotal` —
 * the hotter a node's share of total runtime, the more it leans into
 * the accent. Reads at a glance "this branch is the bottleneck".
 */
export default function ExplainPlanView({
  plan,
  onClose,
  meta,
  prevPlan,
}: Props) {
  const { t } = useI18n();
  const totalActual = useMemo(() => sumActualTotal(plan), [plan]);
  const totalCost = plan.cost?.total;

  return (
    <div className="explain-plan">
      <div className="explain-plan__head mono">
        <span className="explain-plan__title">{t("Plan")}</span>
        {meta && <span className="explain-plan__meta">{meta}</span>}
        {totalCost !== undefined && (
          <span className="explain-plan__chip" title={t("Estimated total cost")}>
            cost {fmtNum(totalCost)}
          </span>
        )}
        {totalActual !== undefined && (
          <span
            className="explain-plan__chip explain-plan__chip--accent"
            title={t("Actual total time, ms")}
          >
            {fmtNum(totalActual)} ms
          </span>
        )}
        {prevPlan && (
          <span
            className="explain-plan__chip"
            title={t("Comparing against previous run")}
          >
            {t("vs prev")}
          </span>
        )}
        <span className="explain-plan__spacer" />
        {onClose && (
          <button
            type="button"
            className="btn is-ghost is-compact"
            onClick={onClose}
            title={t("Close plan view")}
          >
            <X size={10} /> {t("Close")}
          </button>
        )}
      </div>
      <div className="explain-plan__body">
        <PlanRow
          node={plan}
          prev={prevPlan ?? null}
          depth={0}
          totalActual={totalActual}
        />
      </div>
    </div>
  );
}

function PlanRow({
  node,
  prev,
  depth,
  totalActual,
}: {
  node: PlanNode;
  prev: PlanNode | null;
  depth: number;
  totalActual: number | undefined;
}) {
  const { t } = useI18n();
  const heavy = isHeavyBranch(node);
  const [open, setOpen] = useState(!heavy || depth === 0);
  const [showRaw, setShowRaw] = useState(false);
  const hasChildren = node.children.length > 0;
  const indent = depth * 16;

  // Heat tier: 0 (cool) .. 4 (hot). Anchored on the node's actual
  // time as a fraction of the whole plan's actual time. Cool tier
  // means no/negligible time, so we suppress the tint entirely.
  const heatTier = computeHeatTier(node.actualTime?.total, totalActual);

  // Delta annotations vs the matching node in `prev`. Matching is by
  // tree position alone, so a structural change between runs (extra
  // child, reordered branches) will skip the comparison gracefully.
  const rowsDelta =
    prev?.actualRows !== undefined && node.actualRows !== undefined
      ? node.actualRows - prev.actualRows
      : undefined;
  const timeDelta =
    prev?.actualTime?.total !== undefined &&
    node.actualTime?.total !== undefined
      ? node.actualTime.total - prev.actualTime.total
      : undefined;

  return (
    <>
      <div
        className={`explain-plan__row mono explain-plan__row--heat-${heatTier}`}
        style={{ paddingLeft: indent }}
      >
        <button
          type="button"
          className="explain-plan__chev"
          disabled={!hasChildren}
          onClick={() => setOpen((v) => !v)}
          aria-label={open ? "collapse" : "expand"}
        >
          {hasChildren ? (
            open ? (
              <ChevronDown size={10} />
            ) : (
              <ChevronRight size={10} />
            )
          ) : (
            <span className="explain-plan__chev-spacer" />
          )}
        </button>
        <div className="explain-plan__node">
          <div className="explain-plan__label-row">
            <div className="explain-plan__label">{node.label}</div>
            {node.raw !== undefined && (
              <button
                type="button"
                className="explain-plan__raw-toggle"
                onClick={() => setShowRaw((v) => !v)}
                title={
                  showRaw ? t("Hide raw JSON") : t("Show raw JSON for this node")
                }
                aria-pressed={showRaw}
              >
                <Code2 size={10} />
              </button>
            )}
          </div>
          {node.detail && (
            <div className="explain-plan__detail">{node.detail}</div>
          )}
          <div className="explain-plan__chips">
            {node.actualRows !== undefined && (
              <Chip
                kind="accent"
                title="Actual rows"
                label={`rows ${fmtNum(node.actualRows)}`}
              />
            )}
            {node.actualRows === undefined && node.rows !== undefined && (
              <Chip title="Estimated rows" label={`rows ${fmtNum(node.rows)}`} />
            )}
            {node.actualTime !== undefined && (
              <Chip
                kind="accent"
                title="Actual time, ms"
                label={`${fmtNum(node.actualTime.total)} ms`}
              />
            )}
            {node.cost !== undefined && (
              <Chip
                title="Estimated cost"
                label={
                  node.cost.startup !== undefined
                    ? `cost ${fmtNum(node.cost.startup)}…${fmtNum(node.cost.total)}`
                    : `cost ${fmtNum(node.cost.total)}`
                }
              />
            )}
            {node.buffers && (
              <Chip title="Buffers" label={node.buffers} />
            )}
            {rowsDelta !== undefined && rowsDelta !== 0 && (
              <Chip
                kind={rowsDelta > 0 ? "delta-bad" : "delta-good"}
                title={t("Rows change vs previous run")}
                label={`${rowsDelta > 0 ? "+" : ""}${fmtNum(rowsDelta)} rows`}
              />
            )}
            {timeDelta !== undefined && Math.abs(timeDelta) >= 0.01 && (
              <Chip
                kind={timeDelta > 0 ? "delta-bad" : "delta-good"}
                title={t("Time change vs previous run")}
                label={`${timeDelta > 0 ? "+" : ""}${fmtNum(timeDelta)} ms`}
              />
            )}
          </div>
          {showRaw && node.raw !== undefined && (
            <pre className="explain-plan__raw mono">
              {JSON.stringify(node.raw, null, 2)}
            </pre>
          )}
        </div>
      </div>
      {open &&
        node.children.map((child, i) => (
          <PlanRow
            key={i}
            node={child}
            prev={prev?.children?.[i] ?? null}
            depth={depth + 1}
            totalActual={totalActual}
          />
        ))}
    </>
  );
}

function Chip({
  label,
  title,
  kind,
}: {
  label: string;
  title?: string;
  kind?: "accent" | "delta-good" | "delta-bad";
}) {
  const cls =
    "explain-plan__chip" +
    (kind === "accent"
      ? " explain-plan__chip--accent"
      : kind === "delta-good"
        ? " explain-plan__chip--delta-good"
        : kind === "delta-bad"
          ? " explain-plan__chip--delta-bad"
          : "");
  return (
    <span className={cls} title={title}>
      {label}
    </span>
  );
}

function fmtNum(n: number): string {
  if (!Number.isFinite(n)) return "?";
  if (Math.abs(n) >= 1000) return Math.round(n).toLocaleString("en-US");
  if (Math.abs(n) >= 10) return n.toFixed(1);
  return n.toFixed(2);
}

function sumActualTotal(node: PlanNode): number | undefined {
  return node.actualTime?.total;
}

function computeHeatTier(
  nodeTime: number | undefined,
  totalTime: number | undefined,
): 0 | 1 | 2 | 3 | 4 {
  if (
    nodeTime === undefined ||
    totalTime === undefined ||
    totalTime <= 0 ||
    nodeTime <= 0
  ) {
    return 0;
  }
  const frac = nodeTime / totalTime;
  if (frac >= 0.6) return 4;
  if (frac >= 0.35) return 3;
  if (frac >= 0.15) return 2;
  if (frac >= 0.05) return 1;
  return 0;
}

function isHeavyBranch(node: PlanNode): boolean {
  if (node.children.length >= 5) return true;
  let depth = 0;
  let cur = node;
  while (cur.children.length === 1) {
    depth += 1;
    cur = cur.children[0];
    if (depth > 6) return true;
  }
  return false;
}
