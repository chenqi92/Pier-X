import { useState } from "react";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";

export default function MarkdownPanel() {
  const { t } = useI18n();
  const [source, setSource] = useState("");
  const [html, setHtml] = useState("");
  const [filePath, setFilePath] = useState("");

  function renderLive() {
    if (!source.trim()) { setHtml(""); return; }
    cmd.markdownRender(source).then(setHtml).catch(() => {});
  }

  async function renderFile() {
    if (!filePath.trim()) return;
    try { setHtml(await cmd.markdownRenderFile(filePath.trim())); }
    catch (e) { setHtml(`<p style="color:var(--status-error)">${String(e)}</p>`); }
  }

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Markdown")}</span></div>
        <div className="form-stack">
          <label className="field-stack">
            <span className="field-label">{t("Load from file")}</span>
            <div className="branch-row">
              <input className="field-input" onChange={(e) => setFilePath(e.currentTarget.value)} placeholder="/path/to/README.md" value={filePath} />
              <button className="mini-button" disabled={!filePath.trim()} onClick={() => void renderFile()} type="button">Load</button>
            </div>
          </label>
          <label className="field-stack">
            <span className="field-label">{t("Or type Markdown")}</span>
            <textarea className="field-textarea field-textarea--editor" onChange={(e) => setSource(e.currentTarget.value)} placeholder="# Hello" rows={6} value={source} />
          </label>
          <button className="mini-button" onClick={renderLive} type="button">{t("Render")}</button>
        </div>
      </section>

      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Rendered Output")}</span></div>
        {html ? (
          <div className="markdown-preview" dangerouslySetInnerHTML={{ __html: html }} />
        ) : (
          <div className="empty-note">Type or load Markdown content.</div>
        )}
      </section>
    </div>
  );
}
