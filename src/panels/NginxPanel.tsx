import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Code2,
  Diff as DiffIcon,
  Eye,
  EyeOff,
  FileCode,
  FilePlus2,
  FileText,
  Folder,
  Link2,
  Plus,
  RefreshCw,
  RotateCw,
  Save,
  ShieldCheck,
  Sparkles,
  ToggleRight,
  X,
} from "lucide-react";
import DiffPreview from "../components/DiffPreview";
import { useEffect, useMemo, useRef, useState } from "react";

import * as cmd from "../lib/commands";
import type {
  NginxFile,
  NginxLayout,
  NginxNode,
  NginxReadFileResult,
  NginxSaveResult,
  NginxValidateResult,
} from "../lib/commands";
import NginxIcon from "../components/icons/NginxIcon";
import FeatureCatalog from "./NginxFeatureCatalog";
import {
  COMMON_DIRECTIVES,
  newBlockDirective,
  newDirective,
} from "./nginxFeatures";
import PanelHeader from "../components/PanelHeader";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import {
  effectiveSshTarget,
  isSshTargetReady,
  type TabState,
} from "../lib/types";

type Props = { tab: TabState | null };

export default function NginxPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? (
        <NginxPanelBody {...props} />
      ) : (
        <PanelSkeleton variant="rows" rows={9} />
      )}
    </div>
  );
}

function NginxPanelBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);

  const sshTarget = tab ? effectiveSshTarget(tab) : null;
  const canProbe = isSshTargetReady(sshTarget);

  const sshParams = useMemo(() => {
    if (!sshTarget) return null;
    return {
      host: sshTarget.host,
      port: sshTarget.port,
      user: sshTarget.user,
      authMode: sshTarget.authMode,
      password: sshTarget.password,
      keyPath: sshTarget.keyPath,
      savedConnectionIndex: sshTarget.savedConnectionIndex,
    };
  }, [
    sshTarget?.host,
    sshTarget?.port,
    sshTarget?.user,
    sshTarget?.authMode,
    sshTarget?.password,
    sshTarget?.keyPath,
    sshTarget?.savedConnectionIndex,
  ]);

  const [layout, setLayout] = useState<NginxLayout | null>(null);
  const [layoutBusy, setLayoutBusy] = useState(false);
  const [layoutError, setLayoutError] = useState("");
  const [activePath, setActivePath] = useState<string | null>(null);
  const [opened, setOpened] = useState<NginxReadFileResult | null>(null);
  const [openedDirty, setOpenedDirty] = useState<string | null>(null);
  const [openBusy, setOpenBusy] = useState(false);
  const [openError, setOpenError] = useState("");
  const [saveResult, setSaveResult] = useState<NginxSaveResult | null>(null);
  const [saveBusy, setSaveBusy] = useState(false);
  const [validateResult, setValidateResult] =
    useState<NginxValidateResult | null>(null);
  const [validateBusy, setValidateBusy] = useState(false);
  const [reloadResult, setReloadResult] =
    useState<NginxValidateResult | null>(null);
  const [reloadBusy, setReloadBusy] = useState(false);
  const [lintResult, setLintResult] =
    useState<NginxValidateResult | null>(null);
  const [lintBusy, setLintBusy] = useState(false);
  /** "features" → curated feature catalog (toggle gzip/HSTS/HTTP2/etc.);
   *  "structured" → directive cards; "raw" → plain textarea editing the
   *  file content. All three round-trip through the AST so a save from
   *  any mode is well-formed. */
  const [editMode, setEditMode] = useState<"features" | "structured" | "raw">(
    "features",
  );
  /** Standalone-comment cards default off — heavy banner comments
   *  in stock nginx.conf (`##\n# Basic Settings\n##`) push the real
   *  directives off-screen otherwise. Comments are still preserved
   *  in the AST and round-tripped on save. */
  const [showComments, setShowComments] = useState(false);

  async function refreshLayout() {
    if (!sshParams || !canProbe || layoutBusy) return;
    setLayoutBusy(true);
    setLayoutError("");
    try {
      const result = await cmd.nginxLayout(sshParams);
      setLayout(result);
      // Auto-pick the main config on first load if the user hasn't
      // selected anything yet.
      if (!activePath && result.installed) {
        const main = result.files.find((f) => f.kind.kind === "main");
        if (main) setActivePath(main.path);
      }
    } catch (e) {
      setLayoutError(formatError(e));
    } finally {
      setLayoutBusy(false);
    }
  }

  // Probe on host change.
  useEffect(() => {
    if (!sshParams || !canProbe) return;
    void refreshLayout();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sshParams?.host, sshParams?.port, sshParams?.user, canProbe]);

  // Load the active file.
  useEffect(() => {
    if (!sshParams || !activePath) {
      setOpened(null);
      setOpenedDirty(null);
      setSaveResult(null);
      return;
    }
    let cancelled = false;
    setOpenBusy(true);
    setOpenError("");
    cmd
      .nginxReadFile({ ...sshParams, path: activePath })
      .then((result) => {
        if (cancelled) return;
        setOpened(result);
        setOpenedDirty(null);
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
  }, [sshParams?.host, sshParams?.port, activePath]);

  if (!tab) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <NginxIcon size={12} /> {t("Nginx")}
        </div>
        <div className="status-note mono">
          {t("Open an SSH tab to manage Nginx config.")}
        </div>
      </div>
    );
  }
  if (!sshTarget) {
    return (
      <div className="panel-section panel-section--empty">
        <div className="panel-section__title mono">
          <NginxIcon size={12} /> {t("Nginx")}
        </div>
        <div className="status-note mono">
          {t("This tab has no SSH context — Nginx management is remote-only.")}
        </div>
      </div>
    );
  }

  const headerMeta = layout
    ? layout.installed
      ? `${sshTarget.user}@${sshTarget.host} · ${layout.version || "nginx"}`
      : `${sshTarget.user}@${sshTarget.host} · ${t("nginx not installed")}`
    : `${sshTarget.user}@${sshTarget.host}`;

  // Re-render the active file from a freshly-edited node tree.
  // Used when the structured editor wants to update the dirty buffer.
  const handleNodesChange = (nodes: NginxNode[]) => {
    if (!opened) return;
    const text = renderNodes(nodes);
    setOpenedDirty(text);
    setOpened({
      ...opened,
      parse: { ...opened.parse, nodes },
    });
  };

  const handleSave = async () => {
    if (!sshParams || !opened || saveBusy) return;
    const content = openedDirty ?? opened.content;
    setSaveBusy(true);
    try {
      const result = await cmd.nginxSaveFile({
        ...sshParams,
        path: opened.path,
        content,
      });
      setSaveResult(result);
      // On success, snap the dirty state back to clean and re-read so
      // any nginx-side normalization is reflected.
      if (result.validate.ok && result.reloaded) {
        setOpenedDirty(null);
        const fresh = await cmd.nginxReadFile({
          ...sshParams,
          path: opened.path,
        });
        setOpened(fresh);
      }
    } catch (e) {
      setOpenError(formatError(e));
    } finally {
      setSaveBusy(false);
    }
  };

  const handleValidate = async () => {
    if (!sshParams || validateBusy) return;
    setValidateBusy(true);
    try {
      setValidateResult(await cmd.nginxValidate(sshParams));
    } catch (e) {
      setValidateResult({
        ok: false,
        exitCode: -1,
        output: formatError(e),
      });
    } finally {
      setValidateBusy(false);
    }
  };

  const handleLint = async () => {
    if (!sshParams || lintBusy) return;
    setLintBusy(true);
    try {
      const r = await cmd.webServerLintHints({ ...sshParams, kind: "nginx" });
      setLintResult({ ok: r.ok, exitCode: r.exitCode, output: r.output });
    } catch (e) {
      setLintResult({ ok: false, exitCode: -1, output: formatError(e) });
    } finally {
      setLintBusy(false);
    }
  };

  const handleReload = async () => {
    if (!sshParams || reloadBusy) return;
    setReloadBusy(true);
    try {
      setReloadResult(await cmd.nginxReload(sshParams));
    } catch (e) {
      setReloadResult({ ok: false, exitCode: -1, output: formatError(e) });
    } finally {
      setReloadBusy(false);
    }
  };

  const handleToggleSite = async (siteName: string, enable: boolean) => {
    if (!sshParams) return;
    try {
      const r = await cmd.nginxToggleSite({
        ...sshParams,
        siteName,
        enable,
      });
      if (!r.ok) {
        setLayoutError(r.output || `${enable ? "enable" : "disable"} failed`);
      }
      await refreshLayout();
    } catch (e) {
      setLayoutError(formatError(e));
    }
  };

  const handleCreate = async (
    target: "conf.d" | "sites-available",
    name: string,
  ) => {
    if (!sshParams) return;
    // Auto-add `.conf` for the conf.d bucket since nginx only loads
    // matching files there. sites-available has no extension convention.
    const leaf =
      target === "conf.d" && !name.endsWith(".conf") ? `${name}.conf` : name;
    const dir =
      target === "conf.d"
        ? "/etc/nginx/conf.d"
        : "/etc/nginx/sites-available";
    const path = `${dir}/${leaf}`;
    const content = defaultTemplateFor(target, leaf);
    setLayoutError("");
    try {
      const r = await cmd.nginxCreateFile({
        ...sshParams,
        path,
        content,
      });
      if (!r.ok) {
        setLayoutError(r.output.trim() || t("Create failed"));
        return;
      }
      await refreshLayout();
      setActivePath(path);
    } catch (e) {
      setLayoutError(formatError(e));
    }
  };

  return (
    <div className="ngx-panel">
      <PanelHeader
        icon={NginxIcon}
        title={t("Nginx")}
        meta={headerMeta}
        actions={
          <>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void refreshLayout()}
              disabled={layoutBusy || !canProbe}
              title={t("Re-scan config files")}
            >
              <RefreshCw size={10} /> {t("Refresh")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void handleValidate()}
              disabled={validateBusy || !canProbe}
              title={t("Run nginx -t against the live tree")}
            >
              <ShieldCheck size={10} /> {t("Validate")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void handleLint()}
              disabled={lintBusy || !canProbe}
              title={t(
                "Run a deeper static analysis (apachectl -S / caddy adapt --pretty / nginx -t -q)",
              )}
            >
              <Sparkles size={10} /> {lintBusy ? t("Linting…") : t("Lint")}
            </button>
            <button
              type="button"
              className="btn is-ghost is-compact"
              onClick={() => void handleReload()}
              disabled={reloadBusy || !canProbe}
              title={t("systemctl reload nginx")}
            >
              <RotateCw size={10} /> {t("Reload")}
            </button>
          </>
        }
      />

      {layoutError && (
        <div className="status-note status-note--error mono ngx-panel__error">
          {layoutError}
        </div>
      )}

      {layout && !layout.installed && (
        <div className="status-note status-note--error mono ngx-panel__error">
          {t(
            "nginx is not installed on this host. Use the Software panel to install it.",
          )}
        </div>
      )}

      {validateResult && (
        <ValidationBanner result={validateResult} t={t} kind="validate" />
      )}
      {reloadResult && (
        <ValidationBanner result={reloadResult} t={t} kind="reload" />
      )}
      {lintResult && (
        <ValidationBanner result={lintResult} t={t} kind="lint" />
      )}
      {saveResult && <SaveResultBanner result={saveResult} t={t} />}

      <div className="ngx-panel__body">
        <FileTree
          layout={layout}
          activePath={activePath}
          onSelect={setActivePath}
          onToggleSite={handleToggleSite}
          onCreate={handleCreate}
          t={t}
        />
        <div className="ngx-panel__editor">
          {!activePath ? (
            <div className="status-note mono">
              {t("Pick a config file on the left to start editing.")}
            </div>
          ) : openBusy ? (
            <div className="status-note mono">{t("Reading file…")}</div>
          ) : openError ? (
            <div className="status-note status-note--error mono">
              {openError}
            </div>
          ) : opened ? (
            <Editor
              file={opened}
              dirtyContent={openedDirty}
              setDirtyContent={setOpenedDirty}
              editMode={editMode}
              setEditMode={setEditMode}
              showComments={showComments}
              setShowComments={setShowComments}
              onNodesChange={handleNodesChange}
              onSave={handleSave}
              saveBusy={saveBusy}
              t={t}
            />
          ) : null}
        </div>
      </div>

      <ModulesSection layout={layout} t={t} />
    </div>
  );
}

