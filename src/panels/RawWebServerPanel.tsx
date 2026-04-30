import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Code2,
  Diff,
  ExternalLink,
  FilePlus2,
  FileText,
  Network,
  Power,
  RefreshCw,
  Save,
  ShieldCheck,
  Sparkles,
  ToggleLeft,
  ToggleRight,
  Undo2,
  Redo2,
  X,
} from "lucide-react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import DiffPreview from "../components/DiffPreview";
import ApacheFeatureCatalog from "./ApacheFeatureCatalog";
import ApacheIfModuleEditor from "./ApacheIfModuleEditor";
import ApacheTreeView from "./ApacheTreeView";
import CaddyFeatureCatalog from "./CaddyFeatureCatalog";
import CaddyMatcherEditor from "./CaddyMatcherEditor";
import CaddyTreeView from "./CaddyTreeView";
import NewWebServerSite from "./NewWebServerSite";
import type { ApacheNode, CaddyNode } from "../lib/commands";
import * as cmd from "../lib/commands";
import type {
  SshParams,
  WebServerFile,
  WebServerKind,
  WebServerLayout,
  WebServerSaveResult,
  WebServerActionResult,
  WebServerExternalEditEvent,
} from "../lib/commands";
import { WEB_SERVER_EXTERNAL_EDIT_EVENT } from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";

// Raw text editor for apache + caddy. Mirrors the nginx panel's save
// pipeline (backup → write → validate → restore-on-fail → reload) but
// without the AST parser / feature catalog. The user gets a file tree
// on the left and a textarea on the right; that's enough to make
// changes safe (validate before reload) without committing to a full
// per-product AST.

type Props = {
  kind: WebServerKind;
  sshParams: SshParams;
};

