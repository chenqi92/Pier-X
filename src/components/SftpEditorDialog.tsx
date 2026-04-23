import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  AlertTriangle,
  FileText,
  Save,
  Search,
  X,
} from "lucide-react";
import { EditorState, EditorSelection, type Extension } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  drawSelection,
  rectangularSelection,
  crosshairCursor,
  dropCursor,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  search,
  searchKeymap,
  openSearchPanel,
  closeSearchPanel,
  highlightSelectionMatches,
} from "@codemirror/search";
import {
  bracketMatching,
  defaultHighlightStyle,
  foldGutter,
  foldKeymap,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import IconButton from "./IconButton";
import ContextMenu, { type ContextMenuItem } from "./ContextMenu";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import * as cmd from "../lib/commands";
import type { SftpTextFile } from "../lib/commands";
import {
  MAX_EDITOR_BYTES,
  buildEditorPhrases,
  buildEditorTheme,
  languageFromFilename,
  languageLabel,
} from "../lib/sftpEditor";

/** Addressing the editor needs to call `sftp_read_text` /
 *  `sftp_write_text`. Mirrors the spread used by SftpPanel so
 *  parents don't have to reshape. */
export type SftpEditorSshArgs = {
  host: string;
  port: number;
  user: string;
  authMode: string;
  password: string;
  keyPath: string;
  savedConnectionIndex?: number | null;
};

type Props = {
  open: boolean;
  path: string;
  /** Leaf filename — seeds the title bar and language detection. */
  name: string;
  sshArgs: SftpEditorSshArgs;
  onClose: () => void;
  /** Called after a successful save with the persisted byte count. */
  onSaved?: (bytes: number) => void;
};

function basename(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? path : path.slice(i + 1);
}

function useMonoKey(down: (e: KeyboardEvent) => void) {
  useEffect(() => {
    window.addEventListener("keydown", down);
    return () => window.removeEventListener("keydown", down);
  }, [down]);
}

export default function SftpEditorDialog({
  open,
  path,
  name,
  sshArgs,
  onClose,
  onSaved,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const baselineRef = useRef<string>("");
  const saveRef = useRef<() => Promise<void>>(async () => {});

  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [meta, setMeta] = useState<Pick<SftpTextFile, "size" | "permissions" | "modified" | "lossy"> | null>(null);
  const [dirty, setDirty] = useState(false);
  const [cursor, setCursor] = useState<{ line: number; col: number; selLen: number }>({ line: 1, col: 1, selLen: 0 });
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const overlayDownRef = useRef(false);

  const formatError = (e: unknown) => localizeError(e, t);
  const effectiveName = useMemo(() => name || basename(path), [name, path]);
  const phrases = useMemo(() => buildEditorPhrases(t), [t]);

  // Load file content when the dialog opens or path changes.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    setLoading(true);
    setError("");
    setDirty(false);
    setMeta(null);
    void (async () => {
      try {
        const res = await cmd.sftpReadText({
          ...sshArgs,
          path,
          maxBytes: MAX_EDITOR_BYTES,
        });
        if (!alive) return;
        baselineRef.current = res.content;
        setMeta({
          size: res.size,
          permissions: res.permissions,
          modified: res.modified,
          lossy: res.lossy,
        });
        // Defer to next tick so the host div has mounted.
        setTimeout(() => {
          if (!alive) return;
          mountEditor(res.content);
        }, 0);
      } catch (e) {
        if (!alive) return;
        setError(formatError(e));
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
      disposeEditor();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, path]);

  // Dispose on unmount.
  useEffect(() => () => disposeEditor(), []);

  function disposeEditor() {
    if (viewRef.current) {
      viewRef.current.destroy();
      viewRef.current = null;
    }
  }

  function mountEditor(initial: string) {
    disposeEditor();
    const host = hostRef.current;
    if (!host) return;
    const lang = languageFromFilename(effectiveName);
    const extensions: Extension[] = [
      EditorState.phrases.of(phrases),
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightSpecialChars(),
      history(),
      foldGutter(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      indentOnInput(),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      bracketMatching(),
      rectangularSelection(),
      crosshairCursor(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      search({ top: true }),
      keymap.of([
        { key: "Mod-s", preventDefault: true, run: () => { void saveRef.current(); return true; } },
        { key: "Mod-f", preventDefault: true, run: openSearchPanel },
        { key: "Mod-h", preventDefault: true, run: openSearchPanel },
        indentWithTab,
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        ...foldKeymap,
      ]),
      ...buildEditorTheme(),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          const now = u.state.doc.toString();
          setDirty(now !== baselineRef.current);
        }
        if (u.selectionSet || u.docChanged) {
          const sel = u.state.selection.main;
          const line = u.state.doc.lineAt(sel.head);
          setCursor({
            line: line.number,
            col: sel.head - line.from + 1,
            selLen: Math.abs(sel.to - sel.from),
          });
        }
      }),
    ];
    if (lang) extensions.push(lang);

    const state = EditorState.create({ doc: initial, extensions });
    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    // Place cursor at start for consistent initial state.
    view.dispatch({ selection: EditorSelection.single(0) });
    view.focus();
  }

  saveRef.current = async () => {
    const view = viewRef.current;
    if (!view || saving) return;
    const content = view.state.doc.toString();
    setSaving(true);
    setError("");
    try {
      await cmd.sftpWriteText({ ...sshArgs, path, content });
      baselineRef.current = content;
      setDirty(false);
      const bytes = new TextEncoder().encode(content).length;
      setMeta((m) => (m ? { ...m, size: bytes, lossy: false } : m));
      onSaved?.(bytes);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  };

  const requestClose = () => {
    if (dirty) {
      const confirmed = window.confirm(t("Discard unsaved changes?"));
      if (!confirmed) return;
    }
    onClose();
  };

  const isSearchOpen = () => {
    const v = viewRef.current;
    return !!v && !!v.dom.querySelector(".cm-panel.cm-search");
  };

  const toggleSearch = () => {
    const v = viewRef.current;
    if (!v) return;
    if (isSearchOpen()) {
      closeSearchPanel(v);
      v.focus();
    } else {
      v.focus();
      openSearchPanel(v);
    }
  };

  const handleEditorContextMenu = (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY });
  };

  const buildEditorContextMenu = (): ContextMenuItem[] => {
    const v = viewRef.current;
    const hasSelection = !!v && !v.state.selection.main.empty;
    const copySel = async () => {
      if (!v) return;
      const sel = v.state.selection.main;
      if (sel.empty) return;
      try { await navigator.clipboard.writeText(v.state.sliceDoc(sel.from, sel.to)); } catch { /* ignore */ }
    };
    const cutSel = async () => {
      if (!v) return;
      const sel = v.state.selection.main;
      if (sel.empty) return;
      try { await navigator.clipboard.writeText(v.state.sliceDoc(sel.from, sel.to)); } catch { /* ignore */ }
      v.dispatch(v.state.replaceSelection(""));
      v.focus();
    };
    const pasteAt = async () => {
      if (!v) return;
      try {
        const text = await navigator.clipboard.readText();
        if (text) v.dispatch(v.state.replaceSelection(text));
      } catch { /* ignore */ }
      v.focus();
    };
    const selectAll = () => {
      if (!v) return;
      v.dispatch({ selection: EditorSelection.single(0, v.state.doc.length) });
      v.focus();
    };
    return [
      { label: t("Cut"), action: () => void cutSel(), disabled: !hasSelection, shortcut: "Ctrl+X" },
      { label: t("Copy"), action: () => void copySel(), disabled: !hasSelection, shortcut: "Ctrl+C" },
      { label: t("Paste"), action: () => void pasteAt(), shortcut: "Ctrl+V" },
      { divider: true },
      { label: t("Select all"), action: selectAll, shortcut: "Ctrl+A" },
      { label: t("Find / Replace"), action: () => { if (v) { v.focus(); openSearchPanel(v); } }, shortcut: "Ctrl+F" },
    ];
  };

  // Escape to close (with dirty prompt). Scoped to the dialog lifecycle.
  useMonoKey((e) => {
    if (!open) return;
    if (e.key === "Escape") {
      // Let CM6 search panel eat Escape first — it closes itself.
      const view = viewRef.current;
      if (view && view.dom.querySelector(".cm-panel.cm-search")) return;
      e.preventDefault();
      requestClose();
    }
  });

  if (!open) return null;

  const langName = languageLabel(effectiveName);
  const sizeLabel = meta ? formatBytes(meta.size) : "—";
  const permLabel = meta?.permissions != null
    ? (meta.permissions & 0o777).toString(8).padStart(3, "0")
    : "—";

  return (
    <>
    <div
      className="dlg-overlay"
      onMouseDown={(e) => { overlayDownRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
        // Only close when the pointer went down on the overlay itself —
        // otherwise a drag-to-select ending on the overlay would dismiss
        // the dialog mid-selection.
        if (e.target === e.currentTarget && overlayDownRef.current) requestClose();
        overlayDownRef.current = false;
      }}
    >
      <div
        className="dlg dlg--editor"
        style={dialogStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <FileText size={13} />
            {effectiveName}
            {dirty && <span className="editor-dirty" title={t("Unsaved changes")}>●</span>}
          </span>
          <span className="editor-path mono" title={path}>{path}</span>
          <div style={{ flex: 1 }} />
          <IconButton
            variant="mini"
            onClick={toggleSearch}
            title={t("Find / Replace")}
          >
            <Search size={12} />
          </IconButton>
          <IconButton
            variant="mini"
            onClick={() => void saveRef.current()}
            disabled={!dirty || saving}
            title={t("Save")}
          >
            <Save size={12} />
          </IconButton>
          <IconButton variant="mini" onClick={requestClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        {meta?.lossy && (
          <div className="editor-warn">
            <AlertTriangle size={12} />
            <span>{t("Non-UTF-8 bytes were replaced with U+FFFD. Saving will persist the replacement.")}</span>
          </div>
        )}

        <div className="dlg-body dlg-body--editor">
          {loading && <div className="editor-loading mono">{t("Loading…")}</div>}
          {error && !loading && <div className="editor-error">{error}</div>}
          <div ref={hostRef} className="editor-host" onContextMenu={handleEditorContextMenu} />
        </div>

        <div className="editor-status mono">
          <span>{langName}</span>
          <span className="sep">·</span>
          <span>
            {t("Ln {line}, Col {col}", { line: cursor.line, col: cursor.col })}
          </span>
          {cursor.selLen > 0 && (
            <>
              <span className="sep">·</span>
              <span>{t("{n} selected", { n: cursor.selLen })}</span>
            </>
          )}
          <div style={{ flex: 1 }} />
          <span>{sizeLabel}</span>
          <span className="sep">·</span>
          <span>{t("Perm")} {permLabel}</span>
          <span className="sep">·</span>
          <span>UTF-8</span>
        </div>
      </div>
    </div>
    {ctxMenu && (
      <ContextMenu
        x={ctxMenu.x}
        y={ctxMenu.y}
        items={buildEditorContextMenu()}
        onClose={() => setCtxMenu(null)}
      />
    )}
    </>
  );
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let val = n;
  let u = 0;
  while (val >= 1024 && u < units.length - 1) {
    val /= 1024;
    u++;
  }
  return `${val < 10 && u > 0 ? val.toFixed(1) : Math.round(val)} ${units[u]}`;
}