function FileTree({
  layout,
  activePath,
  onSelect,
  onToggleSite,
  onCreate,
  t,
}: {
  layout: NginxLayout | null;
  activePath: string | null;
  onSelect: (path: string) => void;
  onToggleSite: (siteName: string, enable: boolean) => void | Promise<void>;
  /** Called when the user submits a new-file form. The string is the
   *  filename portion (with extension where applicable); the parent
   *  joins it onto the section's directory. */
  onCreate: (
    target: "conf.d" | "sites-available",
    name: string,
  ) => void | Promise<void>;
  t: ReturnType<typeof useI18n>["t"];
}) {
  if (!layout || !layout.installed) {
    return <div className="ngx-tree ngx-tree--empty" />;
  }
  // Bucket files by section so the tree mirrors the on-disk layout.
  const main = layout.files.filter((f) => f.kind.kind === "main");
  const confd = layout.files.filter((f) => f.kind.kind === "conf-d");
  const sites = layout.files.filter(
    (f) => f.kind.kind === "site-available",
  );
  const orphans = layout.files.filter(
    (f) => f.kind.kind === "site-enabled-orphan",
  );

  return (
    <div className="ngx-tree">
      <FileTreeSection title="nginx.conf" files={main} activePath={activePath} onSelect={onSelect} t={t} />
      <FileTreeSection
        title="conf.d"
        files={confd}
        activePath={activePath}
        onSelect={onSelect}
        createKind="conf.d"
        onCreate={onCreate}
        t={t}
      />
      <FileTreeSection
        title="sites-available"
        files={sites}
        activePath={activePath}
        onSelect={onSelect}
        createKind="sites-available"
        onCreate={onCreate}
        t={t}
        renderTrailing={(file) => {
          if (file.kind.kind !== "site-available") return null;
          const enabled = file.kind.enabled;
          return (
            <button
              type="button"
              className={`ngx-tree__toggle ${enabled ? "is-on" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                void onToggleSite(file.name, !enabled);
              }}
              title={
                enabled
                  ? t("Disable this site (rm sites-enabled link)")
                  : t("Enable this site (ln -sf into sites-enabled)")
              }
            >
              <Link2 size={10} />
              {enabled ? t("enabled") : t("disabled")}
            </button>
          );
        }}
      />
      {orphans.length > 0 && (
        <FileTreeSection
          title="sites-enabled (orphans)"
          files={orphans}
          activePath={activePath}
          onSelect={onSelect}
          t={t}
        />
      )}
    </div>
  );
}

function FileTreeSection({
  title,
  files,
  activePath,
  onSelect,
  renderTrailing,
  createKind,
  onCreate,
  t,
}: {
  title: string;
  files: NginxFile[];
  activePath: string | null;
  onSelect: (path: string) => void;
  renderTrailing?: (file: NginxFile) => React.ReactNode;
  /** When set, the section header gets a `+` button that opens an
   *  inline name input. The kind drives the default-content template
   *  the parent picks. */
  createKind?: "conf.d" | "sites-available";
  onCreate?: (
    target: "conf.d" | "sites-available",
    name: string,
  ) => void | Promise<void>;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const canCreate = !!createKind && !!onCreate;

  if (files.length === 0 && !canCreate) return null;

  const submit = () => {
    const name = draft.trim();
    if (!name || !createKind || !onCreate) return;
    void onCreate(createKind, name);
    setCreating(false);
    setDraft("");
  };

  return (
    <div className="ngx-tree__section">
      <div className="ngx-tree__section-title mono">
        <Folder size={10} />
        <span style={{ flex: 1 }}>{title}</span>
        {canCreate && !creating && (
          <button
            type="button"
            className="ngx-tree__add"
            onClick={() => {
              setCreating(true);
              setDraft("");
            }}
            title={
              createKind === "conf.d"
                ? t("New .conf file in conf.d")
                : t("New site in sites-available")
            }
          >
            <Plus size={10} />
          </button>
        )}
      </div>
      {creating && (
        <div className="ngx-tree__create">
          <FilePlus2 size={11} />
          <input
            className="ngx-input mono ngx-input--inline"
            value={draft}
            spellCheck={false}
            autoFocus
            placeholder={
              createKind === "conf.d" ? "mysite.conf" : "example.com"
            }
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submit();
              } else if (e.key === "Escape") {
                e.preventDefault();
                setCreating(false);
                setDraft("");
              }
            }}
          />
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={!draft.trim()}
            onClick={submit}
          >
            {t("Create")}
          </button>
          <button
            type="button"
            className="icon-btn"
            onClick={() => {
              setCreating(false);
              setDraft("");
            }}
            title={t("Cancel")}
          >
            <X size={10} />
          </button>
        </div>
      )}
      {files.map((f) => {
        const isActive = f.path === activePath;
        return (
          <button
            key={f.path}
            type="button"
            className={`ngx-tree__row ${isActive ? "is-active" : ""}`}
            onClick={() => onSelect(f.path)}
            title={f.path}
          >
            <FileCode size={11} />
            <span className="ngx-tree__name">{f.name}</span>
            {renderTrailing?.(f)}
          </button>
        );
      })}
    </div>
  );
}

function ValidationBanner({
  result,
  kind,
  t,
}: {
  result: NginxValidateResult;
  kind: "validate" | "reload" | "lint";
  t: ReturnType<typeof useI18n>["t"];
}) {
  const ok = result.ok;
  const label =
    kind === "validate"
      ? t("nginx -t")
      : kind === "reload"
        ? t("reload")
        : t("Lint");
  return (
    <div
      className={`ngx-banner ${ok ? "is-ok" : "is-bad"}`}
      role={ok ? "status" : "alert"}
    >
      <div className="ngx-banner__head mono">
        {ok ? (
          <ShieldCheck size={12} />
        ) : (
          <AlertTriangle size={12} />
        )}
        {ok
          ? t("{label} OK", { label })
          : t("{label} failed (exit {code})", {
              label,
              code: String(result.exitCode),
            })}
      </div>
      {result.output && (
        <pre className="ngx-banner__output mono">{result.output.trim()}</pre>
      )}
    </div>
  );
}

function SaveResultBanner({
  result,
  t,
}: {
  result: NginxSaveResult;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const validateOk = result.validate.ok;
  const reloaded = result.reloaded;
  const cls = validateOk && reloaded ? "is-ok" : "is-bad";
  return (
    <div className={`ngx-banner ${cls}`} role={validateOk ? "status" : "alert"}>
      <div className="ngx-banner__head mono">
        {validateOk && reloaded ? (
          <ShieldCheck size={12} />
        ) : (
          <AlertTriangle size={12} />
        )}
        {validateOk && reloaded
          ? t("Saved · validated · reloaded.")
          : !validateOk
            ? t("Save aborted — `nginx -t` failed; original restored.")
            : t("Saved + validated, but reload failed.")}
      </div>
      {!validateOk && result.validate.output && (
        <pre className="ngx-banner__output mono">
          {result.validate.output.trim()}
        </pre>
      )}
      {validateOk && !reloaded && result.reloadOutput && (
        <pre className="ngx-banner__output mono">
          {result.reloadOutput.trim()}
        </pre>
      )}
      {result.restoreError && (
        <div className="status-note status-note--error mono">
          {t("Restore from backup failed: {err}", {
            err: result.restoreError,
          })}{" "}
          ({t("Backup at {path}", { path: result.backupPath })})
        </div>
      )}
    </div>
  );
}

function Editor({
  file,
  dirtyContent,
  setDirtyContent,
  editMode,
  setEditMode,
  showComments,
  setShowComments,
  onNodesChange,
  onSave,
  saveBusy,
  t,
}: {
  file: NginxReadFileResult;
  dirtyContent: string | null;
  setDirtyContent: (s: string | null) => void;
  editMode: "features" | "structured" | "raw";
  setEditMode: (m: "features" | "structured" | "raw") => void;
  showComments: boolean;
  setShowComments: (b: boolean) => void;
  onNodesChange: (nodes: NginxNode[]) => void;
  onSave: () => void | Promise<void>;
  saveBusy: boolean;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const dirty = dirtyContent !== null;
  const [showDiff, setShowDiff] = useState(false);
  return (
    <div className="ngx-editor">
      <div className="ngx-editor__head">
        <div className="ngx-editor__path mono" title={file.path}>
          {file.path}
          {dirty && <span className="ngx-editor__dirty"> · {t("modified")}</span>}
        </div>
        <div className="ngx-editor__modes">
          <button
            type="button"
            className={`btn is-compact ${editMode === "features" ? "is-primary" : "is-ghost"}`}
            onClick={() => setEditMode("features")}
            title={t("Toggle common features (TLS, HSTS, gzip, …)")}
          >
            <ToggleRight size={10} /> {t("Features")}
          </button>
          <button
            type="button"
            className={`btn is-compact ${editMode === "structured" ? "is-primary" : "is-ghost"}`}
            onClick={() => setEditMode("structured")}
            title={t("Edit as cards / forms")}
          >
            <FileText size={10} /> {t("Structured")}
          </button>
          <button
            type="button"
            className={`btn is-compact ${editMode === "raw" ? "is-primary" : "is-ghost"}`}
            onClick={() => setEditMode("raw")}
            title={t("Edit raw text")}
          >
            <Code2 size={10} /> {t("Raw")}
          </button>
          {editMode === "structured" && (
            <button
              type="button"
              className={`btn is-compact ${showComments ? "is-primary" : "is-ghost"}`}
              onClick={() => setShowComments(!showComments)}
              title={
                showComments
                  ? t("Hide standalone comment cards")
                  : t("Show standalone comment cards")
              }
            >
              {showComments ? <Eye size={10} /> : <EyeOff size={10} />}
              {t("Comments")}
            </button>
          )}
          <button
            type="button"
            className={`btn is-compact ${showDiff ? "is-primary" : "is-ghost"}`}
            onClick={() => setShowDiff((v) => !v)}
            disabled={!dirty}
            title={t("Preview diff against the on-disk version")}
          >
            <DiffIcon size={10} /> {t("Diff")}
          </button>
          <button
            type="button"
            className="btn is-primary is-compact"
            disabled={!dirty || saveBusy}
            onClick={() => void onSave()}
            title={t("Backup → write → nginx -t → reload")}
          >
            <Save size={10} />
            {saveBusy ? t("Saving…") : t("Save")}
          </button>
        </div>
      </div>

      {showDiff && dirty && (
        <DiffPreview oldText={file.content} newText={dirtyContent ?? ""} />
      )}

      {file.parse.errors.length > 0 && (
        <div className="status-note status-note--error mono">
          {t("Parse warnings:")} {file.parse.errors.join("; ")}
        </div>
      )}

      {editMode === "features" ? (
        <FeatureCatalog
          nodes={file.parse.nodes}
          onChange={onNodesChange}
        />
      ) : editMode === "structured" ? (
        <StructuredEditor
          nodes={file.parse.nodes}
          showComments={showComments}
          onChange={onNodesChange}
          t={t}
        />
      ) : (
        <RawEditor
          value={dirtyContent ?? file.content}
          onChange={(v) => setDirtyContent(v === file.content ? null : v)}
        />
      )}
    </div>
  );
}

function RawEditor({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <textarea
      className="ngx-raw mono"
      value={value}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

// ── Structured editor ────────────────────────────────────────────

function StructuredEditor({
  nodes,
  showComments,
  onChange,
  t,
}: {
  nodes: NginxNode[];
  showComments: boolean;
  onChange: (nodes: NginxNode[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const updateChild = (idx: number, next: NginxNode) => {
    const copy = nodes.slice();
    copy[idx] = next;
    onChange(copy);
  };
  // Filter at render time, not at array level — keeping the index
  // stable means `updateChild(idx, …)` still maps onto the underlying
  // AST array correctly when the user edits a directive.
  const visible = nodes
    .map((n, i) => ({ n, i }))
    .filter(({ n }) => showComments || n.kind !== "comment");
  return (
    <div className="ngx-tree-cards">
      {visible.map(({ n, i }) => (
        <NodeCard
          key={i}
          node={n}
          path={[i]}
          showComments={showComments}
          onChange={(next) => updateChild(i, next)}
          t={t}
        />
      ))}
      {nodes.length > 0 && visible.length === 0 && (
        <div className="status-note mono">
          {t("(only comments in this file — toggle Comments to show)")}
        </div>
      )}
      <AddDirectiveBar
        onAdd={(d) => onChange([...nodes, d])}
        t={t}
      />
    </div>
  );
}

function AddDirectiveBar({
  onAdd,
  t,
}: {
  onAdd: (d: NginxNode) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [args, setArgs] = useState("");
  const [block, setBlock] = useState(false);
  const [blockTouched, setBlockTouched] = useState(false);

  const reset = () => {
    setName("");
    setArgs("");
    setBlock(false);
    setBlockTouched(false);
  };
  const cancel = () => {
    reset();
    setOpen(false);
  };
  const submit = () => {
    const n = name.trim();
    if (!n) return;
    const a = splitArgs(args);
    onAdd(block ? newBlockDirective(n, a) : newDirective(n, a));
    reset();
    setOpen(false);
  };
  const onPickName = (next: string) => {
    setName(next);
    if (!blockTouched) {
      const known = COMMON_DIRECTIVES.find((d) => d.name === next);
      if (known) setBlock(known.block);
    }
  };

  if (!open) {
    return (
      <button
        type="button"
        className="ngx-add-directive__btn"
        onClick={() => setOpen(true)}
      >
        <Plus size={11} /> {t("Add directive")}
      </button>
    );
  }

  return (
    <div className="ngx-add-directive">
      <input
        className="ngx-input mono ngx-add-directive__name"
        list="ngx-common-directives"
        value={name}
        spellCheck={false}
        autoFocus
        placeholder={t("name (e.g. listen)")}
        onChange={(e) => onPickName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          else if (e.key === "Escape") cancel();
        }}
      />
      <datalist id="ngx-common-directives">
        {COMMON_DIRECTIVES.map((d) => (
          <option key={d.name} value={d.name} />
        ))}
      </datalist>
      <input
        className="ngx-input mono ngx-add-directive__args"
        value={args}
        spellCheck={false}
        placeholder={t("args (space-separated)")}
        onChange={(e) => setArgs(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          else if (e.key === "Escape") cancel();
        }}
      />
      <label className="ngx-add-directive__flag">
        <input
          type="checkbox"
          checked={block}
          onChange={(e) => {
            setBlock(e.target.checked);
            setBlockTouched(true);
          }}
        />
        {t("block")}
      </label>
      <button
        type="button"
        className="ngx-add-directive__ok"
        onClick={submit}
        disabled={!name.trim()}
      >
        {t("Add")}
      </button>
      <button
        type="button"
        className="ngx-add-directive__cancel"
        onClick={cancel}
      >
        {t("Cancel")}
      </button>
    </div>
  );
}

/** Render a single AST node as a card. Block directives expand into a
 *  nested set of NodeCards. The `path` is reserved for future
 *  identity-based optimizations; nothing reads it today. */
function NodeCard({
  node,
  path,
  showComments,
  onChange,
  t,
}: {
  node: NginxNode;
  path: number[];
  showComments: boolean;
  onChange: (next: NginxNode) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  if (node.kind === "comment") {
    return (
      <div className="ngx-card ngx-card--comment">
        <div className="ngx-card__head mono">
          <FileText size={10} /> # {node.text}
        </div>
      </div>
    );
  }

  // Directive
  const isBlock = node.block !== null || node.opaqueBody !== null;
  const summary =
    node.args.length > 0 ? node.args.join(" ") : t("(no args)");

  return (
    <DirectiveCard
      node={node}
      path={path}
      summary={summary}
      isBlock={isBlock}
      showComments={showComments}
      onChange={onChange}
      t={t}
    />
  );
}

function DirectiveCard({
  node,
  path,
  summary,
  isBlock,
  showComments,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  path: number[];
  summary: string;
  isBlock: boolean;
  showComments: boolean;
  onChange: (next: NginxNode) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  // Top-level / shallow blocks open by default; deep nesting collapses
  // so the panel doesn't render a wall of cards on first paint.
  const [open, setOpen] = useState(path.length <= 2);

  const updateArgs = (args: string[]) => {
    onChange({ ...node, args });
  };

  const updateBlock = (block: NginxNode[]) => {
    onChange({ ...node, block });
  };

  // Pure-block directive (events / http / server / location-without-
  // path-yet etc.) → no point showing an empty `(args)` input row.
  // Keep the inputs for the directives that have a fine-grained form
  // (LocationHeaderForm needs the inputs even when args are empty).
  const HAS_FINE_FORM = new Set([
    "listen",
    "server_name",
    "root",
    "proxy_pass",
    "ssl_certificate",
    "ssl_certificate_key",
    "location",
    "upstream",
  ]);
  const showFormBody =
    HAS_FINE_FORM.has(node.name) ||
    node.args.length > 0 ||
    (!isBlock && node.opaqueBody === null);

  return (
    <div className="ngx-card">
      <button
        type="button"
        className="ngx-card__head"
        onClick={() => setOpen((cur) => !cur)}
      >
        {isBlock ? (
          open ? (
            <ChevronDown size={11} />
          ) : (
            <ChevronRight size={11} />
          )
        ) : (
          <span style={{ width: 11, display: "inline-block" }} />
        )}
        <span className="ngx-card__name mono">{node.name}</span>
        <span className="ngx-card__summary mono">{summary}</span>
      </button>

      {open && (
        <div className="ngx-card__body">
          {showFormBody && (
            <DirectiveForm node={node} onArgsChange={updateArgs} t={t} />
          )}

          {node.opaqueBody !== null && (
            <div className="ngx-card__lua">
              <div className="ngx-card__field-label mono">
                {t("Lua / njs body (read-only here — edit in Raw mode)")}
              </div>
              <pre className="ngx-card__lua-body mono">{node.opaqueBody}</pre>
            </div>
          )}

          {node.block !== null && (
            <div className="ngx-card__children">
              {node.block
                .map((child, i) => ({ child, i }))
                .filter(
                  ({ child }) => showComments || child.kind !== "comment",
                )
                .map(({ child, i }) => (
                  <NodeCard
                    key={i}
                    node={child}
                    path={[...path, i]}
                    showComments={showComments}
                    onChange={(next) => {
                      const copy = node.block!.slice();
                      copy[i] = next;
                      updateBlock(copy);
                    }}
                    t={t}
                  />
                ))}
              <AddDirectiveBar
                onAdd={(d) => updateBlock([...node.block!, d])}
                t={t}
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** Pick a fine-grained form for the high-frequency directives the user
 *  asked for; fall back to a generic args row otherwise. */
function DirectiveForm({
  node,
  onArgsChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onArgsChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  switch (node.name) {
    case "listen":
      return <ListenForm node={node} onChange={onArgsChange} t={t} />;
    case "server_name":
      return <ServerNameForm node={node} onChange={onArgsChange} t={t} />;
    case "root":
    case "proxy_pass":
    case "ssl_certificate":
    case "ssl_certificate_key":
      return <SinglePathForm node={node} onChange={onArgsChange} t={t} />;
    case "location":
      return <LocationHeaderForm node={node} onChange={onArgsChange} t={t} />;
    case "upstream":
      return <UpstreamHeaderForm node={node} onChange={onArgsChange} t={t} />;
    default:
      return <GenericArgsForm node={node} onChange={onArgsChange} />;
  }
}

function GenericArgsForm({
  node,
  onChange,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
}) {
  const { t } = useI18n();
  // One row, space-joined args, parsed back on change. Quoting on
  // round-trip is handled by the renderer so unquoted "foo bar" survives.
  const [text, setText] = useState(node.args.join(" "));
  // Sync local text when external args change (e.g. file reload).
  const lastExternal = useRef(node.args.join(" "));
  useEffect(() => {
    const ext = node.args.join(" ");
    if (ext !== lastExternal.current) {
      setText(ext);
      lastExternal.current = ext;
    }
  }, [node.args]);
  return (
    <input
      className="ngx-input mono"
      value={text}
      spellCheck={false}
      onChange={(e) => {
        const next = e.target.value;
        setText(next);
        onChange(splitArgs(next));
      }}
      placeholder={t("(args)")}
    />
  );
}

function ListenForm({
  node,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  // First arg is host:port or just port; rest are flags (`ssl`, `http2`,
  // `default_server`, `reuseport`, …). Pull port out for the dedicated
  // input, treat everything else as a flag set.
  const [bind, ...rest] = node.args;
  const flags = new Set(rest.map((s) => s.toLowerCase()));
  const ssl = flags.has("ssl");
  const http2 = flags.has("http2");
  const defaultServer = flags.has("default_server");

  const update = (next: { bind?: string; ssl?: boolean; http2?: boolean; defaultServer?: boolean }) => {
    const newBind = next.bind ?? bind ?? "80";
    const newFlags = new Set(rest.filter(
      (s) =>
        !["ssl", "http2", "default_server"].includes(s.toLowerCase()),
    ));
    if ((next.ssl ?? ssl)) newFlags.add("ssl");
    if ((next.http2 ?? http2)) newFlags.add("http2");
    if ((next.defaultServer ?? defaultServer)) newFlags.add("default_server");
    onChange([newBind, ...Array.from(newFlags)]);
  };

  return (
    <div className="ngx-form">
      <label className="ngx-form__field">
        <span className="ngx-form__label">{t("Listen on")}</span>
        <input
          className="ngx-input mono"
          value={bind ?? ""}
          spellCheck={false}
          onChange={(e) => update({ bind: e.target.value })}
          placeholder="80 / 443 / [::]:80 / 192.168.1.1:8080"
        />
      </label>
      <div className="ngx-form__flags">
        <label className="ngx-form__flag">
          <input
            type="checkbox"
            checked={ssl}
            onChange={(e) => update({ ssl: e.target.checked })}
          />
          ssl
        </label>
        <label className="ngx-form__flag">
          <input
            type="checkbox"
            checked={http2}
            onChange={(e) => update({ http2: e.target.checked })}
          />
          http2
        </label>
        <label className="ngx-form__flag">
          <input
            type="checkbox"
            checked={defaultServer}
            onChange={(e) => update({ defaultServer: e.target.checked })}
          />
          default_server
        </label>
      </div>
    </div>
  );
}

function ServerNameForm({
  node,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  // Comma- or space-separated hosts. Render as a chip list with an
  // input to append; remove on chip click.
  const [draft, setDraft] = useState("");
  const add = () => {
    const v = draft.trim();
    if (!v) return;
    onChange([...node.args, v]);
    setDraft("");
  };
  return (
    <div className="ngx-form">
      <span className="ngx-form__label">{t("Hostnames")}</span>
      <div className="ngx-form__chips">
        {node.args.map((h, i) => (
          <button
            key={`${h}-${i}`}
            type="button"
            className="ngx-chip mono"
            onClick={() =>
              onChange(node.args.filter((_, j) => j !== i))
            }
            title={t("Remove")}
          >
            {h} ×
          </button>
        ))}
        <input
          className="ngx-input mono ngx-input--inline"
          value={draft}
          spellCheck={false}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              add();
            }
          }}
          placeholder="example.com"
        />
        <button
          type="button"
          className="btn is-ghost is-compact"
          onClick={add}
          disabled={!draft.trim()}
        >
          {t("Add")}
        </button>
      </div>
    </div>
  );
}

function SinglePathForm({
  node,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  const value = node.args[0] ?? "";
  return (
    <label className="ngx-form__field">
      <span className="ngx-form__label">{t("Value")}</span>
      <input
        className="ngx-input mono"
        value={value}
        spellCheck={false}
        onChange={(e) => onChange([e.target.value])}
        placeholder="/var/www/html"
      />
      {node.args.length > 1 && (
        <span className="ngx-form__hint mono">
          {t("(extra args preserved on save: {extra})", {
            extra: node.args.slice(1).join(" "),
          })}
        </span>
      )}
    </label>
  );
}

function LocationHeaderForm({
  node,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  // `location [modifier] uri`. Modifier is one of `=`, `~`, `~*`, `^~`
  // or empty. uri is the next non-modifier arg.
  const MODS = ["", "=", "~", "~*", "^~"] as const;
  const first = node.args[0] ?? "";
  const isMod = (MODS as readonly string[]).includes(first);
  const modifier = isMod ? first : "";
  const path = isMod ? node.args[1] ?? "" : first;

  const update = (next: { modifier?: string; path?: string }) => {
    const m = next.modifier ?? modifier;
    const p = next.path ?? path;
    if (m) onChange([m, p]);
    else onChange([p]);
  };

  return (
    <div className="ngx-form">
      <label className="ngx-form__field">
        <span className="ngx-form__label">{t("Match")}</span>
        <select
          className="ngx-input mono"
          value={modifier}
          onChange={(e) => update({ modifier: e.target.value })}
        >
          <option value="">{t("(prefix)")}</option>
          <option value="=">{t("= (exact)")}</option>
          <option value="^~">{t("^~ (prefix, no regex)")}</option>
          <option value="~">{t("~ (regex, case-sensitive)")}</option>
          <option value="~*">{t("~* (regex, case-insensitive)")}</option>
        </select>
      </label>
      <label className="ngx-form__field ngx-form__field--grow">
        <span className="ngx-form__label">{t("Path")}</span>
        <input
          className="ngx-input mono"
          value={path}
          spellCheck={false}
          onChange={(e) => update({ path: e.target.value })}
          placeholder="/api/"
        />
      </label>
    </div>
  );
}

function UpstreamHeaderForm({
  node,
  onChange,
  t,
}: {
  node: Extract<NginxNode, { kind: "directive" }>;
  onChange: (args: string[]) => void;
  t: ReturnType<typeof useI18n>["t"];
}) {
  // `upstream <name>` — single arg. Members live in the block's
  // `server` directives; we don't synthesize a member editor here
  // because that's a nested block that gets its own card recursively.
  const value = node.args[0] ?? "";
  return (
    <label className="ngx-form__field">
      <span className="ngx-form__label">{t("Upstream name")}</span>
      <input
        className="ngx-input mono"
        value={value}
        spellCheck={false}
        onChange={(e) => onChange([e.target.value])}
        placeholder="backend"
      />
      <span className="ngx-form__hint mono">
        {t("Members are the `server …;` directives inside the block.")}
      </span>
    </label>
  );
}

function ModulesSection({
  layout,
  t,
}: {
  layout: NginxLayout | null;
  t: ReturnType<typeof useI18n>["t"];
}) {
  if (!layout || !layout.installed || layout.builtinModules.length === 0) {
    return null;
  }
  return (
    <details className="ngx-modules">
      <summary className="ngx-modules__summary mono">
        {t("Built-in modules ({n})", {
          n: String(layout.builtinModules.length),
        })}
      </summary>
      <div className="ngx-modules__list">
        {layout.builtinModules.map((m) => (
          <span key={m} className="ngx-modules__chip mono">
            {m}
          </span>
        ))}
      </div>
      <div className="ngx-modules__hint mono">
        {t(
          "To install extras (e.g. headers-more, geoip2), use the Software panel — packages like nginx-extras or distro-equivalents.",
        )}
      </div>
    </details>
  );
}

// ── Local renderers / parsers ────────────────────────────────────
//
// We mirror the backend's render() in TypeScript so the structured
// editor can rebuild the file content on every keystroke without an
// IPC round-trip. Save still goes through the backend so the parser /
// renderer agree on round-trip; this client copy is purely an
// optimization for the editor's "dirty preview" buffer.

function renderNodes(nodes: NginxNode[], depth = 0): string {
  let out = "";
  for (const n of nodes) {
    if (n.kind === "comment") {
      for (let i = 0; i < Math.min(n.leadingBlanks, 2); i++) out += "\n";
      out += "    ".repeat(depth) + "#";
      if (n.text && !n.text.startsWith(" ")) out += " ";
      out += n.text + "\n";
    } else {
      out += renderDirective(n, depth);
    }
  }
  return out;
}

function renderDirective(
  d: Extract<NginxNode, { kind: "directive" }>,
  depth: number,
): string {
  let out = "";
  for (let i = 0; i < Math.min(d.leadingBlanks, 2); i++) out += "\n";
  for (const c of d.leadingComments) {
    out += "    ".repeat(depth) + "#";
    if (c && !c.startsWith(" ")) out += " ";
    out += c + "\n";
  }
  out += "    ".repeat(depth) + d.name;
  for (const a of d.args) {
    out += " ";
    out += needsQuoting(a) ? `"${a.replace(/(["\\])/g, "\\$1")}"` : a;
  }
  if (d.opaqueBody !== null) {
    out += " {" + d.opaqueBody + "}";
    if (d.inlineComment) {
      out += " #";
      if (!d.inlineComment.startsWith(" ")) out += " ";
      out += d.inlineComment;
    }
    out += "\n";
    return out;
  }
  if (d.block !== null) {
    out += " {";
    if (d.inlineComment) {
      out += " #";
      if (!d.inlineComment.startsWith(" ")) out += " ";
      out += d.inlineComment;
    }
    out += "\n";
    out += renderNodes(d.block, depth + 1);
    out += "    ".repeat(depth) + "}\n";
    return out;
  }
  out += ";";
  if (d.inlineComment) {
    out += " #";
    if (!d.inlineComment.startsWith(" ")) out += " ";
    out += d.inlineComment;
  }
  out += "\n";
  return out;
}

