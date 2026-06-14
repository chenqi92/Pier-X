// Code Search panel (M8).
//
// Runs `rg` (preferred) or `git grep` (fallback) over the active
// SSH session at the terminal's last cwd, and renders matches as
// a flat file-grouped list. Clicking a hit opens the SFTP file
// editor at that path; the editor's built-in CodeMirror search +
// "Go to line" carry the user the rest of the way.

import {
  Filter,
  Folder,
  Loader2,
  Search,
  X,
  Regex,
  CaseSensitive,
  WholeWord,
  FileSearch,
  FileText,
  TerminalSquare,
  Download,
} from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";

import * as cmd from "../lib/commands";
import type {
  CodeSearchHit,
  CodeSearchMode,
  CodeSearchOutput,
} from "../lib/commands";
import { RIGHT_TOOL_META } from "../lib/rightToolMeta";
import type { TabState } from "../lib/types";
import { effectiveShellUser, effectiveSshTarget, isSshTargetReady } from "../lib/types";
import { useTabStore } from "../stores/useTabStore";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";
import PanelHeader from "../components/PanelHeader";
import PanelSkeleton, { useDeferredMount } from "../components/PanelSkeleton";
import { sudoKeyFor, useSudoStore } from "../stores/useSudoStore";
import "../styles/code-search-panel.css";

const SEARCH_ICON = RIGHT_TOOL_META.search.icon;
const SftpEditorDialog = lazy(() => import("../components/SftpEditorDialog"));

type Props = { tab: TabState };

export default function CodeSearchPanel(props: Props) {
  const ready = useDeferredMount();
  return (
    <div className="panel-stage">
      {ready ? <CodeSearchBody {...props} /> : <PanelSkeleton variant="rows" rows={8} />}
    </div>
  );
}

type GroupedFile = {
  file: string;
  hits: CodeSearchHit[];
};

function groupHitsByFile(hits: CodeSearchHit[]): GroupedFile[] {
  const out: GroupedFile[] = [];
  const idxByFile = new Map<string, number>();
  for (const h of hits) {
    const key = h.file;
    const idx = idxByFile.get(key);
    if (idx === undefined) {
      idxByFile.set(key, out.length);
      out.push({ file: key, hits: [h] });
    } else {
      out[idx].hits.push(h);
    }
  }
  return out;
}

function joinPath(root: string, rel: string): string {
  if (!rel) return root;
  if (rel.startsWith("/")) return rel;
  if (!root) return rel;
  return root.endsWith("/") ? `${root}${rel}` : `${root}/${rel}`;
}

function basename(path: string): string {
  if (!path) return "";
  const i = path.lastIndexOf("/");
  return i < 0 ? path : path.slice(i + 1);
}

