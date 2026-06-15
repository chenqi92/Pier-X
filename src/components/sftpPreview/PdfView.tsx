import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Loader2, ZoomIn, ZoomOut } from "lucide-react";
import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import type {
  PDFDocumentLoadingTask,
  PDFDocumentProxy,
  RenderTask,
} from "pdfjs-dist/types/src/display/api";
import * as cmd from "../../lib/commands";
import IconButton from "../IconButton";
import { useI18n } from "../../i18n/useI18n";
import type { ViewerProps } from "./types";

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

/** PDF viewer. Renders pages to a canvas via pdf.js (Mozilla), which
 *  range-fetches from the `pierfs://` URL so only the viewed page's
 *  bytes transfer. Runs decoding in a Web Worker — render stays
 *  paint-only. */
export default function PdfView({ sshArgs, path }: ViewerProps) {
  const { t } = useI18n();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const docRef = useRef<PDFDocumentProxy | null>(null);
  const loadTaskRef = useRef<PDFDocumentLoadingTask | null>(null);
  const taskRef = useRef<RenderTask | null>(null);
  const [numPages, setNumPages] = useState(0);
  const [page, setPage] = useState(1);
  const [scale, setScale] = useState(1.2);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");

  // Load the document once per file.
  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setError("");
    setPage(1);
    cmd
      .pierfsUrl(sshArgs, path)
      .then((url) => {
        if (cancelled) return undefined;
        const task = pdfjsLib.getDocument({ url });
        loadTaskRef.current = task;
        return task.promise;
      })
      .then((doc) => {
        if (cancelled || !doc) return;
        docRef.current = doc;
        setNumPages(doc.numPages);
        setStatus("ready");
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
    return () => {
      cancelled = true;
      taskRef.current?.cancel();
      void loadTaskRef.current?.destroy();
      loadTaskRef.current = null;
      docRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  // Render the current page whenever it or the zoom changes.
  useEffect(() => {
    const doc = docRef.current;
    const canvas = canvasRef.current;
    if (status !== "ready" || !doc || !canvas) return;
    let cancelled = false;
    void (async () => {
      try {
        const pdfPage = await doc.getPage(page);
        if (cancelled) return;
        const dpr = window.devicePixelRatio || 1;
        const viewport = pdfPage.getViewport({ scale: scale * dpr });
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        canvas.style.width = `${Math.floor(viewport.width / dpr)}px`;
        canvas.style.height = `${Math.floor(viewport.height / dpr)}px`;
        taskRef.current?.cancel();
        const task = pdfPage.render({ canvas, viewport });
        taskRef.current = task;
        await task.promise;
      } catch (e) {
        // Cancelled renders throw RenderingCancelledException — ignore.
        if (!cancelled && e instanceof Error && !/cancel/i.test(e.message)) {
          setError(e.message);
          setStatus("error");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [page, scale, status]);

  if (status === "loading") {
    return (
      <div className="spv-center">
        <Loader2 size={20} className="spv-spin" />
        {t("Loading PDF…")}
      </div>
    );
  }
  if (status === "error") {
    return <div className="spv-center is-error">{error}</div>;
  }

  return (
    <>
      <div className="spv-image-scroll">
        <canvas ref={canvasRef} className="spv-image" />
      </div>
      <div className="spv-statusbar">
        <IconButton
          variant="mini"
          disabled={page <= 1}
          onClick={() => setPage((p) => Math.max(1, p - 1))}
          title={t("Previous page")}
        >
          <ChevronLeft size={13} />
        </IconButton>
        <span>
          {page} / {numPages}
        </span>
        <IconButton
          variant="mini"
          disabled={page >= numPages}
          onClick={() => setPage((p) => Math.min(numPages, p + 1))}
          title={t("Next page")}
        >
          <ChevronRight size={13} />
        </IconButton>
        <span className="spv-grow" />
        <IconButton
          variant="mini"
          onClick={() => setScale((s) => Math.max(0.4, s - 0.2))}
          title={t("Zoom out")}
        >
          <ZoomOut size={13} />
        </IconButton>
        <span>{Math.round(scale * 100)}%</span>
        <IconButton
          variant="mini"
          onClick={() => setScale((s) => Math.min(4, s + 0.2))}
          title={t("Zoom in")}
        >
          <ZoomIn size={13} />
        </IconButton>
      </div>
    </>
  );
}
