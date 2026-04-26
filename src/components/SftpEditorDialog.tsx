import { useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import {
  AlertTriangle,
  AlignJustify,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  Copy,
  Download,
  Edit,
  ExternalLink,
  Eye,
  FileText,
  HardDrive,
  Key,
  List,
  Loader2,
  Replace,
  RotateCcw,
  Save,
  Search,
  Server,
  User,
  X,
} from "lucide-react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { EditorState, EditorSelection, Compartment, type Extension } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers as cmLineNumbers,
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
  closeSearchPanel,
  highlightSelectionMatches,
  SearchQuery,
  setSearchQuery,
  findNext,
  findPrevious,
  replaceNext,
  replaceAll,
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
import { useSettingsStore } from "../stores/useSettingsStore";
import { writeClipboardText } from "../lib/clipboard";
import * as cmd from "../lib/commands";
import type { SftpExternalEditEvent, SftpTextFile } from "../lib/commands";
import { SFTP_EXTERNAL_EDIT_EVENT } from "../lib/commands";
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
  /** Optional file size hint from the listing. When > MAX_EDITOR_BYTES
   *  the dialog skips the inline read entirely and renders the
   *  "too large to inline-edit" branch with the external-editor
   *  hand-off + download fallback instead of erroring out. */
  size?: number;
  sshArgs: SftpEditorSshArgs;
  onClose: () => void;
  /** Called after a successful save with the persisted byte count. */
  onSaved?: (bytes: number) => void;
  /** Optional owner label shown in the head chips (e.g. "deploy"). */
  ownerLabel?: string;
};

type Mode = "view" | "edit";

/** Above this byte count we skip the CodeMirror language extension —
 *  syntax highlighting on multi-MB files turns the initial mount
 *  into a multi-second blocking parse. Line numbers, search, and
 *  bracket matching still work; only the colored tokens go. */
const LARGE_LANGUAGE_BYTES = 256 * 1024;

