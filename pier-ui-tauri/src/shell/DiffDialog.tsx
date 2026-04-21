import {
  AlignJustify,
  ArrowDown,
  ArrowUp,
  Columns,
  FileText,
  X,
} from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import IconButton from "../components/IconButton";
import { useI18n } from "../i18n/useI18n";
import {
  pairHunkLines,
  parseUnifiedDiff,
  type DiffHunk,
  type DiffLine,
  type ParsedDiff,
} from "../lib/diffParse";

export type DiffFileInput = {
  /** Stable key (usually the path). */
  id: string;
  /** Path to show in the sidebar and header. */
  path: string;
  /** Git status letter: "M" | "A" | "D" | "R" | "U" | etc. */
  status: "modified" | "added" | "deleted" | "renamed" | "untracked";
  /** Raw unified diff text, or null if not yet loaded. */
  diffText: string | null;
  /** Optional precomputed totals (if absent, computed from diffText). */
  additions?: number;
  deletions?: number;
};

type Props = {
  open: boolean;
  onClose: () => void;
  files: DiffFileInput[];
  activeId?: string;
  onSelectFile?: (id: string) => void;
  /** Shown as footer hint; caller owns keyboard handling. */
  footer?: ReactNode;
  /** Optional action buttons on the dialog footer. */
  actions?: ReactNode;
};

function statusLetter(status: DiffFileInput["status"]): string {
  switch (status) {
    case "added": return "A";
    case "deleted": return "D";
    case "renamed": return "R";
    case "untracked": return "U";
    default: return "M";
  }
}