export default function RawWebServerPanel({ kind, sshParams }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const [layout, setLayout] = useState<WebServerLayout | null>(null);
  const [layoutBusy, setLayoutBusy] = useState(false);
  const [layoutError, setLayoutError] = useState("");
  const [activePath, setActivePathState] = useState<string | null>(null);
  const [content, setContent] = useState<string | null>(null);
  const [dirty, setDirty] = useState<string | null>(null);
  // Pending edits for files other than the currently-open one. We
  // stash dirty-buffer + on-disk baseline keyed by file path so the
  // user can switch tabs without losing work and so a Save-all run
  // has the full set in one place. The active file's edit lives in
  // `dirty` until either (a) the user switches files (we move it
  // into the map) or (b) a save commits it.
  const [pendingDirty, setPendingDirty] = useState<Record<string, string>>(
    {},
  );
  const [pendingBaselines, setPendingBaselines] = useState<
    Record<string, string>
  >({});
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchResult, setBatchResult] =
    useState<cmd.WebServerBatchSaveResult | null>(null);
  const [openBusy, setOpenBusy] = useState(false);
  const [openError, setOpenError] = useState("");
  const [saveBusy, setSaveBusy] = useState(false);
  const [saveResult, setSaveResult] = useState<WebServerSaveResult | null>(null);
  const [validateBusy, setValidateBusy] = useState(false);
  const [validateResult, setValidateResult] =
    useState<WebServerActionResult | null>(null);
  const [reloadBusy, setReloadBusy] = useState(false);
  const [reloadResult, setReloadResult] =
    useState<WebServerActionResult | null>(null);
  const [lintBusy, setLintBusy] = useState(false);
  const [lintResult, setLintResult] =
    useState<WebServerActionResult | null>(null);
  const [actionError, setActionError] = useState("");
  const [toggleBusy, setToggleBusy] = useState<string | null>(null);
  const [newSiteOpen, setNewSiteOpen] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  // Undo/redo for AST-level mutations (feature toggles, tree edits,
  // site enable/disable). Textarea typing has its own native undo —
  // we don't intercept that to avoid double-undo confusion. Stacks
  // are bounded to prevent unbounded memory growth on long sessions.
  const [undoStack, setUndoStack] = useState<string[]>([]);
  const [redoStack, setRedoStack] = useState<string[]>([]);
  const UNDO_CAP = 50;
  // "raw" textarea | "tree" read-only structured view | "features"
  // toggle catalog (caddy + apache) | "ifmodule" Apache <IfModule>
  // conditional editor. Modes that operate on the AST share the
  // same parse-on-entry / re-render-on-apply flow keyed by `mode`.
  const [mode, setMode] = useState<
    "raw" | "tree" | "features" | "ifmodule" | "matchers"
  >("raw");
  const [featuresAst, setFeaturesAst] = useState<
    CaddyNode[] | ApacheNode[] | null
  >(null);
  const [featuresParseError, setFeaturesParseError] = useState("");
  const [featuresApplying, setFeaturesApplying] = useState(false);

  // Hand-off to the OS default editor. While `externalEdit` is non-null
  // the panel shows a status banner and disables the inline editor —
  // every save in the external app re-runs the backup→write→validate→
  // restore-on-fail→reload pipeline on the backend, so in-panel edits
  // would silently lose to the external file's next save.
  type ExternalEditState = {
    watcherId: string;
    localPath: string;
    remotePath: string;
    status: "opened" | "uploading" | "uploaded" | "error";
    lastError?: string;
    lastSyncedAt?: number;
    validateOk?: boolean;
    reloaded?: boolean;
    restored?: boolean;
  };
  const [externalEdit, setExternalEdit] = useState<ExternalEditState | null>(
    null,
  );
  const externalEditRef = useRef<ExternalEditState | null>(null);
  externalEditRef.current = externalEdit;
  const [openExternalBusy, setOpenExternalBusy] = useState(false);

  const handleOpenExternal = async () => {
    if (!activePath || openExternalBusy || externalEdit) return;
    if (isDirty) {
      setActionError(
        t(
          "Save or discard the unsaved buffer before opening this file in an external editor.",
        ),
      );
      return;
    }
    setOpenExternalBusy(true);
    setActionError("");
    try {
      const result = await cmd.webServerOpenExternal({
        ...sshParams,
        kind,
        path: activePath,
      });
      setExternalEdit({
        watcherId: result.watcherId,
        localPath: result.localPath,
        remotePath: activePath,
        status: "opened",
      });
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setOpenExternalBusy(false);
    }
  };

  const stopExternalEdit = async () => {
    const cur = externalEditRef.current;
    if (!cur) return;
    setExternalEdit(null);
    try {
      await cmd.webServerExternalEditStop(cur.watcherId);
    } catch {
      /* best-effort — backend is idempotent */
    }
  };

  // Subscribe to backend events while a watcher is alive. Filters by
  // watcherId so multiple panels (or two open files) don't cross-talk.
  useEffect(() => {
    const watcherId = externalEdit?.watcherId;
    if (!watcherId) return;
    let alive = true;
    let unlisten: UnlistenFn | null = null;
    void listen<WebServerExternalEditEvent>(
      WEB_SERVER_EXTERNAL_EDIT_EVENT,
      (evt) => {
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
                lastSyncedAt:
                  payload.modified ?? Math.floor(Date.now() / 1000),
                lastError: undefined,
                validateOk: payload.validateOk ?? undefined,
                reloaded: payload.reloaded ?? undefined,
                restored: payload.restored ?? undefined,
              };
            case "error":
              return {
                ...cur,
                status: "error",
                lastError: payload.error ?? "",
                validateOk: payload.validateOk ?? undefined,
                reloaded: payload.reloaded ?? undefined,
                restored: payload.restored ?? undefined,
              };
            case "stopped":
              return null;
            default:
              return cur;
          }
        });
        // After a successful save, refresh the in-panel buffer so the
        // textarea matches what the external editor wrote — the user
        // can stop the session and keep editing in the panel without
        // a stale baseline.
        if (payload.kind === "uploaded" && payload.validateOk) {
          void cmd
            .webServerReadFile({
              ...sshParams,
              kind,
              path: externalEditRef.current?.remotePath ?? activePath ?? "",
            })
            .then((text) => {
              setContent(text);
              setDirty(null);
            })
            .catch(() => {});
        }
      },
    ).then((dispose) => {
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalEdit?.watcherId]);

  // Wind down the watcher if the user navigates to a different file
  // or unmounts the panel — leaving an orphaned watcher would keep
  // overwriting the now-inactive remote file with stale local edits.
  useEffect(() => {
    return () => {
      const cur = externalEditRef.current;
      if (!cur) return;
      void cmd.webServerExternalEditStop(cur.watcherId).catch(() => {});
    };
  }, []);
  useEffect(() => {
    const cur = externalEditRef.current;
    if (!cur) return;
    if (cur.remotePath !== activePath) {
      void cmd.webServerExternalEditStop(cur.watcherId).catch(() => {});
      setExternalEdit(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activePath]);

  const refreshLayout = async () => {
    if (layoutBusy) return;
    setLayoutBusy(true);
    setLayoutError("");
    try {
      const result = await cmd.webServerLayout({ ...sshParams, kind });
      setLayout(result);
      if (!activePath && result.files.length > 0) {
        const main = result.files.find((f) => f.kind.kind === "main");
        setActivePath(main?.path ?? result.files[0].path);
      }
    } catch (e) {
      setLayoutError(formatError(e));
    } finally {
      setLayoutBusy(false);
    }
  };

  useEffect(() => {
    void refreshLayout();
    // Refresh on host / kind change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshParams.host, sshParams.port, sshParams.user, kind]);

  /** Switch the active file. Snapshots the current dirty buffer +
   *  on-disk baseline into the pending maps so the user can switch
   *  tabs without losing edits. */
  const setActivePath = (next: string | null) => {
    if (activePath && content !== null) {
      const isDirtyNow = dirty !== null && dirty !== content;
      setPendingDirty((prev) => {
        const out = { ...prev };
        if (isDirtyNow) {
          out[activePath] = dirty as string;
        } else {
          delete out[activePath];
        }
        return out;
      });
      setPendingBaselines((prev) => ({ ...prev, [activePath]: content }));
    }
    setActivePathState(next);
  };

  // Load active file content. If we already have a baseline + a
  // pending dirty buffer for this path (because the user is flipping
  // back to a tab they edited earlier), seed from the maps instead of
  // hitting the server again.
  useEffect(() => {
    if (!activePath) {
      setContent(null);
      setDirty(null);
      setSaveResult(null);
      return;
    }
    const cachedBaseline = pendingBaselines[activePath];
    const cachedDirty = pendingDirty[activePath];
    if (cachedBaseline !== undefined) {
      setContent(cachedBaseline);
      setDirty(cachedDirty !== undefined ? cachedDirty : null);
      setSaveResult(null);
      return;
    }
    let cancelled = false;
    setOpenBusy(true);
    setOpenError("");
    cmd
      .webServerReadFile({ ...sshParams, kind, path: activePath })
      .then((text) => {
        if (cancelled) return;
        setContent(text);
        setDirty(null);
        setSaveResult(null);
      })
      .catch((e) => {
        if (cancelled) return;
        setOpenError(formatError(e));
      })
      .finally(() => {
        if (!cancelled) setOpenBusy(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshParams.host, sshParams.port, kind, activePath]);

  const editorValue = dirty ?? content ?? "";
  const isDirty = dirty !== null && dirty !== content;

  // When entering Features mode (or the buffer changes underneath
  // it), re-parse so the catalog reflects the current state. The
  // parser used depends on `kind`.
  useEffect(() => {
    const astMode =
      mode === "features" || mode === "ifmodule" || mode === "matchers";
    if (!astMode) {
      setFeaturesAst(null);
      setFeaturesParseError("");
      return;
    }
    // ifmodule mode is apache-only; matchers is caddy-only; features
    // works for both apache + caddy.
    if (mode === "ifmodule" && kind !== "apache") return;
    if (mode === "matchers" && kind !== "caddy") return;
    if (kind !== "caddy" && kind !== "apache") return;
    let cancelled = false;
    const parseFn =
      kind === "caddy" ? cmd.caddyParse : cmd.apacheParse;
    parseFn(editorValue)
      .then((result) => {
        if (cancelled) return;
        setFeaturesAst(result.nodes as CaddyNode[] | ApacheNode[]);
        setFeaturesParseError(
          result.errors.length > 0 ? result.errors.join("; ") : "",
        );
      })
      .catch((e) => {
        if (cancelled) return;
        setFeaturesParseError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [mode, editorValue, kind]);

  // Snapshot the *current* buffer state before applying an
  // AST-level mutation, so the user can step back through feature
  // toggles + tree edits. Clears the redo stack — anything ahead of
  // the new edit is unreachable.
  const pushUndoSnapshot = (current: string) => {
    setUndoStack((prev) => {
      const next = [...prev, current];
      return next.length > UNDO_CAP ? next.slice(next.length - UNDO_CAP) : next;
    });
    setRedoStack([]);
  };

  const handleFeatureChange = async (
    nextAst: CaddyNode[] | ApacheNode[],
  ) => {
    pushUndoSnapshot(editorValue);
    setFeaturesAst(nextAst);
    setFeaturesApplying(true);
    try {
      const text =
        kind === "caddy"
          ? await cmd.caddyRender(nextAst as CaddyNode[])
          : await cmd.apacheRender(nextAst as ApacheNode[]);
      setDirty(text);
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setFeaturesApplying(false);
    }
  };

  const handleTreeChange = (nextText: string) => {
    pushUndoSnapshot(editorValue);
    setDirty(nextText);
  };

  const undo = () => {
    if (undoStack.length === 0) return;
    const prev = undoStack[undoStack.length - 1];
    setUndoStack((s) => s.slice(0, -1));
    setRedoStack((s) => [...s, editorValue]);
    setDirty(prev);
  };

  const redo = () => {
    if (redoStack.length === 0) return;
    const next = redoStack[redoStack.length - 1];
    setRedoStack((s) => s.slice(0, -1));
    setUndoStack((s) => [...s, editorValue]);
    setDirty(next);
  };

  // Reset history when the active file changes — undo across files
  // would be confusing (and also semantically wrong since the on-disk
  // baseline differs).
  useEffect(() => {
    setUndoStack([]);
    setRedoStack([]);
  }, [activePath]);

  // Ctrl/Cmd+Z to undo, Ctrl/Cmd+Shift+Z (or Ctrl+Y) to redo. Skips
  // when focus is inside a text input / textarea so the native
  // textarea undo keeps working — same boundary the comment at
  // line 93 calls out. The AST-stack stays orthogonal to typing.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable) return;
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      if (e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if ((e.key === "z" && e.shiftKey) || e.key === "y") {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [undoStack, redoStack, editorValue]);

  const runValidate = async () => {
    if (validateBusy) return;
    setValidateBusy(true);
    setActionError("");
    setValidateResult(null);
    try {
      const result = await cmd.webServerValidate({ ...sshParams, kind });
      setValidateResult(result);
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setValidateBusy(false);
    }
  };

  const runReload = async () => {
    if (reloadBusy) return;
    setReloadBusy(true);
    setActionError("");
    setReloadResult(null);
    try {
      const result = await cmd.webServerReload({ ...sshParams, kind });
      setReloadResult(result);
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setReloadBusy(false);
    }
  };

  /** Deeper static-analysis pass: `apachectl -S` for Apache,
   *  `caddy adapt --pretty` for Caddy, `nginx -t -q` for nginx.
   *  Surfaces duplicate ServerNames, fall-through routes, and
   *  adapter warnings that pass `validate` but indicate sketchy
   *  config. */
  const runLint = async () => {
    if (lintBusy) return;
    setLintBusy(true);
    setActionError("");
    setLintResult(null);
    try {
      const result = await cmd.webServerLintHints({ ...sshParams, kind });
      setLintResult(result);
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setLintBusy(false);
    }
  };

  const handleSave = async () => {
    if (!activePath || !isDirty || saveBusy) return;
    setSaveBusy(true);
    setActionError("");
    setSaveResult(null);
    try {
      const result = await cmd.webServerSaveFile({
        ...sshParams,
        kind,
        path: activePath,
        content: editorValue,
      });
      setSaveResult(result);
      // On a successful round-trip the on-disk content matches editor;
      // on a validation-fail-then-restore round-trip the original is
      // back, so we should re-fetch.
      if (result.validate.ok) {
        setContent(editorValue);
        setDirty(null);
      } else {
        // Pull the restored content so the editor reflects it.
        const fresh = await cmd.webServerReadFile({
          ...sshParams,
          kind,
          path: activePath,
        });
        setContent(fresh);
        setDirty(null);
      }
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setSaveBusy(false);
    }
  };

  /** All paths currently dirty across the panel — pendingDirty plus
   *  the active file when its buffer differs from on-disk. */
  const allDirtyPaths = useMemo(() => {
    const paths = new Set(Object.keys(pendingDirty));
    if (activePath && isDirty) paths.add(activePath);
    return Array.from(paths);
    // isDirty is derived; tracking dirty/content/activePath suffices.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingDirty, activePath, dirty, content]);

  /** Save every dirty file in one shot: write all → validate the
   *  whole tree once → reload once. On validate-fail the backend
   *  restores every backup before returning, so the panel state
   *  needs to refetch for accurate baselines either way. */
  const handleBatchSave = async () => {
    if (batchBusy || saveBusy) return;
    if (allDirtyPaths.length === 0) return;
    const entries = allDirtyPaths.map((path) => {
      const draft =
        path === activePath ? editorValue : pendingDirty[path] ?? "";
      return { path, content: draft };
    });
    setBatchBusy(true);
    setActionError("");
    setSaveResult(null);
    setBatchResult(null);
    try {
      const result = await cmd.webServerSaveFilesBatch({
        ...sshParams,
        kind,
        entries,
      });
      setBatchResult(result);
      if (result.validate.ok) {
        // On success, clear pending edits and refresh baselines for
        // edited files. The active file's `content` follows suit.
        setPendingDirty({});
        const newBaselines: Record<string, string> = {
          ...pendingBaselines,
        };
        for (const e of entries) {
          newBaselines[e.path] = e.content;
        }
        setPendingBaselines(newBaselines);
        if (activePath) {
          if (newBaselines[activePath] !== undefined) {
            setContent(newBaselines[activePath]);
          }
          setDirty(null);
        }
      } else {
        // Restore landed; re-read every edited file so baselines and
        // visible buffers reflect the original on-disk content.
        const refreshed: Record<string, string> = { ...pendingBaselines };
        for (const e of entries) {
          try {
            const fresh = await cmd.webServerReadFile({
              ...sshParams,
              kind,
              path: e.path,
            });
            refreshed[e.path] = fresh;
          } catch {
            // best-effort
          }
        }
        setPendingBaselines(refreshed);
        setPendingDirty({});
        if (activePath && refreshed[activePath] !== undefined) {
          setContent(refreshed[activePath]);
          setDirty(null);
        }
      }
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setBatchBusy(false);
    }
  };

  const handleToggle = async (file: WebServerFile) => {
    if (file.kind.kind !== "site-available" || toggleBusy) return;
    setToggleBusy(file.path);
    setActionError("");
    try {
      await cmd.webServerToggleSite({
        ...sshParams,
        kind,
        siteName: file.label,
        enable: !file.kind.enabled,
      });
      await refreshLayout();
    } catch (e) {
      setActionError(formatError(e));
    } finally {
      setToggleBusy(null);
    }
  };

  const productName =
    kind === "apache" ? "Apache" : kind === "caddy" ? "Caddy" : "nginx";

  return (
    <div className="ws-raw">
      <div className="ws-raw__toolbar mono">
        <span className="ws-raw__product">
          {productName}
          {layout?.version && (
            <span className="ws-raw__version">{layout.version}</span>
          )}
        </span>
        <span className="ws-raw__spacer" />
        {(kind === "caddy" || kind === "apache") && (
          <div className="ws-raw__mode" role="tablist">
            <button
              type="button"
              role="tab"
              aria-selected={mode === "features"}
              className={`ws-raw__mode-btn ${mode === "features" ? "is-active" : ""}`}
              onClick={() => setMode("features")}
              title={
                kind === "apache"
                  ? t("Toggle common Apache features (SSL, Rewrite, headers, …)")
                  : t("Toggle common Caddy features (reverse_proxy, file_server, …)")
              }
            >
              <Sparkles size={11} /> {t("Features")}
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={mode === "tree"}
              className={`ws-raw__mode-btn ${mode === "tree" ? "is-active" : ""}`}
              onClick={() => setMode("tree")}
              title={t("Structured tree view (read-only)")}
            >
              <Network size={11} /> {t("Tree")}
            </button>
            {kind === "apache" && (
              <button
                type="button"
                role="tab"
                aria-selected={mode === "ifmodule"}
                className={`ws-raw__mode-btn ${mode === "ifmodule" ? "is-active" : ""}`}
                onClick={() => setMode("ifmodule")}
                title={t(
                  "List and edit <IfModule> conditional blocks",
                )}
              >
                <ShieldCheck size={11} /> {t("IfModule")}
              </button>
            )}
            {kind === "caddy" && (
              <button
                type="button"
                role="tab"
                aria-selected={mode === "matchers"}
                className={`ws-raw__mode-btn ${mode === "matchers" ? "is-active" : ""}`}
                onClick={() => setMode("matchers")}
                title={t(
                  "List and edit named (@xxx) matchers per site or snippet",
                )}
              >
                <ShieldCheck size={11} /> {t("Matchers")}
              </button>
            )}
            <button
              type="button"
              role="tab"
              aria-selected={mode === "raw"}
              className={`ws-raw__mode-btn ${mode === "raw" ? "is-active" : ""}`}
              onClick={() => setMode("raw")}
              title={t("Raw text editor")}
            >
              <Code2 size={11} /> {t("Raw")}
            </button>
          </div>
        )}
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => setNewSiteOpen(true)}
          title={t("Create a new site config")}
        >
          <FilePlus2 size={11} /> {t("New site")}
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon"
          onClick={undo}
          disabled={undoStack.length === 0}
          title={t("Undo last AST edit (feature toggle / tree edit)")}
          aria-label={t("Undo")}
        >
          <Undo2 size={11} />
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--icon"
          onClick={redo}
          disabled={redoStack.length === 0}
          title={t("Redo")}
          aria-label={t("Redo")}
        >
          <Redo2 size={11} />
        </button>
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => void refreshLayout()}
          disabled={layoutBusy}
          title={t("Re-scan config files")}
        >
          <RefreshCw size={11} />
        </button>
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => void runValidate()}
          disabled={validateBusy}
          title={t("Run config syntax check")}
        >
          <ShieldCheck size={11} /> {t("Validate")}
        </button>
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => void runLint()}
          disabled={lintBusy}
          title={t(
            "Run a deeper static analysis (apachectl -S / caddy adapt --pretty / nginx -t -q)",
          )}
        >
          <Sparkles size={11} /> {lintBusy ? t("Linting…") : t("Lint")}
        </button>
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => void runReload()}
          disabled={reloadBusy}
          title={t("Reload the daemon")}
        >
          <Power size={11} /> {t("Reload")}
        </button>
        <button
          type="button"
          className={`btn btn--ghost ${showDiff ? "is-active" : ""}`}
          onClick={() => setShowDiff((v) => !v)}
          disabled={!isDirty || !activePath}
          title={t("Preview diff against the on-disk version")}
        >
          <Diff size={11} /> {t("Diff")}
        </button>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => void handleSave()}
          disabled={
            !isDirty ||
            saveBusy ||
            batchBusy ||
            !activePath ||
            !!externalEdit
          }
          title={t("Save → validate → reload (with auto-restore on fail)")}
        >
          <Save size={11} /> {saveBusy ? t("Saving…") : t("Save")}
        </button>
        <button
          type="button"
          className="btn btn--ghost"
          onClick={() => void handleOpenExternal()}
          disabled={
            !activePath ||
            openExternalBusy ||
            !!externalEdit ||
            saveBusy ||
            batchBusy
          }
          title={t(
            "Open in your OS default editor; saves auto-push back through validate + reload",
          )}
        >
          <ExternalLink size={11} />{" "}
          {openExternalBusy ? t("Opening…") : t("Open externally")}
        </button>
        {allDirtyPaths.length > 1 && (
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => void handleBatchSave()}
            disabled={batchBusy || saveBusy}
            title={t(
              "Write every dirty file, then run a single validate + reload (auto-restores all on fail)",
            )}
          >
            <Save size={11} />{" "}
            {batchBusy
              ? t("Saving all…")
              : t("Save all ({n})", { n: allDirtyPaths.length })}
          </button>
        )}
      </div>

      {externalEdit && (
        <div
          className={`ws-raw__extedit ws-raw__extedit--${externalEdit.status}`}
        >
          <div className="ws-raw__extedit-line">
            <ExternalLink size={11} />
            <span className="ws-raw__extedit-label">
              {externalEdit.status === "uploading"
                ? t("Saving from external editor…")
                : externalEdit.status === "uploaded"
                  ? externalEdit.validateOk === false
                    ? t("Validate failed — config restored.")
                    : externalEdit.reloaded
                      ? t("Saved · validate ok · reloaded")
                      : t("Saved · validate ok · reload skipped")
                  : externalEdit.status === "error"
                    ? t("Save failed: {msg}", {
                        msg: externalEdit.lastError ?? "",
                      })
                    : t("Editing externally — saves auto-sync")}
            </span>
            {externalEdit.lastSyncedAt && (
              <span className="ws-raw__extedit-time mono">
                {new Date(
                  externalEdit.lastSyncedAt * 1000,
                ).toLocaleTimeString()}
              </span>
            )}
            <span className="ws-raw__extedit-path mono" title={externalEdit.localPath}>
              {externalEdit.localPath}
            </span>
            <button
              type="button"
              className="btn btn--ghost btn--sm"
              onClick={() => void stopExternalEdit()}
              title={t("Stop external-edit watcher and clean up the temp file")}
            >
              <X size={11} /> {t("Stop")}
            </button>
          </div>
        </div>
      )}

      <div className="ws-raw__body">
        <FileTree
          files={layout?.files ?? []}
          activePath={activePath}
          loading={layoutBusy && !layout}
          error={layoutError}
          installed={layout?.installed !== false}
          dirtyPaths={new Set(allDirtyPaths)}
          onPick={(p) => {
            // Switching files no longer discards: the edit is stashed
            // into the pending map by setActivePath and reappears when
            // the user comes back.
            setActivePath(p);
          }}
          canToggle={kind === "apache"}
          toggleBusy={toggleBusy}
          onToggle={handleToggle}
          t={t}
        />
        {(kind === "caddy" || kind === "apache") &&
        mode === "tree" &&
        activePath &&
        !openBusy &&
        !openError ? (
          <div className="ws-raw__editor">
            <div className="ws-raw__editor-head mono">
              <span>{activePath}</span>
              <span className="ws-raw__mode-hint">
                {isDirty
                ? t("(unsaved — tree shows draft)")
                : t("(editable — changes update the buffer)")}
              </span>
            </div>
            {kind === "caddy" ? (
              <CaddyTreeView
                content={editorValue}
                onChange={handleTreeChange}
              />
            ) : (
              <ApacheTreeView
                content={editorValue}
                onChange={handleTreeChange}
              />
            )}
          </div>
        ) : (kind === "caddy" || kind === "apache") &&
          mode === "features" &&
          activePath &&
          !openBusy &&
          !openError ? (
          <div className="ws-raw__editor">
            <div className="ws-raw__editor-head mono">
              <span>{activePath}</span>
              <span className="ws-raw__mode-hint">
                {featuresApplying
                  ? t("Applying…")
                  : isDirty
                    ? t("(unsaved — toggle to commit to buffer)")
                    : ""}
              </span>
            </div>
            {featuresParseError && (
              <div className="status-note mono status-note--error">
                {featuresParseError}
              </div>
            )}
            {featuresAst &&
              (kind === "caddy" ? (
                <CaddyFeatureCatalog
                  nodes={featuresAst as CaddyNode[]}
                  onChange={(next) => void handleFeatureChange(next)}
                />
              ) : (
                <ApacheFeatureCatalog
                  nodes={featuresAst as ApacheNode[]}
                  onChange={(next) => void handleFeatureChange(next)}
                />
              ))}
          </div>
        ) : kind === "apache" &&
          mode === "ifmodule" &&
          activePath &&
          !openBusy &&
          !openError ? (
          <div className="ws-raw__editor">
            <div className="ws-raw__editor-head mono">
              <span>{activePath}</span>
              <span className="ws-raw__mode-hint">
                {featuresApplying
                  ? t("Applying…")
                  : isDirty
                    ? t("(unsaved — edits commit to buffer)")
                    : ""}
              </span>
            </div>
            {featuresParseError && (
              <div className="status-note mono status-note--error">
                {featuresParseError}
              </div>
            )}
            {featuresAst && (
              <ApacheIfModuleEditor
                nodes={featuresAst as ApacheNode[]}
                onChange={(next) =>
                  void handleFeatureChange(next as ApacheNode[])
                }
              />
            )}
          </div>
        ) : kind === "caddy" &&
          mode === "matchers" &&
          activePath &&
          !openBusy &&
          !openError ? (
          <div className="ws-raw__editor">
            <div className="ws-raw__editor-head mono">
              <span>{activePath}</span>
              <span className="ws-raw__mode-hint">
                {featuresApplying
                  ? t("Applying…")
                  : isDirty
                    ? t("(unsaved — edits commit to buffer)")
                    : ""}
              </span>
            </div>
            {featuresParseError && (
              <div className="status-note mono status-note--error">
                {featuresParseError}
              </div>
            )}
            {featuresAst && (
              <CaddyMatcherEditor
                nodes={featuresAst as CaddyNode[]}
                onChange={(next) =>
                  void handleFeatureChange(next as CaddyNode[])
                }
              />
            )}
          </div>
        ) : (
          <Editor
            activePath={activePath}
            openBusy={openBusy}
            openError={openError}
            value={editorValue}
            onChange={setDirty}
            dirty={isDirty}
            t={t}
          />
        )}
      </div>

      {showDiff && isDirty && content !== null && (
        <DiffPreview oldText={content} newText={editorValue} />
      )}

      <StatusBar
        actionError={actionError}
        validateResult={validateResult}
        reloadResult={reloadResult}
        saveResult={saveResult}
        batchResult={batchResult}
        lintResult={lintResult}
        t={t}
      />

      {newSiteOpen && (kind === "apache" || kind === "caddy") && (
        <NewWebServerSite
          kind={kind}
          sshParams={sshParams}
          onClose={() => setNewSiteOpen(false)}
          onCreated={async (path) => {
            await refreshLayout();
            setActivePath(path);
          }}
        />
      )}
    </div>
  );
}

function FileTree({
  files,
  activePath,
  loading,
  error,
  installed,
  onPick,
  canToggle,
  toggleBusy,
  onToggle,
  dirtyPaths,
  t,
}: {
  files: WebServerFile[];
  activePath: string | null;
  loading: boolean;
  error: string;
  installed: boolean;
  onPick: (path: string) => void;
  canToggle: boolean;
  toggleBusy: string | null;
  onToggle: (f: WebServerFile) => void;
  dirtyPaths: Set<string>;
  t: (s: string) => string;
}) {
  // Group by section.
  const sections = useMemo(() => {
    const main = files.filter((f) => f.kind.kind === "main");
    const confd = files.filter((f) => f.kind.kind === "conf-d");
    const sites = files.filter((f) => f.kind.kind === "site-available");
    const other = files.filter((f) => f.kind.kind === "other");
    return { main, confd, sites, other };
  }, [files]);

  return (
    <div className="ws-raw__tree">
      {loading && (
        <div className="status-note mono">{t("Reading file…")}</div>
      )}
      {!loading && error && (
        <div className="status-note mono status-note--error">{error}</div>
      )}
      {!loading && !error && !installed && (
        <div className="status-note mono">
          {t("Not installed on this host.")}
        </div>
      )}
      {!loading && !error && installed && files.length === 0 && (
        <div className="status-note mono">
          {t("(no config files discovered)")}
        </div>
      )}

      {sections.main.length > 0 && (
        <FileGroup
          title={t("Main")}
          files={sections.main}
          activePath={activePath}
          onPick={onPick}
          canToggle={false}
          toggleBusy={toggleBusy}
          onToggle={onToggle}
          dirtyPaths={dirtyPaths}
        />
      )}
      {sections.confd.length > 0 && (
        <FileGroup
          title={t("Includes")}
          files={sections.confd}
          activePath={activePath}
          onPick={onPick}
          canToggle={false}
          toggleBusy={toggleBusy}
          onToggle={onToggle}
          dirtyPaths={dirtyPaths}
        />
      )}
      {sections.sites.length > 0 && (
        <FileGroup
          title={t("Sites")}
          files={sections.sites}
          activePath={activePath}
          onPick={onPick}
          canToggle={canToggle}
          toggleBusy={toggleBusy}
          onToggle={onToggle}
          dirtyPaths={dirtyPaths}
        />
      )}
      {sections.other.length > 0 && (
        <FileGroup
          title={t("Other")}
          files={sections.other}
          activePath={activePath}
          onPick={onPick}
          canToggle={false}
          toggleBusy={toggleBusy}
          onToggle={onToggle}
          dirtyPaths={dirtyPaths}
        />
      )}
    </div>
  );
}

function FileGroup({
  title,
  files,
  activePath,
  onPick,
  canToggle,
  toggleBusy,
  onToggle,
  dirtyPaths,
}: {
  title: string;
  files: WebServerFile[];
  activePath: string | null;
  onPick: (path: string) => void;
  canToggle: boolean;
  toggleBusy: string | null;
  onToggle: (f: WebServerFile) => void;
  dirtyPaths: Set<string>;
}) {
  return (
    <div className="ws-raw__group">
      <div className="ws-raw__group-title mono">{title}</div>
      {files.map((f) => {
        const isSite = f.kind.kind === "site-available";
        const enabled = isSite && (f.kind as { enabled: boolean }).enabled;
        const isDirty = dirtyPaths.has(f.path);
        return (
          <div
            key={f.path}
            className={`ws-raw__file ${f.path === activePath ? "is-active" : ""} ${
              isSite && !enabled ? "is-disabled" : ""
            }`}
          >
            <button
              type="button"
              className="ws-raw__file-name mono"
              onClick={() => onPick(f.path)}
              title={isDirty ? `${f.path} · modified` : f.path}
            >
              <FileText size={10} /> {f.label}
              {isDirty && <span className="ws-raw__file-dirty">●</span>}
            </button>
            {isSite && canToggle && (
              <button
                type="button"
                className="ws-raw__toggle"
                onClick={() => onToggle(f)}
                disabled={toggleBusy === f.path}
                title={enabled ? "a2dissite" : "a2ensite"}
              >
                {enabled ? (
                  <ToggleRight size={12} />
                ) : (
                  <ToggleLeft size={12} />
                )}
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

function Editor({
  activePath,
  openBusy,
  openError,
  value,
  onChange,
  dirty,
  t,
}: {
  activePath: string | null;
  openBusy: boolean;
  openError: string;
  value: string;
  onChange: (v: string) => void;
  dirty: boolean;
  t: (s: string) => string;
}) {
  const ref = useRef<HTMLTextAreaElement | null>(null);

  if (!activePath) {
    return (
      <div className="ws-raw__editor ws-raw__editor--empty">
        <div className="status-note mono">
          {t("Pick a config file on the left to start editing.")}
        </div>
      </div>
    );
  }
  if (openBusy) {
    return (
      <div className="ws-raw__editor ws-raw__editor--empty">
        <div className="status-note mono">{t("Reading file…")}</div>
      </div>
    );
  }
  if (openError) {
    return (
      <div className="ws-raw__editor ws-raw__editor--empty">
        <div className="status-note mono status-note--error">{openError}</div>
      </div>
    );
  }
  return (
    <div className="ws-raw__editor">
      <div className="ws-raw__editor-head mono">
        <span>{activePath}</span>
        {dirty && <span className="ws-raw__dirty">●</span>}
      </div>
      <textarea
        ref={ref}
        className="ws-raw__textarea mono"
        value={value}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function StatusBar({
  actionError,
  validateResult,
  reloadResult,
  saveResult,
  batchResult,
  lintResult,
  t,
}: {
  actionError: string;
  validateResult: WebServerActionResult | null;
  reloadResult: WebServerActionResult | null;
  saveResult: WebServerSaveResult | null;
  batchResult: cmd.WebServerBatchSaveResult | null;
  lintResult: WebServerActionResult | null;
  t: (s: string) => string;
}) {
  if (
    !actionError &&
    !validateResult &&
    !reloadResult &&
    !saveResult &&
    !batchResult &&
    !lintResult
  ) {
    return null;
  }
  return (
    <div className="ws-raw__status">
      {actionError && (
        <div className="ws-raw__status-line is-bad mono">
          <AlertTriangle size={11} /> {actionError}
        </div>
      )}
      {validateResult && (
        <ResultLine
          label={t("Validate")}
          ok={validateResult.ok}
          exit={validateResult.exitCode}
          output={validateResult.output}
        />
      )}
      {reloadResult && (
        <ResultLine
          label={t("Reload")}
          ok={reloadResult.ok}
          exit={reloadResult.exitCode}
          output={reloadResult.output}
        />
      )}
      {saveResult && <SaveLine result={saveResult} t={t} />}
      {batchResult && <BatchSaveLine result={batchResult} t={t} />}
      {lintResult && (
        <ResultLine
          label={t("Lint")}
          ok={lintResult.ok}
          exit={lintResult.exitCode}
          output={lintResult.output || t("(no warnings)")}
        />
      )}
    </div>
  );
}

function BatchSaveLine({
  result,
  t,
}: {
  result: cmd.WebServerBatchSaveResult;
  t: (
    s: string,
    vars?: Record<string, string | number | null | undefined>,
  ) => string;
}) {
  const ok = result.validate.ok && result.reloaded;
  const restoreFails = result.restoreErrors.filter((e) => e.length > 0).length;
  const summary = ok
    ? t("Save all · {n} files written, validate + reload OK", {
        n: result.backupPaths.length,
      })
    : result.validate.ok
      ? t(
          "Save all · {n} files written, reload failed (config still valid)",
          { n: result.backupPaths.length },
        )
      : restoreFails === 0
        ? t("Save all · validate failed, all {n} backups restored", {
            n: result.backupPaths.length,
          })
        : t(
            "Save all · validate failed and {fails}/{n} restore steps had errors",
            { fails: restoreFails, n: result.backupPaths.length },
          );
  return (
    <ResultLine
      label={t("Save all")}
      ok={ok}
      exit={result.validate.exitCode}
      output={
        result.validate.output
          ? `${summary}\n${result.validate.output}`
          : summary
      }
    />
  );
}

function ResultLine({
  label,
  ok,
  exit,
  output,
}: {
  label: string;
  ok: boolean;
  exit: number;
  output: string;
}) {
  return (
    <div className={`ws-raw__status-line ${ok ? "is-ok" : "is-bad"} mono`}>
      {ok ? <CheckCircle2 size={11} /> : <AlertTriangle size={11} />}
      <span>
        {label} · exit {exit}
      </span>
      {output && <pre className="ws-raw__status-output">{output}</pre>}
    </div>
  );
}

function SaveLine({
  result,
  t,
}: {
  result: WebServerSaveResult;
  t: (s: string) => string;
}) {
  const ok = result.validate.ok && result.reloaded;
  let summary: string;
  if (result.validate.ok && result.reloaded) {
    summary = t("Saved · validated · reloaded.");
  } else if (result.validate.ok && !result.reloaded) {
    summary = t("Saved + validated, but reload failed.");
  } else if (result.restored) {
    summary = t("Save aborted — validation failed; original restored.");
  } else {
    summary = t("Save aborted — validation failed AND restore failed.");
  }
  return (
    <div className={`ws-raw__status-line ${ok ? "is-ok" : "is-bad"} mono`}>
      {ok ? <CheckCircle2 size={11} /> : <AlertTriangle size={11} />}
      <span>{summary}</span>
      {result.backupPath && (
        <span className="ws-raw__backup">
          {t("Backup at {path}").replace("{path}", result.backupPath)}
        </span>
      )}
      {result.validate.output && (
        <pre className="ws-raw__status-output">{result.validate.output}</pre>
      )}
      {result.reloadOutput && !result.reloaded && (
        <pre className="ws-raw__status-output">{result.reloadOutput}</pre>
      )}
      {result.restoreError && (
        <pre className="ws-raw__status-output">{result.restoreError}</pre>
      )}
    </div>
  );
}