function CodeSearchBody({ tab }: Props) {
  const { t } = useI18n();
  const formatError = (e: unknown) => localizeError(e, t);

  const sshTarget = effectiveSshTarget(tab);
  const ready = isSshTargetReady(sshTarget);

  // Default cwd: the persisted `lastCwd` from the terminal panel.
  // Empty falls through to the backend's `$HOME` resolver.
  const [cwd, setCwd] = useState<string>(tab.lastCwd ?? "");
  useEffect(() => {
    // Re-seed when the tab's cwd changes externally — but only when
    // the user hasn't typed an override yet (empty / matches the
    // previous value).
    setCwd((cur) =>
      cur === "" || cur === (tab.lastCwd ?? "") ? tab.lastCwd ?? "" : cur,
    );
  }, [tab.lastCwd]);

  const [mode, setMode] = useState<CodeSearchMode>("content");
  const [query, setQuery] = useState("");
  const [caseInsensitive, setCaseInsensitive] = useState(true);
  const [regex, setRegex] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [glob, setGlob] = useState("");

  // Per-mode chrome. Content greps file bodies; FileName matches
  // paths; Command locates an executable on $PATH (cwd-independent).
  const showCwd = mode !== "command";
  const showGlob = mode === "content";
  const showToggles = mode !== "command";
  const modeTabs: { id: CodeSearchMode; label: string; icon: typeof Search }[] = [
    { id: "content", label: t("Content"), icon: FileSearch },
    { id: "filename", label: t("File name"), icon: FileText },
    { id: "command", label: t("Command"), icon: TerminalSquare },
  ];
  const placeholder =
    mode === "filename"
      ? t("Find files by name, e.g. nginx.conf")
      : mode === "command"
        ? t("Locate a command, e.g. python3")
        : t("Search the terminal's working directory…");

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<CodeSearchOutput | null>(null);
  const searchSeqRef = useRef(0);

  const inputRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Editor target: opens the SftpEditorDialog at a hit's file. Path
  // is absolute (we resolve `cwd + relative`); the dialog reads the
  // bytes via SFTP and renders in CodeMirror.
  const [editorTarget, setEditorTarget] = useState<{
    path: string;
    name: string;
    line: number;
    column: number;
  } | null>(null);

  const grouped = useMemo(
    () => (result ? groupHitsByFile(result.hits) : []),
    [result],
  );

  const codeSearchSudoPw = useSudoStore((s) =>
    sshTarget
      ? s.passwords[sudoKeyFor({
          host: sshTarget.host,
          port: sshTarget.port,
          user: sshTarget.user,
          authMode: sshTarget.authMode,
          password: sshTarget.password,
          keyPath: sshTarget.keyPath,
          savedConnectionIndex: sshTarget.savedConnectionIndex,
        })] ?? null
      : null,
  );
  const sshArgs = {
    host: sshTarget?.host ?? "",
    port: sshTarget?.port ?? 22,
    user: sshTarget?.user ?? "",
    authMode: sshTarget?.authMode ?? "password",
    password: sshTarget?.password ?? "",
    keyPath: sshTarget?.keyPath ?? "",
    savedConnectionIndex: sshTarget?.savedConnectionIndex ?? null,
    sudoPassword: codeSearchSudoPw,
  };

  async function runSearch() {
    if (!ready || !sshTarget) {
      setError(t("SSH connection required."));
      return;
    }
    const trimmed = query.trim();
    if (!trimmed) {
      setError(t("Type a query first."));
      return;
    }
    const seq = searchSeqRef.current + 1;
    searchSeqRef.current = seq;
    setBusy(true);
    setError("");
    setResult(null);
    try {
      const out = await cmd.codeSearch({
        ...sshArgs,
        cwd: showCwd ? cwd.trim() : "",
        query: trimmed,
        mode,
        caseInsensitive,
        regex,
        wholeWord,
        glob: showGlob ? glob.trim() : "",
        maxHits: 500,
      });
      if (seq !== searchSeqRef.current) return;
      setResult(out);
      // `none` (no search tool found) is surfaced as a clickable
      // install CTA below, not a blocking error. Real failures stay
      // errors.
      if (out.engine === "cwd-missing") {
        setError(
          t("Working directory does not exist on the remote: {cwd}", {
            cwd: cwd || "$HOME",
          }),
        );
      } else if (
        out.exitCode !== 0 &&
        out.exitCode !== 1 &&
        out.hits.length === 0
      ) {
        setError(
          t("Search engine exited with code {code}.", { code: out.exitCode }),
        );
      }
    } catch (e) {
      if (seq !== searchSeqRef.current) return;
      setResult(null);
      setError(formatError(e));
    } finally {
      if (seq === searchSeqRef.current) setBusy(false);
    }
  }

  function openHit(hit: CodeSearchHit) {
    const root = result?.cwd || cwd;
    if (!root && !hit.file.startsWith("/")) {
      setError(
        t(
          "Cannot resolve absolute path for this hit — open the file via SFTP first.",
        ),
      );
      return;
    }
    const absolute = joinPath(root, hit.file);
    setEditorTarget({
      path: absolute,
      name: basename(hit.file),
      line: hit.line || 1,
      column: hit.column || 1,
    });
  }

  function gotoSoftware() {
    if (sshTarget) useTabStore.getState().setTabRightTool(tab.id, "software");
  }

  const ENGINE_LABELS: Partial<Record<CodeSearchOutput["engine"], string>> = {
    rg: "rg",
    "git-grep": "git grep",
    grep: "grep",
    fd: "fd",
    "rg-files": "rg --files",
    find: "find",
    command: "command -v",
  };
  const engineLabel = result ? ENGINE_LABELS[result.engine] ?? null : null;

  // A clickable "install a faster tool" nudge. Shown when the search
  // fell back to a slow engine (plain grep / find) or found no tool
  // at all — the CTA jumps to the Software panel to install it.
  const nudge = !result
    ? null
    : result.engine === "none"
      ? {
          text:
            mode === "filename"
              ? t("No file-search tool found — install fd or ripgrep.")
              : t("No search tool found — install ripgrep."),
          cta: t("Install"),
          tone: "error" as const,
        }
      : result.engine === "grep"
        ? {
            text: t("Using grep — install ripgrep for faster search."),
            cta: t("Install ripgrep"),
            tone: "warn" as const,
          }
        : result.engine === "find"
          ? {
              text: t("Using find — install fd for faster search."),
              cta: t("Install fd"),
              tone: "warn" as const,
            }
          : null;

  const headerMeta = result
    ? t("{n} hits", { n: result.hits.length }) +
      (result.truncated ? " · " + t("truncated") : "")
    : undefined;

  return (
    <>
      <PanelHeader
        icon={SEARCH_ICON}
        title={t("Code Search")}
        meta={headerMeta}
      />
      <div className="cs">
        <div className="cs-modes" role="tablist" aria-label={t("Search mode")}>
          {modeTabs.map((m) => {
            const Icon = m.icon;
            return (
              <button
                key={m.id}
                type="button"
                role="tab"
                aria-selected={mode === m.id}
                className={"cs-mode" + (mode === m.id ? " on" : "")}
                onClick={() => setMode(m.id)}
                disabled={busy}
              >
                <Icon size={12} />
                {m.label}
              </button>
            );
          })}
        </div>
        <div className="cs-toolbar">
          <div className="cs-input-wrap">
            <Search size={12} className="cs-input-icon" aria-hidden="true" />
            <input
              ref={inputRef}
              type="text"
              className="dlg-input mono cs-input"
              spellCheck={false}
              placeholder={placeholder}
              value={query}
              onChange={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void runSearch();
                }
              }}
              disabled={busy}
            />
            {query ? (
              <button
                type="button"
                className="cs-input-clear"
                onClick={() => setQuery("")}
                aria-label={t("Clear")}
              >
                <X size={11} />
              </button>
            ) : null}
          </div>
          {showToggles ? (
            <div className="cs-toggles">
              <button
                type="button"
                className={"cs-toggle" + (caseInsensitive ? "" : " on")}
                onClick={() => setCaseInsensitive((v) => !v)}
                title={t("Case sensitive")}
                disabled={busy}
              >
                <CaseSensitive size={12} />
              </button>
              {mode === "content" ? (
                <button
                  type="button"
                  className={"cs-toggle" + (wholeWord ? " on" : "")}
                  onClick={() => setWholeWord((v) => !v)}
                  title={t("Whole word")}
                  disabled={busy}
                >
                  <WholeWord size={12} />
                </button>
              ) : null}
              <button
                type="button"
                className={"cs-toggle" + (regex ? " on" : "")}
                onClick={() => setRegex((v) => !v)}
                title={t("Regex")}
                disabled={busy}
              >
                <Regex size={12} />
              </button>
            </div>
          ) : null}
          <button
            type="button"
            className="btn is-primary is-compact"
            onClick={() => void runSearch()}
            disabled={busy || !ready || !query.trim()}
          >
            {busy ? <Loader2 size={11} className="cs-spin" /> : <Search size={11} />}
            {busy ? t("Searching…") : t("Search")}
          </button>
        </div>

        {showCwd ? (
          <div className="cs-cwd-row">
            <Folder size={11} className="muted" aria-hidden="true" />
            <input
              type="text"
              className="dlg-input mono cs-cwd-input"
              spellCheck={false}
              placeholder="$HOME"
              value={cwd}
              onChange={(e) => setCwd(e.currentTarget.value)}
              disabled={busy}
              aria-label={t("Working directory")}
            />
            {engineLabel ? (
              <span className="cs-engine-badge mono" title={t("Search engine")}>
                {engineLabel}
              </span>
            ) : null}
          </div>
        ) : engineLabel ? (
          <div className="cs-cwd-row cs-meta-row">
            <TerminalSquare size={11} className="muted" aria-hidden="true" />
            <span className="cs-meta-label muted">{t("Located on $PATH")}</span>
            <span className="cs-engine-badge mono" title={t("Search engine")}>
              {engineLabel}
            </span>
          </div>
        ) : null}

        {showGlob ? (
          <div className="cs-cwd-row">
            <Filter size={11} className="muted" aria-hidden="true" />
            <input
              type="text"
              className="dlg-input mono cs-cwd-input"
              spellCheck={false}
              placeholder={t("Glob filter, e.g. src/**/*.ts")}
              value={glob}
              onChange={(e) => setGlob(e.currentTarget.value)}
              disabled={busy}
              aria-label={t("Glob filter")}
            />
            {glob ? (
              <button
                type="button"
                className="cs-input-clear cs-inline-clear"
                onClick={() => setGlob("")}
                aria-label={t("Clear glob filter")}
              >
                <X size={11} />
              </button>
            ) : null}
          </div>
        ) : null}

        {error ? (
          <div className="status-note status-note--error mono cs-error">
            {error}
          </div>
        ) : null}

        {nudge ? (
          <div
            className={
              "status-note mono cs-nudge" +
              (nudge.tone === "error"
                ? " status-note--error"
                : " status-note--warn")
            }
          >
            <span className="cs-nudge-text">{nudge.text}</span>
            <button
              type="button"
              className="cs-nudge-cta"
              onClick={gotoSoftware}
            >
              <Download size={11} />
              {nudge.cta}
            </button>
          </div>
        ) : null}

        {!ready ? (
          <div className="empty-note">{t("SSH connection required.")}</div>
        ) : !result && !busy ? (
          <div className="empty-note">
            {t(
              "Type a query and press Enter to search the terminal's current directory.",
            )}
          </div>
        ) : busy ? (
          <div className="empty-note cs-busy">
            <Loader2 size={14} className="cs-spin" /> {t("Searching…")}
          </div>
        ) : result && result.hits.length === 0 && !error ? (
          <div className="empty-note">{t("No matches.")}</div>
        ) : null}

        {result && result.hits.length > 0 ? (
          <div className="cs-results">
            {mode === "content"
              ? grouped.map((group) => (
                  <div key={group.file} className="cs-file">
                    <div className="cs-file-head mono" title={group.file}>
                      <span className="cs-file-path">{group.file}</span>
                      <span className="cs-file-count muted">
                        {group.hits.length}
                      </span>
                    </div>
                    <div className="cs-file-hits">
                      {group.hits.map((hit, i) => (
                        <button
                          key={`${group.file}:${hit.line}:${hit.column}:${i}`}
                          type="button"
                          className="cs-hit mono"
                          onClick={() => openHit(hit)}
                          title={t("Open in SFTP editor")}
                        >
                          <span className="cs-hit-loc muted">
                            {hit.line}
                            {hit.column ? `:${hit.column}` : ""}
                          </span>
                          <span className="cs-hit-text">{hit.text}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                ))
              : result.hits.map((hit, i) => {
                  const slash = hit.file.lastIndexOf("/");
                  const dir = slash < 0 ? "" : hit.file.slice(0, slash + 1);
                  const base = slash < 0 ? hit.file : hit.file.slice(slash + 1);
                  return (
                    <button
                      key={`${hit.file}:${i}`}
                      type="button"
                      className="cs-path mono"
                      onClick={() => openHit(hit)}
                      title={t("Open in SFTP editor")}
                    >
                      {dir ? <span className="cs-path-dir muted">{dir}</span> : null}
                      <span className="cs-path-base">{base}</span>
                    </button>
                  );
                })}
            {result.truncated ? (
              <div className="status-note status-note--warn mono cs-truncated">
                {t(
                  "Results truncated at {n} hits — refine your query or scope.",
                  { n: result.hits.length },
                )}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      {editorTarget && (
        <Suspense fallback={null}>
          <SftpEditorDialog
            open
            path={editorTarget.path}
            name={editorTarget.name}
            initialLine={editorTarget.line}
            initialColumn={editorTarget.column}
            sshArgs={sshArgs}
            ownerLabel={effectiveShellUser(tab, sshTarget) || sshArgs.user}
            onClose={() => setEditorTarget(null)}
          />
        </Suspense>
      )}
    </>
  );
}
