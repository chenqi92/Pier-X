import { useEffect, useState } from "react";

type Variant = "rows" | "grid" | "chrome" | "prose" | "splash";

/* Defer the heavy panel body to the next animation frame so the parent's
 * skeleton paints first. Use to wrap a panel whose mount cost (large
 * component tree, many useMemo, eager IPC in useEffect) makes the click
 * feel laggy. */
export function useDeferredMount(): boolean {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const handle = requestAnimationFrame(() => setReady(true));
    return () => cancelAnimationFrame(handle);
  }, []);
  return ready;
}

type Props = {
  variant?: Variant;
  rows?: number;
  className?: string;
};

export default function PanelSkeleton({
  variant = "rows",
  rows = 8,
  className,
}: Props) {
  const cls = `panel-sk panel-sk--${variant}${className ? ` ${className}` : ""}`;
  return (
    <div className={cls} aria-hidden="true">
      {variant === "rows" && (
        <>
          {Array.from({ length: rows }, (_, i) => {
            const nameWidth = 50 + ((i * 17) % 45);
            return (
              <div key={i} className="panel-sk-row">
                <span className="panel-sk-bar is-icon" />
                <span className="panel-sk-bar is-name" style={{ width: `${nameWidth}%` }} />
                <span className="panel-sk-bar is-meta" />
              </div>
            );
          })}
        </>
      )}
      {variant === "grid" && (
        <>
          <div className="panel-sk-head">
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
          </div>
          {Array.from({ length: rows }, (_, i) => (
            <div key={i} className="panel-sk-row">
              <span className="panel-sk-bar" />
              <span className="panel-sk-bar" />
              <span className="panel-sk-bar" />
              <span className="panel-sk-bar" />
            </div>
          ))}
        </>
      )}
      {variant === "chrome" && (
        <>
          <div className="panel-sk-host">
            <span className="panel-sk-bar is-title" />
            <span className="panel-sk-bar is-sub" />
          </div>
          <div className="panel-sk-gauges">
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
            <span className="panel-sk-bar" />
          </div>
        </>
      )}
      {variant === "prose" && (
        <>
          <span className="panel-sk-bar is-title" />
          {Array.from({ length: rows }, (_, i) => (
            <span
              key={i}
              className={`panel-sk-bar ${i % 4 === 3 ? "is-short" : "is-line"}`}
            />
          ))}
        </>
      )}
      {variant === "splash" && (
        <>
          <div className="panel-sk-head">
            <span className="panel-sk-bar is-title" />
            <span className="panel-sk-bar is-sub" />
          </div>
          {Array.from({ length: Math.max(2, Math.min(rows, 4)) }, (_, i) => (
            <div key={i} className="panel-sk-card" />
          ))}
        </>
      )}
    </div>
  );
}
