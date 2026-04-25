import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlignJustify,
  ArrowDown,
  ChevronDown,
  ChevronUp,
  Clock,
  Download,
  Pause,
  Play,
  Plus,
  Search,
  X,
} from "lucide-react";
import { useDraggableDialog } from "../components/useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { describeLogSource } from "../lib/logSource";
import type { LogSource } from "../lib/types";

/** Same `Enriched` row shape that LogViewerPanel produces — kept here as a
 *  duplicate type to avoid a circular import; the producer is the parent and
 *  passes the array down. */
export type LogDialogEvent = {
  idx: number;
  kind: string;
  text: string;
  level: "info" | "warn" | "error" | "debug";
  ts: string;
};

type Props = {
  open: boolean;
  onClose: () => void;
  events: LogDialogEvent[];
  source: LogSource;
  hostLabel: string;
  streaming: boolean;
  onToggleStreaming: () => void;
  onClear?: () => void;
  /** Compiled command string from the parent — used to show the path/cmd
   *  in the title bar. */
  compiledCommand: string;
};

type Levels = Record<"INFO" | "WARN" | "ERROR" | "DEBUG", boolean>;
type Cols = { n: boolean; t: boolean; lvl: boolean; src: boolean };
type Range = "last1m" | "last15m" | "last1h" | "last24h" | "all";

const LEVELS_ORDER: Array<keyof Levels> = ["INFO", "WARN", "ERROR", "DEBUG"];