type ExternalEditState = {
  watcherId: string;
  localPath: string;
  status: "opening" | "watching" | "uploading" | "uploaded" | "error";
  /** Wall-clock seconds since epoch for the most recent successful
   *  upload — drives the "Last synced" footer label. */
  lastSyncedAt?: number;
  lastError?: string;
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
  size,
  sshArgs,
  onClose,
  onSaved,
  ownerLabel,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const viewRef = useRef<EditorView | null>(null);
  const baselineRef = useRef<string>("");
  const saveRef = useRef<() => Promise<void>>(async () => {});
  const openFindRef = useRef<(withReplace: boolean) => void>(() => {});

  // ── Editor preferences (Settings → Editor) ─────────────────
  // Read defaults from the global settings store; the dialog's
  // wrap / line-numbers toggles are session overrides on top.
  const editorWrapDefault = useSettingsStore((s) => s.editorWrapDefault);
  const editorLineNumbersDefault = useSettingsStore((s) => s.editorLineNumbersDefault);
  const editorTabSize = useSettingsStore((s) => s.editorTabSize);
  const trimTrailingOnSave = useSettingsStore((s) => s.editorTrimTrailingOnSave);
  const ensureFinalNewlineOnSave = useSettingsStore((s) => s.editorEnsureFinalNewlineOnSave);

  // Compartments let us toggle features at runtime — read-only for the
  // View mode segment, line-wrap for the toolbar, line-numbers for the
  // toolbar, without rebuilding the whole EditorState each time.
  const readOnlyComp = useRef(new Compartment()).current;
  const wrapComp = useRef(new Compartment()).current;
  const lineNumsComp = useRef(new Compartment()).current;

  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [meta, setMeta] = useState<Pick<
    SftpTextFile,
    "size" | "permissions" | "modified" | "lossy" | "owner" | "group" | "eol" | "encoding"
  > | null>(null);
  const [dirty, setDirty] = useState(false);
  const [mode, setMode] = useState<Mode>("edit");
  // External-editor session, when the user picks "Open with system
  // editor" (either explicitly via the toolbar or implicitly because
  // the file is too large for inline editing). Held in a ref alongside
  // state so the cleanup path on close can stop the watcher even if
  // the React state has already moved on.
  const [externalEdit, setExternalEdit] = useState<ExternalEditState | null>(null);
  const externalEditRef = useRef<ExternalEditState | null>(null);
  externalEditRef.current = externalEdit;
  const [externalBusy, setExternalBusy] = useState(false);
  // True the moment we know the file exceeds MAX_EDITOR_BYTES — drives
  // the "too large to inline-edit" branch so we never even attempt
  // sftp_read_text against a multi-MB file.
  const tooLargeForInline = typeof size === "number" && size > MAX_EDITOR_BYTES;
  // "Card" view — replaces the inline editor / toolbar / footer with a
  // single status card. Triggered when the file is too large to inline
  // edit OR when the user has handed it off to the OS default editor
  // (in which case keeping the inline editor visible would invite
  // confusing competing-write races).
  const cardMode = tooLargeForInline || !!externalEdit;
  const [wrap, setWrap] = useState(editorWrapDefault);
  const [showNums, setShowNums] = useState(editorLineNumbersDefault);
  const [cursor, setCursor] = useState<{ line: number; col: number; selLen: number; totalLines: number }>({ line: 1, col: 1, selLen: 0, totalLines: 0 });
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const [copiedPath, setCopiedPath] = useState(false);
  const overlayDownRef = useRef(false);

  // In-dialog Find/Replace bar state — drives CodeMirror via setSearchQuery
  // + findNext/findPrevious/replaceNext/replaceAll, instead of CM's bottom panel.
  const [findOpen, setFindOpen] = useState(false);
  const [findReplace, setFindReplace] = useState(false);
  const [findText, setFindText] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const [findRegex, setFindRegex] = useState(false);
  const [findCase, setFindCase] = useState(false);
  const findInputRef = useRef<HTMLInputElement | null>(null);

  const formatError = (e: unknown) => localizeError(e, t);
  const effectiveName = useMemo(() => name || basename(path), [name, path]);
  const phrases = useMemo(() => buildEditorPhrases(t), [t]);

  // Load file content when the dialog opens or path changes.
  useEffect(() => {
    if (!open) return;
    let alive = true;
    setError("");
    setDirty(false);
    setMode("edit");
    // Too-large files skip the inline read entirely — the dialog
    // body shows the external-editor / download branch instead.
    // Seeding `meta.size` from the prop lets the header chip render
    // useful info even though we never call sftp_read_text.
    if (tooLargeForInline) {
      setLoading(false);
      setMeta({
        size: size ?? 0,
        permissions: null,
        modified: null,
        lossy: false,
        owner: "",
        group: "",
        eol: "",
        encoding: "",
      });
      return () => {
        alive = false;
        disposeEditor();
      };
    }
    setLoading(true);
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
          owner: res.owner,
          group: res.group,
          eol: res.eol,
          encoding: res.encoding,
        });
        // Two-step mount: bring the empty editor up immediately so the
        // dialog frame paints, then push the content on the next frame.
        // For multi-100KB files the language extension turns the
        // initial parse into a multi-second blocking call, so we drop
        // it past LARGE_LANGUAGE_BYTES and keep just line numbers,
        // search, and bracket matching.
        const disableLanguage = res.content.length > LARGE_LANGUAGE_BYTES;
        mountEditor("", { disableLanguage });
        requestAnimationFrame(() => {
          if (!alive) return;
          const v = viewRef.current;
          if (!v) return;
          v.dispatch({ changes: { from: 0, insert: res.content } });
          baselineRef.current = res.content;
          setDirty(false);
        });
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
  }, [open, path, tooLargeForInline]);

  useEffect(() => () => disposeEditor(), []);

  function disposeEditor() {
    if (viewRef.current) {
      viewRef.current.destroy();
      viewRef.current = null;
    }
  }

  // Subscribe to backend `sftp:external-edit` events while a watcher
  // is active so the dialog can show upload status (uploading →
  // uploaded → optional error) and clear itself when the backend
  // emits `stopped`. Filter by watcherId so multiple SFTP panels
  // don't cross-talk when the user opens the same file twice.
  useEffect(() => {
    if (!externalEdit?.watcherId) return;
    const watcherId = externalEdit.watcherId;
    let alive = true;
    let unlisten: UnlistenFn | null = null;
    void listen<SftpExternalEditEvent>(SFTP_EXTERNAL_EDIT_EVENT, (evt) => {
      const payload = evt.payload;
      if (!payload || payload.watcherId !== watcherId) return;
      setExternalEdit((cur) => {
        if (!cur || cur.watcherId !== watcherId) return cur;
        switch (payload.kind) {
          case "uploading":
            return { ...cur, status: "uploading", lastError: undefined };
          case "uploaded":
            return {
              ...cur,
              status: "uploaded",
              lastSyncedAt: payload.modified ?? Math.floor(Date.now() / 1000),
              lastError: undefined,
            };
          case "error":
            return { ...cur, status: "error", lastError: payload.error ?? "" };
          case "stopped":
            return null;
          default:
            return cur;
        }
      });
      if (payload.kind === "uploaded") {
        // Refresh the panel listing so the file's mtime/size jump is
        // visible without a manual refresh.
        onSaved?.(payload.bytes ?? 0);
      }
    }).then((dispose) => {
      if (!alive) {
        dispose();
      } else {
        unlisten = dispose;
      }
    });
    return () => {
      alive = false;
      unlisten?.();
    };
    // onSaved is intentionally omitted — capturing the latest reference
    // would re-subscribe on every render. The closure above is safe to
    // call against a stale onSaved because it's an event hook only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalEdit?.watcherId]);

  // Wind down any active watcher when the dialog actually closes
  // (the user picked Close, parent cleared `editorTarget`, etc.).
  // Using a ref means we don't have to thread externalEdit through
  // requestClose / unmount paths.
  useEffect(() => {
    if (open) return;
    const cur = externalEditRef.current;
    if (!cur) return;
    void cmd.sftpExternalEditStop(cur.watcherId).catch(() => {});
    setExternalEdit(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Hard cleanup on unmount — covers the React Strict Mode double-
  // invoke and the unusual case where the parent unmounts the dialog
  // without flipping `open` first.
  useEffect(
    () => () => {
      const cur = externalEditRef.current;
      if (cur) void cmd.sftpExternalEditStop(cur.watcherId).catch(() => {});
    },
    [],
  );

  async function startExternalEdit() {
    if (externalBusy || externalEdit) return;
    setExternalBusy(true);
    setError("");
    try {
      const res = await cmd.sftpOpenExternal({ ...sshArgs, path });
      // Tear down the inline editor — we've now handed authority
      // over the file to the OS editor, and keeping the CodeMirror
      // buffer alive would let an in-dialog Save race the watcher.
      disposeEditor();
      setDirty(false);
      setExternalEdit({
        watcherId: res.watcherId,
        localPath: res.localPath,
        status: "watching",
      });
    } catch (e) {
      setError(formatError(e));
    } finally {
      setExternalBusy(false);
    }
  }

  async function stopExternalEdit() {
    const cur = externalEditRef.current;
    if (!cur) return;
    setExternalBusy(true);
    try {
      await cmd.sftpExternalEditStop(cur.watcherId);
    } catch {
      /* idempotent on the backend; UI already cleared */
    } finally {
      setExternalEdit(null);
      setExternalBusy(false);
      // Close the dialog as well — once the watcher is gone there's
      // nothing left to render in card mode, and dropping back to a
      // blank inline editor host would be more confusing than
      // helpful. Re-opening the file from the panel re-fetches.
      onClose();
    }
  }

  function mountEditor(initial: string, opts: { disableLanguage?: boolean } = {}) {
    disposeEditor();
    const host = hostRef.current;
    if (!host) return;
    const lang = opts.disableLanguage ? null : languageFromFilename(effectiveName);
    const extensions: Extension[] = [
      EditorState.phrases.of(phrases),
      EditorState.tabSize.of(editorTabSize),
      lineNumsComp.of(showNums ? cmLineNumbers() : []),
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
      wrapComp.of(wrap ? EditorView.lineWrapping : []),
      readOnlyComp.of(EditorState.readOnly.of(false)),
      keymap.of([
        { key: "Mod-s", preventDefault: true, run: () => { void saveRef.current(); return true; } },
        { key: "Mod-f", preventDefault: true, run: () => { openFindRef.current(false); return true; } },
        { key: "Mod-h", preventDefault: true, run: () => { openFindRef.current(true); return true; } },
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
            totalLines: u.state.doc.lines,
          });
        }
      }),
    ];
    if (lang) extensions.push(lang);

    const state = EditorState.create({ doc: initial, extensions });
    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    setCursor((c) => ({ ...c, totalLines: state.doc.lines }));
    view.dispatch({ selection: EditorSelection.single(0) });
    view.focus();
  }

  // Toggle compartments when the toolbar state changes.
  useEffect(() => {
    const v = viewRef.current;
    if (!v) return;
    v.dispatch({
      effects: wrapComp.reconfigure(wrap ? EditorView.lineWrapping : []),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wrap]);

  useEffect(() => {
    const v = viewRef.current;
    if (!v) return;
    v.dispatch({
      effects: lineNumsComp.reconfigure(showNums ? cmLineNumbers() : []),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showNums]);

  useEffect(() => {
    const v = viewRef.current;
    if (!v) return;
    v.dispatch({
      effects: readOnlyComp.reconfigure(EditorState.readOnly.of(mode === "view")),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode]);

  saveRef.current = async () => {
    const view = viewRef.current;
    if (!view || saving) return;
    let content = view.state.doc.toString();

    // Apply on-save transforms before either replacing the buffer or
    // shipping bytes. We mutate the buffer too so the user sees the
    // post-save shape — otherwise editing a "clean" file then saving
    // would silently change it on disk vs what's in the editor.
    if (trimTrailingOnSave) {
      content = content.replace(/[ \t]+$/gm, "");
    }
    if (ensureFinalNewlineOnSave) {
      // Normalize to exactly one trailing newline (drop excess, add
      // one if missing). Empty buffer stays empty.
      if (content.length > 0) {
        content = content.replace(/\n+$/, "") + "\n";
      }
    }
    if (content !== view.state.doc.toString()) {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: content },
      });
    }

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

  // Sync the in-dialog find inputs to CodeMirror's search query so
  // findNext/findPrevious/replaceNext/replaceAll act on the right string.
  const syncSearchQuery = () => {
    const v = viewRef.current;
    if (!v) return;
    v.dispatch({
      effects: setSearchQuery.of(
        new SearchQuery({
          search: findText,
          replace: replaceText,
          caseSensitive: findCase,
          regexp: findRegex,
        }),
      ),
    });
  };

  useEffect(() => {
    if (!findOpen) return;
    syncSearchQuery();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [findOpen, findText, replaceText, findCase, findRegex]);

  const openFind = (withReplace: boolean) => {
    if (withReplace) {
      setFindReplace(true);
      if (mode === "view") setMode("edit");
    }
    setFindOpen(true);
    requestAnimationFrame(() => findInputRef.current?.focus());
  };
  openFindRef.current = openFind;

  const closeFind = () => {
    setFindOpen(false);
    const v = viewRef.current;
    if (v) {
      // CM might also have its bottom panel open from a stray ⌘F earlier — tidy up.
      closeSearchPanel(v);
      v.focus();
    }
  };

  const goNext = () => {
    const v = viewRef.current;
    if (!v || !findText) return;
    findNext(v);
  };
  const goPrev = () => {
    const v = viewRef.current;
    if (!v || !findText) return;
    findPrevious(v);
  };
  const doReplaceOne = () => {
    const v = viewRef.current;
    if (!v || !findText) return;
    replaceNext(v);
  };
  const doReplaceAll = () => {
    const v = viewRef.current;
    if (!v || !findText) return;
    replaceAll(v);
  };

  const revert = () => {
    const v = viewRef.current;
    if (!v || !dirty) return;
    v.dispatch({
      changes: { from: 0, to: v.state.doc.length, insert: baselineRef.current },
    });
    v.focus();
  };

  const copyPath = () => {
    void writeClipboardText(path);
    setCopiedPath(true);
    window.setTimeout(() => setCopiedPath(false), 1200);
  };

  const download = async () => {
    const v = viewRef.current;
    // Inline-editor path: serialize the in-memory buffer through the
    // browser's anchor-download trick. Lets the user grab their
    // edited-but-unsaved buffer without an SFTP round-trip.
    if (v) {
      try {
        const blob = new Blob([v.state.doc.toString()], { type: "text/plain;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = effectiveName || "download.txt";
        document.body.appendChild(a);
        a.click();
        a.remove();
        window.setTimeout(() => URL.revokeObjectURL(url), 0);
      } catch {
        /* ignore — browser may reject download in sandboxed env */
      }
      return;
    }
    // Card-mode path: no in-memory buffer to write out, so route
    // through `sftp_download` against a user-picked folder. This is
    // the same flow the panel context menu uses for "Download…".
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: t("Select download folder"),
      });
      if (!picked || typeof picked !== "string") return;
      const sep = /^[A-Za-z]:($|[\\/])|^\\\\/.test(picked) ? "\\" : "/";
      const localPath = `${picked.replace(/[\\/]+$/, "")}${sep}${effectiveName || "download"}`;
      await cmd.sftpDownload({ ...sshArgs, remotePath: path, localPath });
    } catch (e) {
      setError(formatError(e));
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
      { label: t("Find / Replace"), action: () => openFindRef.current(true), shortcut: "Ctrl+F" },
    ];
  };

  useMonoKey((e) => {
    if (!open) return;
    if (e.key === "Escape") {
      const view = viewRef.current;
      if (view && view.dom.querySelector(".cm-panel.cm-search")) return;
      if (findOpen) {
        e.preventDefault();
        closeFind();
        return;
      }
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
  const modifiedLabel = meta?.modified
    ? new Date(meta.modified * 1000).toISOString().replace("T", " ").slice(0, 16)
    : "—";
  // Prefer the owner string from the read response (may include
  // both owner and group in `user:group` form); fall back to the
  // panel-supplied `ownerLabel` prop so the chip stays useful
  // before the read settles.
  const headerOwnerLabel = (() => {
    const o = meta?.owner ?? "";
    const g = meta?.group ?? "";
    if (o && g && o !== g) return `${o}:${g}`;
    if (o) return o;
    return ownerLabel;
  })();
  const eolLabel = (() => {
    switch (meta?.eol) {
      case "lf":
        return "LF";
      case "crlf":
        return "CRLF";
      case "cr":
        return "CR";
      case "mixed":
        return t("Mixed");
      case "none":
        return "—";
      default:
        return null;
    }
  })();
  const encodingLabel = (() => {
    switch (meta?.encoding) {
      case "utf-8":
        return "UTF-8";
      case "utf-8-bom":
        return "UTF-8 BOM";
      case "utf-16-le":
        return "UTF-16 LE";
      case "utf-16-be":
        return "UTF-16 BE";
      case "binary":
        return t("Binary");
      default:
        return null;
    }
  })();

  return (
    <>
    <div
      className="dlg-overlay"
      onMouseDown={(e) => { overlayDownRef.current = e.target === e.currentTarget; }}
      onClick={(e) => {
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
          <span className="editor-head-chips">
            <span className="editor-chip"><HardDrive size={9} /> {sizeLabel}</span>
            <span className="editor-chip"><Clock size={9} /> {modifiedLabel}</span>
            <span className="editor-chip"><Key size={9} /> {permLabel}</span>
            {headerOwnerLabel && (
              <span className="editor-chip"><User size={9} /> {headerOwnerLabel}</span>
            )}
          </span>
          <span className="editor-path mono" title={path}>{path}</span>
          <div className="editor-mode-seg" role="tablist">
            <button
              type="button"
              className={"editor-mode" + (mode === "view" ? " on" : "")}
              onClick={() => setMode("view")}
            >
              <Eye size={10} /> {t("View")}
            </button>
            <button
              type="button"
              className={"editor-mode" + (mode === "edit" ? " on" : "")}
              onClick={() => setMode("edit")}
            >
              <Edit size={10} /> {t("Edit")}
            </button>
          </div>
          <IconButton variant="mini" onClick={requestClose} title={t("Close")}>
            <X size={12} />
          </IconButton>
        </div>

        {!cardMode && (
        <div className="editor-toolbar">
          <span className="editor-toolbar-stat">
            <b>{langName}</b>
          </span>
          <span className="editor-toolbar-stat">
            <b>{cursor.totalLines.toLocaleString()}</b> {t("lines")}
          </span>
          <span className="editor-toolbar-stat">
            <b>{sizeLabel}</b>
          </span>
          <span className="editor-toolbar-spacer" />
          <button
            type="button"
            className={"editor-tool-btn" + (findOpen && !findReplace ? " on" : "")}
            title={t("Find (⌘F)")}
            onClick={() => (findOpen && !findReplace ? closeFind() : openFind(false))}
          >
            <Search size={11} />
          </button>
          <button
            type="button"
            className={"editor-tool-btn" + (findOpen && findReplace ? " on" : "")}
            title={t("Find & Replace (⌘H)")}
            onClick={() => (findOpen && findReplace ? closeFind() : openFind(true))}
          >
            <Replace size={11} />
          </button>
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
            className={"editor-tool-btn" + (showNums ? " on" : "")}
            title={t("Line numbers")}
            onClick={() => setShowNums((v) => !v)}
          >
            <List size={11} />
          </button>
          <span className="editor-toolbar-divider" />
          <button
            type="button"
            className="editor-tool-btn"
            title={t("Download")}
            onClick={() => void download()}
          >
            <Download size={11} />
          </button>
          <button
            type="button"
            className="editor-tool-btn"
            title={t("Open with system editor")}
            disabled={externalBusy || !!externalEdit}
            onClick={() => void startExternalEdit()}
          >
            <ExternalLink size={11} />
          </button>
          <button
            type="button"
            className={"editor-tool-btn" + (copiedPath ? " on" : "")}
            title={t("Copy path")}
            onClick={copyPath}
          >
            <Copy size={11} />
          </button>
          {mode === "edit" && (
            <>
              <span className="editor-toolbar-divider" />
              <button
                type="button"
                className="editor-tool-btn"
                title={t("Revert")}
                disabled={!dirty}
                onClick={revert}
              >
                <RotateCcw size={11} />
              </button>
              <button
                type="button"
                className="btn is-primary is-compact"
                disabled={!dirty || saving}
                onClick={() => void saveRef.current()}
              >
                <Save size={10} /> {saving ? t("Saving...") : t("Save")}
              </button>
            </>
          )}
        </div>
        )}

        {!cardMode && meta?.lossy && (
          <div className="editor-warn">
            <AlertTriangle size={12} />
            <span>{t("Non-UTF-8 bytes were replaced with U+FFFD. Saving will persist the replacement.")}</span>
          </div>
        )}

        {!cardMode && findOpen && (
          <div className="editor-find">
            <div className="editor-find-row">
              <Search size={11} />
              <input
                ref={findInputRef}
                className="editor-find-input mono"
                placeholder={findRegex ? t("/regex/") : t("find…")}
                value={findText}
                onChange={(e) => setFindText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    if (e.shiftKey) goPrev();
                    else goNext();
                  }
                }}
              />
              <button
                type="button"
                className={"editor-find-opt mono" + (findCase ? " on" : "")}
                title={t("Case sensitive")}
                onClick={() => setFindCase((v) => !v)}
              >
                Aa
              </button>
              <button
                type="button"
                className={"editor-find-opt mono" + (findRegex ? " on" : "")}
                title={t("Regex")}
                onClick={() => setFindRegex((v) => !v)}
              >
                .*
              </button>
              <button
                type="button"
                className="editor-tool-btn"
                title={t("Previous (⇧⏎)")}
                onClick={goPrev}
                disabled={!findText}
              >
                <ChevronUp size={11} />
              </button>
              <button
                type="button"
                className="editor-tool-btn"
                title={t("Next (⏎)")}
                onClick={goNext}
                disabled={!findText}
              >
                <ChevronDown size={11} />
              </button>
              <button
                type="button"
                className={"editor-tool-btn" + (findReplace ? " on" : "")}
                title={t("Toggle replace")}
                onClick={() => {
                  setFindReplace((v) => !v);
                  if (!findReplace && mode === "view") setMode("edit");
                }}
              >
                <Edit size={11} />
              </button>
              <button
                type="button"
                className="editor-tool-btn"
                title={t("Close (Esc)")}
                onClick={closeFind}
              >
                <X size={11} />
              </button>
            </div>
            {findReplace && (
              <div className="editor-find-row">
                <Replace size={11} />
                <input
                  className="editor-find-input mono"
                  placeholder={t("replace with…")}
                  value={replaceText}
                  onChange={(e) => setReplaceText(e.target.value)}
                />
                <button
                  type="button"
                  className="btn is-compact"
                  onClick={doReplaceOne}
                  disabled={!findText || mode !== "edit"}
                >
                  {t("Replace")}
                </button>
                <button
                  type="button"
                  className="btn is-primary is-compact"
                  onClick={doReplaceAll}
                  disabled={!findText || mode !== "edit"}
                >
                  {t("Replace all")}
                </button>
              </div>
            )}
          </div>
        )}

        <div className="dlg-body dlg-body--editor">
          {cardMode ? (
            <ExternalEditCard
              size={size ?? meta?.size ?? 0}
              tooLarge={tooLargeForInline}
              limit={MAX_EDITOR_BYTES}
              externalEdit={externalEdit}
              busy={externalBusy}
              error={error}
              onOpenExternal={() => void startExternalEdit()}
              onStopWatcher={() => void stopExternalEdit()}
              onDownload={() => void download()}
              t={t}
            />
          ) : (
            <>
              {loading && <div className="editor-loading mono">{t("Loading…")}</div>}
              {error && !loading && <div className="editor-error">{error}</div>}
              <div ref={hostRef} className="editor-host" onContextMenu={handleEditorContextMenu} />
            </>
          )}
        </div>

        {!cardMode && (
          <div className="editor-status mono">
            <span className="editor-status-cell">
              <Server size={9} /> {sshArgs.user}@{sshArgs.host}
            </span>
            <span className="sep">·</span>
            <span>{t("Ln {line}, Col {col}", { line: cursor.line, col: cursor.col })}</span>
            {cursor.selLen > 0 && (
              <>
                <span className="sep">·</span>
                <span>{t("{n} selected", { n: cursor.selLen })}</span>
              </>
            )}
            <span className="editor-status-spacer" />
            <span>{encodingLabel ?? "UTF-8"}</span>
            <span className="sep">·</span>
            <span>{eolLabel ?? "LF"}</span>
            <span className="sep">·</span>
            <span>{langName}</span>
            <span className="sep">·</span>
            <span className={dirty ? "editor-status-dirty" : "editor-status-saved"}>
              {dirty ? t("modified") : t("saved")}
            </span>
          </div>
        )}
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

function formatClock(unixSeconds: number | undefined): string {
  if (!unixSeconds) return "—";
  const d = new Date(unixSeconds * 1000);
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Card view shown when the file is too large to inline-edit OR
 *  when the user has handed off to the OS default editor. Replaces
 *  the CodeMirror host + toolbar + footer for the duration of the
 *  external session. */
function ExternalEditCard({
  size,
  tooLarge,
  limit,
  externalEdit,
  busy,
  error,
  onOpenExternal,
  onStopWatcher,
  onDownload,
  t,
}: {
  size: number;
  tooLarge: boolean;
  limit: number;
  externalEdit: ExternalEditState | null;
  busy: boolean;
  error: string;
  onOpenExternal: () => void;
  onStopWatcher: () => void;
  onDownload: () => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
}) {
  const sizeLabel = formatBytes(size);
  const limitLabel = formatBytes(limit);
  return (
    <div className="editor-toolarge">
      <div className="editor-toolarge-icon">
        {externalEdit ? <ExternalLink size={28} /> : <AlertTriangle size={28} />}
      </div>
      <h3 className="editor-toolarge-title">
        {externalEdit
          ? t("Editing in your system editor")
          : tooLarge
            ? t("File is too large for inline editing")
            : t("Open with system editor")}
      </h3>
      <p className="editor-toolarge-sub">
        {externalEdit
          ? t("Saves are auto-uploaded back over SFTP.")
          : tooLarge
            ? t("{size} · inline editor handles up to {limit}.", { size: sizeLabel, limit: limitLabel })
            : t("Hand the file off to your OS default editor; saves auto-upload back.")}
      </p>

      {externalEdit ? (
        <>
          <div className="editor-toolarge-pathrow mono" title={externalEdit.localPath}>
            <HardDrive size={11} />
            <span className="editor-toolarge-path">{externalEdit.localPath}</span>
          </div>
          <div className="editor-extstatus">
            {externalEdit.status === "uploading" && (
              <>
                <Loader2 size={12} className="editor-extstatus-spin" />
                <span>{t("Uploading change…")}</span>
              </>
            )}
            {externalEdit.status === "uploaded" && (
              <>
                <CheckCircle2 size={12} className="editor-extstatus-ok" />
                <span>{t("Last synced {time}", { time: formatClock(externalEdit.lastSyncedAt) })}</span>
              </>
            )}
            {externalEdit.status === "error" && (
              <>
                <AlertTriangle size={12} className="editor-extstatus-err" />
                <span>{externalEdit.lastError || t("Upload failed")}</span>
              </>
            )}
            {externalEdit.status === "watching" && (
              <>
                <Clock size={12} />
                <span>{t("Watching for changes…")}</span>
              </>
            )}
            {externalEdit.status === "opening" && (
              <>
                <Loader2 size={12} className="editor-extstatus-spin" />
                <span>{t("Opening editor…")}</span>
              </>
            )}
          </div>
          <div className="editor-toolarge-actions">
            <button
              type="button"
              className="btn"
              onClick={onStopWatcher}
              disabled={busy}
              title={t("Stop watching and close this dialog. The file may remain open in your system editor.")}
            >
              <X size={11} /> {t("Done")}
            </button>
          </div>
        </>
      ) : (
        <>
          {error && <div className="editor-toolarge-err">{error}</div>}
          <div className="editor-toolarge-actions">
            <button
              type="button"
              className="btn is-primary"
              onClick={onOpenExternal}
              disabled={busy}
            >
              <ExternalLink size={11} />{" "}
              {busy ? t("Opening…") : t("Open with system editor")}
            </button>
            <button
              type="button"
              className="btn"
              onClick={onDownload}
              disabled={busy}
            >
              <Download size={11} /> {t("Download…")}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
