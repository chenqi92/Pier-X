import { GitBranch, History } from "lucide-react";
import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import type { GitChangeEntry, GitCommitEntry, GitOverview, GitStashEntry } from "../lib/types";
import { useI18n } from "../i18n/useI18n";

type Props = { browserPath: string };

export default function GitPanel({ browserPath }: Props) {
  const { t } = useI18n();
  const [overview, setOverview] = useState<GitOverview | null>(null);
  const [gitError, setGitError] = useState("");
  const [selectedKey, setSelectedKey] = useState("");
  const [diffText, setDiffText] = useState("");
  const [diffLoading, setDiffLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const [commits, setCommits] = useState<GitCommitEntry[]>([]);
  const [stashes, setStashes] = useState<GitStashEntry[]>([]);
  const [commitMsg, setCommitMsg] = useState("");
  const [stashMsg, setStashMsg] = useState("");
  const [branchTarget, setBranchTarget] = useState("");
  const [notice, setNotice] = useState("");
  const [opError, setOpError] = useState("");

  function changeKey(c: GitChangeEntry) { return `${c.path}:${c.staged ? "s" : "w"}`; }

  useEffect(() => {
    if (!browserPath) return;
    let disposed = false;
    const poll = () => {
      cmd.gitOverview(browserPath).then((o) => { if (!disposed) { setOverview(o); setGitError(""); } })
        .catch((e) => { if (!disposed) { setOverview(null); setGitError(String(e)); } });
    };
    poll();
    const id = setInterval(poll, 3000);
    return () => { disposed = true; clearInterval(id); };
  }, [browserPath]);

  useEffect(() => {
    if (!browserPath || !overview) { setBranches([]); setCommits([]); setStashes([]); return; }
    Promise.all([
      cmd.gitBranchList(browserPath),
      cmd.gitRecentCommits(browserPath, 8),
      cmd.gitStashList(browserPath),
    ]).then(([b, c, s]) => {
      setBranches(b); setCommits(c); setStashes(s);
      setBranchTarget((prev) => b.includes(prev) ? prev : overview.branchName);
    }).catch(() => {});
  }, [browserPath, overview?.repoPath, overview?.branchName]);

  useEffect(() => {
    if (!overview?.changes.length) { setSelectedKey(""); return; }
    if (!selectedKey || !overview.changes.some((c) => changeKey(c) === selectedKey)) {
      setSelectedKey(changeKey(overview.changes[0]));
    }
  }, [overview]);

  const selected = overview?.changes.find((c) => changeKey(c) === selectedKey) ?? null;
  useEffect(() => {
    if (!browserPath || !selected) { setDiffText(""); return; }
    let cancelled = false;
    setDiffLoading(true);
    cmd.gitDiff(browserPath, selected.path, selected.staged)
      .then((d) => { if (!cancelled) setDiffText(d || t("No diff output.")); })
      .catch((e) => { if (!cancelled) setDiffText(String(e)); })
      .finally(() => { if (!cancelled) setDiffLoading(false); });
    return () => { cancelled = true; };
  }, [browserPath, selected?.path, selected?.staged, t]);

  async function gitOp(fn: () => Promise<unknown>, successMsg?: string) {
    setBusy(true); setOpError(""); setNotice("");
    try { await fn(); if (successMsg) setNotice(successMsg); }
    catch (e) { setOpError(String(e)); }
    finally { setBusy(false); }
  }

  if (!overview) {
    return <div className="empty-note">{gitError || t("Not a Git repository.")}</div>;
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><GitBranch size={14} /><span>{t("Repository")}</span></div>
        <ul className="stack-list">
          <li><span>{t("Branch")}</span><strong>{overview.branchName}</strong></li>
          <li><span>{t("Tracking")}</span><strong>{overview.tracking || t("local only")}</strong></li>
          <li><span>{t("Ahead / Behind")}</span><strong>{overview.ahead} / {overview.behind}</strong></li>
          <li><span>{t("Staged / Unstaged")}</span><strong>{overview.stagedCount} / {overview.unstagedCount}</strong></li>
        </ul>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><GitBranch size={14} /><span>{t("Working Tree")}</span></div>
        {overview.changes.length > 0 ? (
          <div className="git-change-list">
            {overview.changes.map((c) => (
              <button key={changeKey(c)} className={changeKey(c) === selectedKey ? "git-change-button git-change-button--selected" : "git-change-button"} onClick={() => setSelectedKey(changeKey(c))} type="button">
                <span className={c.staged ? "git-badge git-badge--staged" : "git-badge"}>{c.status}</span>
                <span className="git-change-row__path">{c.path}</span>
              </button>
            ))}
          </div>
        ) : <div className="empty-note">{overview.isClean ? t("Workspace clean.") : t("No changes.")}</div>}
      </section>

      {selected && (
        <section className="panel-section">
          <div className="panel-section__title"><GitBranch size={14} /><span>{t("Diff Preview")}</span></div>
          <div className="diff-actions">
            <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => selected.staged ? cmd.gitUnstagePaths(browserPath, [selected.path]) : cmd.gitStagePaths(browserPath, [selected.path]))} type="button">{selected.staged ? t("Unstage Selected") : t("Stage Selected")}</button>
            <button className="mini-button" disabled={busy || overview.isClean} onClick={() => void gitOp(() => cmd.gitStageAll(browserPath))} type="button">{t("Stage All")}</button>
            <button className="mini-button" disabled={busy || overview.stagedCount === 0} onClick={() => void gitOp(() => cmd.gitUnstageAll(browserPath))} type="button">{t("Unstage All")}</button>
          </div>
          {diffLoading ? <div className="empty-note">{t("Loading diff...")}</div> : <pre className="diff-viewer">{diffText}</pre>}
        </section>
      )}

      <section className="panel-section">
        <div className="panel-section__title"><GitBranch size={14} /><span>{t("Repository Actions")}</span></div>
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">{t("Switch branch")}</span>
            <div className="branch-row">
              <select className="field-input field-select" disabled={!branches.length || busy} onChange={(e) => setBranchTarget(e.currentTarget.value)} value={branchTarget}>
                {branches.map((b) => <option key={b} value={b}>{b}</option>)}
              </select>
              <button className="mini-button" disabled={!branchTarget || branchTarget === overview.branchName || busy} onClick={() => void gitOp(() => cmd.gitCheckoutBranch(browserPath, branchTarget), t("Switched to {branch}", { branch: branchTarget }))} type="button">{t("Switch")}</button>
            </div>
          </label>
          <label className="field-stack">
            <span className="field-label">{t("Commit staged changes")}</span>
            <textarea className="field-textarea" disabled={busy} onChange={(e) => setCommitMsg(e.currentTarget.value)} placeholder={overview.stagedCount > 0 ? t("Commit message") : t("Stage files first")} rows={3} value={commitMsg} />
          </label>
          <div className="button-row">
            <button className="mini-button" disabled={overview.stagedCount === 0 || !commitMsg.trim() || busy} onClick={() => void gitOp(async () => { await cmd.gitCommit(browserPath, commitMsg); setCommitMsg(""); }, t("Committed"))} type="button">{t("Commit Staged")}</button>
            <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => cmd.gitPull(browserPath), t("Pulled"))} type="button">{t("Pull")}</button>
            <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => cmd.gitPush(browserPath), t("Pushed"))} type="button">{t("Push")}</button>
          </div>
          {notice && <div className="status-note">{notice}</div>}
          {opError && <div className="status-note status-note--error">{opError}</div>}
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><GitBranch size={14} /><span>{t("Sync & Stash")}</span></div>
        <div className="form-stack">
          <div className="branch-row">
            <input className="field-input" disabled={busy} onChange={(e) => setStashMsg(e.currentTarget.value)} placeholder={t("Optional stash label")} value={stashMsg} />
            <button className="mini-button" disabled={overview.isClean || busy} onClick={() => void gitOp(async () => { await cmd.gitStashPush(browserPath, stashMsg); setStashMsg(""); }, t("Stashed"))} type="button">{t("Stash")}</button>
          </div>
          {stashes.map((s) => (
            <div className="stash-row" key={s.index}>
              <div className="stash-row__head"><span className="commit-hash">{s.index}</span><span className="inline-note">{s.relativeDate}</span></div>
              <div className="stash-row__message">{s.message || t("WIP")}</div>
              <div className="stash-row__actions">
                <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => cmd.gitStashApply(browserPath, s.index))} type="button">{t("Apply")}</button>
                <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => cmd.gitStashPop(browserPath, s.index))} type="button">{t("Pop")}</button>
                <button className="mini-button" disabled={busy} onClick={() => void gitOp(() => cmd.gitStashDrop(browserPath, s.index))} type="button">{t("Drop")}</button>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><History size={14} /><span>{t("Recent Commits")}</span></div>
        {commits.length > 0 ? (
          <div className="history-list">
            {commits.map((c) => (
              <div className="history-row" key={c.hash}>
                <div className="history-row__head"><span className="commit-hash">{c.shortHash}</span><span className="inline-note">{c.relativeDate}</span></div>
                <div className="history-row__message">{c.message}</div>
                <div className="history-row__meta">{c.author}{c.refs ? ` · ${c.refs}` : ""}</div>
              </div>
            ))}
          </div>
        ) : <div className="empty-note">{t("No commit history.")}</div>}
      </section>
    </div>
  );
}