export default function LogViewerDialog({
  open,
  onClose,
  events,
  source,
  hostLabel,
  streaming,
  onToggleStreaming,
  onClear,
  compiledCommand,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);

  const [levels, setLevels] = useState<Levels>({
    INFO: true, WARN: true, ERROR: true, DEBUG: false,
  });
  const [search, setSearch] = useState("");
  const [searchRe, setSearchRe] = useState(false);
  const [searchCi, setSearchCi] = useState(true);
  const [sourceFilter, setSourceFilter] = useState("");
  const [activeHit, setActiveHit] = useState(0);
  const [wrap, setWrap] = useState(true);
  const [follow, setFollow] = useState(true);
  const [cols, setCols] = useState<Cols>({ n: true, t: true, lvl: true, src: true });
  const [contextLines, setContextLines] = useState(0);
  const [range, setRange] = useState<Range>("all");
  const [selected, setSelected] = useState<number | null>(null);
  const overlayDownRef = useRef(false);
  const bodyRef = useRef<HTMLDivElement | null>(null);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        const el = document.querySelector<HTMLInputElement>(".lv-search input");
        el?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // Per-level counts across the unfiltered stream
  const counts = useMemo(() => {
    const acc: Record<string, number> = { INFO: 0, WARN: 0, ERROR: 0, DEBUG: 0 };
    for (const e of events) acc[e.level.toUpperCase()] = (acc[e.level.toUpperCase()] || 0) + 1;
    return acc;
  }, [events]);

  // Apply level + source filters
  const filtered = useMemo(() => {
    const sf = sourceFilter.trim().toLowerCase();
    return events.filter((e) => {
      const lvKey = e.level.toUpperCase() as keyof Levels;
      if (!levels[lvKey]) return false;
      if (sf && !e.text.toLowerCase().includes(sf)) return false;
      return true;
    });
  }, [events, levels, sourceFilter]);

  // Search hits — count + per-line offsets, with one shared regex
  const searchRegex = useMemo(() => {
    if (!search) return null;
    try {
      const pat = searchRe ? search : search.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      return new RegExp(pat, "g" + (searchCi ? "i" : ""));
    } catch {
      return null;
    }
  }, [search, searchRe, searchCi]);

  const { hitCount, perLineStart } = useMemo(() => {
    if (!searchRegex) return { hitCount: 0, perLineStart: [] as number[] };
    let total = 0;
    const starts: number[] = [];
    for (const e of filtered) {
      starts.push(total);
      const hay = `${e.ts} ${e.level.toUpperCase()} ${e.text}`;
      let m: RegExpExecArray | null;
      searchRegex.lastIndex = 0;
      while ((m = searchRegex.exec(hay)) !== null) {
        total++;
        if (m[0].length === 0) searchRegex.lastIndex++;
      }
    }
    return { hitCount: total, perLineStart: starts };
  }, [searchRegex, filtered]);

  useEffect(() => {
    if (hitCount === 0) setActiveHit(0);
    else if (activeHit >= hitCount) setActiveHit(0);
  }, [hitCount, activeHit]);

  // Scroll the active hit into view
  useEffect(() => {
    if (!bodyRef.current) return;
    const el = bodyRef.current.querySelector(".lv-hl.active");
    if (el) (el as HTMLElement).scrollIntoView({ block: "center" });
  }, [activeHit, search]);

  // Auto-follow tail when enabled and new events arrive
  useEffect(() => {
    if (!follow || !bodyRef.current) return;
    bodyRef.current.scrollTop = bodyRef.current.scrollHeight;
  }, [filtered.length, follow]);

  const nextHit = () => { if (hitCount) setActiveHit((activeHit + 1) % hitCount); };
  const prevHit = () => { if (hitCount) setActiveHit((activeHit - 1 + hitCount) % hitCount); };

  // Render a message with hit highlights, wired to activeHit so the live
  // cursor animates as the user steps through matches.
  const renderMsg = (text: string, lineIdx: number) => {
    if (!searchRegex) return text;
    const out: Array<string | { t: string; id: number }> = [];
    let last = 0;
    let m: RegExpExecArray | null;
    let id = perLineStart[lineIdx] || 0;
    searchRegex.lastIndex = 0;
    while ((m = searchRegex.exec(text)) !== null) {
      if (m.index > last) out.push(text.slice(last, m.index));
      out.push({ t: m[0], id: id++ });
      last = m.index + m[0].length;
      if (m[0].length === 0) searchRegex.lastIndex++;
    }
    if (last < text.length) out.push(text.slice(last));
    return out.map((p, j) =>
      typeof p === "string" ? <span key={j}>{p}</span>
        : <span key={j} className={"lv-hl" + (p.id === activeHit ? " active" : "")}>{p.t}</span>,
    );
  };

  const RANGES: Array<{ id: Range; label: string }> = [
    { id: "last1m", label: "1m" },
    { id: "last15m", label: "15m" },
    { id: "last1h", label: "1h" },
    { id: "last24h", label: "24h" },
    { id: "all", label: t("all") },
  ];

  if (!open) return null;
  const sourceLabel = describeLogSource(source);

  return (
    <div
      className="dlg-overlay"
      onMouseDown={(e) => { overlayDownRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        if (e.target === e.currentTarget && overlayDownRef.current) onClose();
        overlayDownRef.current = false;
      }}
    >
      <div className="dlg dlg--logviewer" style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <div className="lv-top" {...handleProps}>
          <span className="lv-title">
            <b>{sourceLabel}</b>
            <span className="lv-path mono">{compiledCommand || ""}</span>
          </span>
          <span className="lv-top-spacer" />
          <span className={"lv-status mono " + (streaming ? "on" : "off")}>
            <span className={"lv-dot " + (streaming ? "pos" : "off")} />
            {streaming
              ? t("streaming · {count} lines", { count: events.length })
              : t("paused")}
          </span>
          <button type="button" className="mini-button mini-button--ghost" onClick={onClose} title={t("Pop in (Esc)")}>
            <X size={11} />
          </button>
        </div>

        <div className="lv-split">
          <div className="lv-rail">
            <div className="lv-rail-head">
              <span className="lv-rail-t">{t("SOURCES")}</span>
              <button type="button" className="mini-button mini-button--ghost" disabled title={t("Add log source")}>
                <Plus size={10} />
              </button>
            </div>
            <div className="lv-rail-list">
              <div className="lv-src sel">
                <span className={"lv-dot " + (streaming ? "pos" : "off")} />
                <div className="lv-src-body">
                  <div className="lv-src-name">{sourceLabel}</div>
                  <div className="lv-src-path mono" title={compiledCommand}>{compiledCommand || "—"}</div>
                  <div className="lv-src-meta mono">
                    <span>{events.length} {t("lines")}</span>
                    <span className="sep">·</span>
                    <span>{streaming ? t("live") : t("idle")}</span>
                  </div>
                </div>
              </div>
            </div>

            <div className="lv-rail-head">
              <span className="lv-rail-t">{t("TIME RANGE")}</span>
            </div>
            <div className="lv-rail-range">
              {RANGES.map((r) => (
                <button
                  key={r.id}
                  type="button"
                  className={"lv-rng" + (r.id === range ? " on" : "")}
                  onClick={() => setRange(r.id)}
                >
                  {r.label}
                </button>
              ))}
              <button type="button" className="lv-rng" title={t("Custom range")} disabled>
                <Clock size={10} />
              </button>
            </div>

            <div className="lv-rail-head">
              <span className="lv-rail-t">{t("COLUMNS")}</span>
            </div>
            <div className="lv-rail-cols">
              {([
                { k: "n", l: t("line #") },
                { k: "t", l: t("timestamp") },
                { k: "lvl", l: t("level") },
                { k: "src", l: t("source") },
              ] as const).map((c) => (
                <label key={c.k} className="lv-col">
                  <input
                    type="checkbox"
                    checked={cols[c.k]}
                    onChange={(e) => setCols({ ...cols, [c.k]: e.currentTarget.checked })}
                  />
                  {c.l}
                </label>
              ))}
            </div>

            <div className="lv-rail-head">
              <span className="lv-rail-t">{t("CONTEXT")}</span>
            </div>
            <div className="lv-rail-ctx">
              {[0, 1, 3, 5].map((n) => (
                <button
                  key={n}
                  type="button"
                  className={"lv-rng" + (contextLines === n ? " on" : "")}
                  onClick={() => setContextLines(n)}
                  disabled
                  title={t("Context preview only — coming soon")}
                >
                  {n === 0 ? t("off") : `±${n}`}
                </button>
              ))}
            </div>
          </div>

          <div className="lv-main">
            <div className="lv-toolbar">
              <div className="lg-levels">
                {LEVELS_ORDER.map((lv) => (
                  <button
                    key={lv}
                    type="button"
                    className={"lg-chip lv-" + lv.toLowerCase() + (levels[lv] ? " on" : "")}
                    onClick={() => setLevels({ ...levels, [lv]: !levels[lv] })}
                  >
                    <span className="lg-chip-dot" />
                    {lv}
                    <span className="lg-chip-n">{counts[lv] || 0}</span>
                  </button>
                ))}
              </div>

              <div className="lv-search">
                <Search size={11} />
                <input
                  placeholder={t("search — regex, fields, messages…")}
                  value={search}
                  onChange={(e) => setSearch(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      if (e.shiftKey) prevHit();
                      else nextHit();
                    }
                  }}
                />
                <button
                  type="button"
                  className={"editor-find-opt mono" + (searchCi ? " on" : "")}
                  title={t("Case insensitive")}
                  onClick={() => setSearchCi((v) => !v)}
                >
                  Aa
                </button>
                <button
                  type="button"
                  className={"editor-find-opt mono" + (searchRe ? " on" : "")}
                  title={t("Regex")}
                  onClick={() => setSearchRe((v) => !v)}
                >
                  .*
                </button>
                <span className="lv-hits mono">
                  {hitCount === 0 ? (search ? t("no matches") : "")
                    : `${activeHit + 1}/${hitCount}`}
                </span>
                <button type="button" className="editor-tool-btn" title={t("Previous (⇧⏎)")} onClick={prevHit} disabled={!hitCount}>
                  <ChevronUp size={11} />
                </button>
                <button type="button" className="editor-tool-btn" title={t("Next (⏎)")} onClick={nextHit} disabled={!hitCount}>
                  <ChevronDown size={11} />
                </button>
              </div>

              <input
                className="lv-source-filter mono"
                placeholder={t("source filter…")}
                value={sourceFilter}
                onChange={(e) => setSourceFilter(e.currentTarget.value)}
              />

              <span className="lv-toolbar-spacer" />

              <button
                type="button"
                className={"editor-tool-btn" + (wrap ? " on" : "")}
                title={t("Wrap lines")}
                onClick={() => setWrap((v) => !v)}
              >
                <AlignJustify size={11} />
              </button>
              <button
                type="button"
                className={"editor-tool-btn" + (follow ? " on" : "")}
                title={t("Follow tail")}
                onClick={() => setFollow((v) => !v)}
              >
                <ArrowDown size={11} />
              </button>
              <button
                type="button"
                className={"editor-tool-btn" + (streaming ? " on" : "")}
                title={streaming ? t("Pause") : t("Resume")}
                onClick={onToggleStreaming}
              >
                {streaming ? <Pause size={11} /> : <Play size={11} />}
              </button>
              <button
                type="button"
                className="editor-tool-btn"
                title={t("Download")}
                onClick={() => downloadEvents(events)}
                disabled={events.length === 0}
              >
                <Download size={11} />
              </button>
            </div>

            <div
              className={"lv-body mono" + (wrap ? " wrap" : "")}
              ref={bodyRef}
              onScroll={(e) => {
                const el = e.currentTarget;
                const atBottom = el.scrollHeight - el.clientHeight - el.scrollTop < 4;
                if (atBottom !== follow) setFollow(atBottom);
              }}
            >
              {filtered.map((e, i) => {
                const sel = selected === e.idx;
                return (
                  <div
                    key={e.idx}
                    className={"lv-line lv-" + e.level + (sel ? " sel" : "")}
                    onClick={() => setSelected(sel ? null : e.idx)}
                  >
                    {cols.n && <span className="lv-n">{String(e.idx).padStart(4, " ")}</span>}
                    {cols.t && <span className="lv-t">{e.ts}</span>}
                    {cols.lvl && <span className={"lv-lvl " + e.level}>{e.level.toUpperCase()}</span>}
                    {cols.src && <span className="lv-src">{sourceLabel}</span>}
                    <span className="lv-msg">{renderMsg(e.text, i)}</span>
                  </div>
                );
              })}
            </div>

            {selected !== null && (() => {
              const e = events.find((x) => x.idx === selected);
              if (!e) return null;
              return (
                <div className="lv-detail">
                  <div className="lv-detail-head">
                    <span className="lv-rail-t">{t("line {n}", { n: e.idx })}</span>
                    <span className="lv-detail-spacer" />
                    <button type="button" className="mini-button mini-button--ghost" onClick={() => setSelected(null)} title={t("Close")}>
                      <X size={10} />
                    </button>
                  </div>
                  <div className="lv-detail-body mono">
                    <span className="lv-detail-k">{t("timestamp")}</span>
                    <span className="lv-detail-v">{e.ts}</span>
                    <span className="lv-detail-k">{t("level")}</span>
                    <span className={"lv-lvl " + e.level + " lv-detail-lvl"}>{e.level.toUpperCase()}</span>
                    <span className="lv-detail-k">{t("source")}</span>
                    <span className="lv-detail-v">{sourceLabel}</span>
                    <span className="lv-detail-k">{t("message")}</span>
                    <span className="lv-detail-v">{e.text}</span>
                    {hostLabel && (
                      <>
                        <span className="lv-detail-k">{t("host")}</span>
                        <span className="lv-detail-v">{hostLabel}</span>
                      </>
                    )}
                  </div>
                </div>
              );
            })()}

            <div className="lv-foot mono">
              <span>
                {t("showing {shown} / {total} lines", { shown: filtered.length, total: events.length })}
              </span>
              <span className="sep">·</span>
              <span>{hitCount ? t("{n} matches", { n: hitCount }) : (search ? t("no matches") : t("no filter"))}</span>
              <span className="lv-toolbar-spacer" />
              {onClear && events.length > 0 && (
                <button type="button" className="mini-button mini-button--ghost" onClick={onClear}>
                  {t("Clear buffer")}
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function downloadEvents(events: LogDialogEvent[]) {
  const text = events
    .map((e) => `${e.ts} [${e.level.toUpperCase()}] ${e.text}`)
    .join("\n");
  const blob = new Blob([text], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `pier-log-${Date.now()}.log`;
  a.click();
  URL.revokeObjectURL(url);
}