function needsQuoting(arg: string): boolean {
  if (arg.length === 0) return true;
  const f = arg[0];
  const l = arg[arg.length - 1];
  if ((f === '"' && l === '"') || (f === "'" && l === "'")) return false;
  return /[\s;{}#"']/.test(arg);
}

/** Stub content for newly-created config files. We bias toward
 *  templates that pass `nginx -t` immediately (no listen-port
 *  collisions, no missing root paths) so the user's first save is a
 *  noop validation, not a debugging session. */
function defaultTemplateFor(
  target: "conf.d" | "sites-available",
  leafName: string,
): string {
  if (target === "sites-available") {
    return `# ${leafName} — created by Pier-X
# Edit listen / server_name / root then save to validate + reload.
server {
    listen 8080;
    server_name ${leafName.replace(/[^a-zA-Z0-9.-]/g, "_")};
    root /var/www/html;
    index index.html index.htm;

    location / {
        try_files $uri $uri/ =404;
    }
}
`;
  }
  // conf.d — typically used for snippets that get included into the
  // global http context. Empty file passes `nginx -t` cleanly.
  return `# ${leafName} — created by Pier-X
# Add directives below; this file is included into the http context.
`;
}

/** Crude quote-aware split for the GenericArgsForm. Splits on
 *  whitespace, but keeps double / single-quoted segments together
 *  with quotes preserved (so the renderer doesn't strip them). */
function splitArgs(s: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (quote) {
      cur += c;
      if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'") {
      cur += c;
      quote = c as '"' | "'";
      continue;
    }
    if (/\s/.test(c)) {
      if (cur) {
        out.push(cur);
        cur = "";
      }
      continue;
    }
    cur += c;
  }
  if (cur) out.push(cur);
  return out;
}
