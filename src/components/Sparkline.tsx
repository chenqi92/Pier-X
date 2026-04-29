// Tiny inline sparkline used by the Hosts Bus latency column. Pure
// SVG, fits in ~80×16, no external charting dep. Each input is a
// non-negative number (latency ms in our case, but the component
// itself is unit-agnostic).
//
// Design choices:
//   - Auto-scale Y to the visible window so a flat 5ms host doesn't
//     flatten next to a 200ms one — each spark is its own context.
//   - Mark the latest point with a small dot so the user can pick out
//     the current sample at a glance vs. the trail.
//   - Use `--accent` for the line and `--neg` for samples that came
//     back as `null` (rendered as a downward spike to 0).

type Props = {
  /** Latency samples, oldest-first. Pass `null` for failed probes
   *  so they show as gaps rather than fake-zero data points. */
  values: (number | null)[];
  /** Width in CSS pixels. Defaults to 60px which fits nicely in a
   *  compact monospace table cell next to the "12 ms" label. */
  width?: number;
  /** Height in CSS pixels. */
  height?: number;
  /** Tooltip / aria-label override; default summarises last value. */
  title?: string;
};

export default function Sparkline({
  values,
  width = 60,
  height = 16,
  title,
}: Props) {
  if (values.length < 2) {
    // Single sample (or none) — nothing meaningful to plot.
    return null;
  }
  const numeric = values.filter((v): v is number => v != null && Number.isFinite(v));
  if (numeric.length < 2) return null;
  const min = Math.min(...numeric);
  const max = Math.max(...numeric);
  const range = Math.max(1, max - min);
  const stepX = values.length === 1 ? 0 : width / (values.length - 1);

  // Walk the points; null samples break the polyline so failures
  // render as actual gaps instead of synthetic zeros.
  const segments: string[] = [];
  let cur: string[] = [];
  values.forEach((v, i) => {
    if (v == null || !Number.isFinite(v)) {
      if (cur.length >= 2) segments.push(cur.join(" "));
      cur = [];
      return;
    }
    const x = i * stepX;
    const y = height - ((v - min) / range) * (height - 2) - 1;
    cur.push(`${x.toFixed(1)},${y.toFixed(1)}`);
  });
  if (cur.length >= 2) segments.push(cur.join(" "));

  const lastVal = numeric[numeric.length - 1];
  const lastIndex = (() => {
    for (let i = values.length - 1; i >= 0; i--) {
      if (values[i] != null) return i;
    }
    return -1;
  })();
  const lastX = lastIndex * stepX;
  const lastY = height - ((lastVal - min) / range) * (height - 2) - 1;
  const failed = values.filter((v) => v == null).length;

  const aria =
    title ??
    `last ${numeric.length} samples · ${Math.round(min)}–${Math.round(
      max,
    )} ms${failed > 0 ? ` · ${failed} failed` : ""}`;

  return (
    <svg
      className="sparkline"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={aria}
    >
      {segments.map((pts, i) => (
        <polyline
          key={i}
          points={pts}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.2"
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      ))}
      <circle cx={lastX} cy={lastY} r={1.5} fill="currentColor" />
    </svg>
  );
}
