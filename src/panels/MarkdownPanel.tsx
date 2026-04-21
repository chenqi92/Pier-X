import { useEffect, useState } from "react";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";
import { localizeError } from "../i18n/localizeMessage";

type Props = {
  filePath?: string;
};

export default function MarkdownPanel({ filePath }: Props) {
  const { t } = useI18n();
  const formatError = (error: unknown) => localizeError(error, t);
  const [html, setHtml] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    const path = filePath?.trim() ?? "";
    if (!path) {
      setHtml("");
      setError("");
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError("");
    cmd.markdownRenderFile(path)
      .then((rendered) => {
        if (cancelled) return;
        setHtml(rendered);
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        setHtml("");
        setError(formatError(err));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        {!filePath ? (
          <div className="empty-note">{t("Select a Markdown file on the left to preview.")}</div>
        ) : loading ? (
          <div className="empty-note">{t("Rendering…")}</div>
        ) : error ? (
          <div className="empty-note empty-note--error">{error}</div>
        ) : (
          <div className="markdown-preview ux-selectable" dangerouslySetInnerHTML={{ __html: html }} />
        )}
      </section>
    </div>
  );
}
