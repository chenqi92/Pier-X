import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import mammoth from "mammoth";
import DOMPurify from "dompurify";
import * as cmd from "../../lib/commands";
import { useI18n } from "../../i18n/useI18n";
import type { ViewerProps } from "./types";

/** Word (.docx) viewer. mammoth converts the document to semantic
 *  HTML in the WebView; the output is sanitized with DOMPurify before
 *  injection. A readable approximation, not a print-fidelity render. */
export default function DocxView({ sshArgs, path }: ViewerProps) {
  const { t } = useI18n();
  const [html, setHtml] = useState("");
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setError("");
    cmd
      .pierfsUrl(sshArgs, path)
      .then((url) => fetch(url))
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.arrayBuffer();
      })
      .then((arrayBuffer) => mammoth.convertToHtml({ arrayBuffer }))
      .then((result) => {
        if (cancelled) return;
        setHtml(DOMPurify.sanitize(result.value));
        setStatus("ready");
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  if (status === "loading") {
    return (
      <div className="spv-center">
        <Loader2 size={20} className="spv-spin" />
        {t("Converting document…")}
      </div>
    );
  }
  if (status === "error") {
    return <div className="spv-center is-error">{error}</div>;
  }

  return (
    <div className="spv-docx-scroll">
      {/* Sanitized by DOMPurify above. */}
      <div className="spv-docx" dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}
