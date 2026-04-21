import { FileText } from "lucide-react";
import { useEffect, useState } from "react";
import PanelHeader from "../components/PanelHeader";
import * as cmd from "../lib/commands";
import { useI18n } from "../i18n/useI18n";

type Props = {
  filePath?: string;
};

function basename(p: string): string {
  if (!p) return "";
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(i + 1) : p;
}

export default function MarkdownPanel({ filePath }: Props) {
  const { t } = useI18n();
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
        setError(String(err));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  return (
    <>
      <PanelHeader
        icon={FileText}
        title="MARKDOWN"
        meta={filePath ? basename(filePath) : undefined}
      />
      <div className="panel-scroll">
        <section className="panel-section">
          {!filePath ? (
            <div className="empty-note">{t("Select a Markdown file on the left to preview.")}</div>
          ) : loading ? (
            <div className="empty-note">{t("Rendering…")}</div>
          ) : error ? (
            <div className="empty-note empty-note--error">{error}</div>
          ) : (
            <div className="markdown-preview" dangerouslySetInnerHTML={{ __html: html }} />
          )}
        </section>
      </div>
    </>
  );
}