export default function DiffDialog({ open, onClose, files, activeId, onSelectFile, footer, actions }: Props) {
  const { t } = useI18n();
  const [mode, setMode] = useState<"unified" | "split">("split");
  const [wrap, setWrap] = useState(false);
  const [ignoreWS, setIgnoreWS] = useState(false);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const selected = useMemo(() => {
    if (!files.length) return null;
    return files.find((f) => f.id === activeId) ?? files[0];
  }, [files, activeId]);

  const parsed = useMemo<ParsedDiff | null>(() => {
    if (!selected?.diffText) return null;
    return parseUnifiedDiff(selected.diffText);
  }, [selected?.diffText]);

  if (!open) return null;

  const addTotal = selected?.additions ?? parsed?.additions ?? 0;
  const delTotal = selected?.deletions ?? parsed?.deletions ?? 0;

  return (
    <div className="cmdp-overlay" onClick={onClose}>
      <div className="dlg dlg--diff" onClick={(e) => e.stopPropagation()}>
        <div className="dialog__header dialog__header--diff">
          <span className="dlg-title">
            <FileText size={13} />
            <span className="mono dlg-title__path">{selected?.path ?? ""}</span>
            {selected ? <span className={"dlg-diff-status s-" + selected.status}>{selected.status}</span> : null}
            {addTotal > 0 ? <span className="mono dlg-diff-count dlg-diff-count--add">+{addTotal}</span> : null}
            {delTotal > 0 ? <span className="mono dlg-diff-count dlg-diff-count--del">−{delTotal}</span> : null}
          </span>

          <div className="dlg-diff-toolbar">
            <div className="dlg-seg">
              <button type="button" className={"dlg-seg-btn" + (mode === "unified" ? " on" : "")} onClick={() => setMode("unified")}>
                <AlignJustify size={11} /> {t("Unified")}
              </button>
              <button type="button" className={"dlg-seg-btn" + (mode === "split" ? " on" : "")} onClick={() => setMode("split")}>
                <Columns size={11} /> {t("Split")}
              </button>
            </div>
            <button
              type="button"
              className={"dlg-ic-btn" + (wrap ? " on" : "")}
              title={t("Wrap lines")}
              onClick={() => setWrap((v) => !v)}
            >
              <AlignJustify size={11} />
            </button>
            <button
              type="button"
              className={"dlg-ic-btn" + (ignoreWS ? " on" : "")}
              title={t("Ignore whitespace")}
              onClick={() => setIgnoreWS((v) => !v)}
            >
              <span className="mono dlg-ic-btn__label">WS</span>
            </button>
            <button type="button" className="dlg-ic-btn" title={t("Previous change")}>
              <ArrowUp size={11} />
            </button>
            <button type="button" className="dlg-ic-btn" title={t("Next change")}>
              <ArrowDown size={11} />
            </button>
          </div>

          <IconButton variant="mini" onClick={onClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        <div className="dialog__body dialog__body--diff">
          {files.length > 1 && (
            <div className="dlg-diff-files">
              <div className="dlg-diff-files-head mono">
                <span>{t("Files")}</span>
                <span className="dlg-diff-files-meta">{files.length} {t("changed")}</span>
              </div>
              {files.map((df) => {
                const segs = df.path.split("/");
                const name = segs[segs.length - 1];
                const dir = segs.length > 1 ? segs.slice(0, -1).join("/") + "/" : "";
                return (
                  <div
                    key={df.id}
                    className={"dlg-diff-file" + (df.id === selected?.id ? " sel" : "")}
                    onClick={() => onSelectFile?.(df.id)}
                  >
                    <span className={"dlg-diff-stat s-" + df.status}>{statusLetter(df.status)}</span>
                    <div className="dlg-diff-file-main">
                      <div className="dlg-diff-filename mono">{name}</div>
                      {dir ? <div className="dlg-diff-filepath mono">{dir}</div> : null}
                    </div>
                    <span className="dlg-diff-delta mono">
                      {df.additions && df.additions > 0 ? <span className="dlg-diff-count--add">+{df.additions}</span> : null}
                      {df.deletions && df.deletions > 0 ? <span className="dlg-diff-count--del">−{df.deletions}</span> : null}
                    </span>
                  </div>
                );
              })}
            </div>
          )}

          <div className="dlg-diff-pane">
            {!selected ? (
              <div className="dlg-diff-empty mono">
                <FileText size={18} />
                <div>{t("No file selected")}</div>
              </div>
            ) : selected.diffText === null ? (
              <div className="dlg-diff-empty mono">
                <FileText size={18} />
                <div>{t("Loading diff…")}</div>
              </div>
            ) : !parsed || parsed.hunks.length === 0 ? (
              <div className="dlg-diff-empty mono">
                <FileText size={18} />
                <div>
                  {selected.status === "added"
                    ? t("New file")
                    : selected.status === "deleted"
                      ? t("Deleted file")
                      : t("No diff output")}
                </div>
              </div>
            ) : mode === "split" ? (
              <DiffSplit hunks={parsed.hunks} wrap={wrap} oldPath={parsed.oldPath || selected.path} newPath={parsed.newPath || selected.path} />
            ) : (
              <DiffUnified hunks={parsed.hunks} wrap={wrap} path={selected.path} />
            )}
          </div>
        </div>

        <div className="dialog__footer dialog__footer--diff">
          <span className="dlg-foot-hint mono">
            {footer ?? (
              <>
                <span className="kbd">Esc</span> {t("close")}
              </>
            )}
          </span>
          <div style={{ flex: 1 }} />
          {actions}
        </div>
      </div>
    </div>
  );
}

function DiffUnified({ hunks, wrap, path }: { hunks: DiffHunk[]; wrap: boolean; path: string }) {
  return (
    <div className={"dlg-diff-scroll mono" + (wrap ? " wrap" : "")}>
      <div className="dlg-diff-hunk-bar mono"><span>{path}</span></div>
      {hunks.map((h, hi) => (
        <Fragment key={hi}>
          <div className="dlg-diff-hunk-head mono">{h.header}</div>
          {h.lines.map((l, i) => (
            <div key={i} className={"dlg-diff-line u-" + l.kind}>
              <span className="dlg-diff-ln">{l.oldLine ?? ""}</span>
              <span className="dlg-diff-ln">{l.newLine ?? ""}</span>
              <span className="dlg-diff-sign">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
              <span className="dlg-diff-code">{l.text}</span>
            </div>
          ))}
        </Fragment>
      ))}
    </div>
  );
}

function DiffSplit({ hunks, wrap, oldPath, newPath }: { hunks: DiffHunk[]; wrap: boolean; oldPath: string; newPath: string }) {
  return (
    <div className={"dlg-diff-scroll mono split" + (wrap ? " wrap" : "")}>
      <div className="dlg-diff-split-head mono">
        <div><span className="dlg-diff-count--del">−</span> {oldPath}</div>
        <div><span className="dlg-diff-count--add">+</span> {newPath}</div>
      </div>
      {hunks.map((h, hi) => {
        const pairs = pairHunkLines(h.lines);
        return (
          <Fragment key={hi}>
            <div className="dlg-diff-hunk-head split mono">
              <span>{h.header}</span>
              <span>{h.header}</span>
            </div>
            {pairs.map((p, i) => (
              <div key={i} className="dlg-diff-split-row">
                <SplitSide side="left" line={p.left} />
                <SplitSide side="right" line={p.right} />
              </div>
            ))}
          </Fragment>
        );
      })}
    </div>
  );
}

function SplitSide({ side, line }: { side: "left" | "right"; line: DiffLine | null }) {
  if (!line) {
    return (
      <div className={"dlg-diff-line s-empty s-" + side}>
        <span className="dlg-diff-ln" />
        <span className="dlg-diff-sign" />
        <span className="dlg-diff-code" />
      </div>
    );
  }
  const ln = side === "left" ? line.oldLine : line.newLine;
  return (
    <div className={"dlg-diff-line s-" + line.kind + " s-" + side}>
      <span className="dlg-diff-ln">{ln ?? ""}</span>
      <span className="dlg-diff-sign">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
      <span className="dlg-diff-code">{line.text}</span>
    </div>
  );
}
