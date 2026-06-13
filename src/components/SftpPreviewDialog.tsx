import { Suspense, lazy, useEffect, useRef, useState } from "react";
import {
  Download,
  ExternalLink,
  FileText,
  Loader2,
  Maximize2,
  Minimize2,
  Pencil,
  X,
} from "lucide-react";
import IconButton from "./IconButton";
import { useDraggableDialog } from "./useDraggableDialog";
import { useI18n } from "../i18n/useI18n";
import { formatBytes } from "../lib/dbConnCache";
import * as cmd from "../lib/commands";
import type { PreviewKind } from "../lib/sftpEditorMeta";
import { MAX_EDITOR_BYTES } from "../lib/sftpEditorMeta";
import type { PreviewSshArgs } from "./sftpPreview/types";
import "../styles/sftp-preview.css";

const TextStreamView = lazy(() => import("./sftpPreview/TextStreamView"));
const TextCodeView = lazy(() => import("./sftpPreview/TextCodeView"));
const HexView = lazy(() => import("./sftpPreview/HexView"));
const TableView = lazy(() => import("./sftpPreview/TableView"));
const PdfView = lazy(() => import("./sftpPreview/PdfView"));
const DocxView = lazy(() => import("./sftpPreview/DocxView"));

type Props = {
  open: boolean;
  path: string;
  name: string;
  size: number;
  kind: PreviewKind;
  sshArgs: PreviewSshArgs;
  onClose: () => void;
  /** Open the same file in the editable text editor. */
  onOpenInEditor?: () => void;
  /** Save the file locally. */
  onDownload?: () => void;
  /** Hand the file to the OS default application. */
  onOpenExternal?: () => void;
};

const KIND_LABEL: Record<PreviewKind, string> = {
  image: "Image",
  svg: "SVG",
  tiff: "Image",
  pdf: "PDF",
  spreadsheet: "Spreadsheet",
  csv: "CSV",
  docx: "Word",
  video: "Video",
  audio: "Audio",
  text: "Text",
};

function Fallback() {
  return (
    <div className="spv-center">
      <Loader2 size={20} className="spv-spin" />
    </div>
  );
}

/** Read-only, instant multi-format previewer launched by double-click
 *  in the SFTP panel. Routes by file kind to a streaming text / hex /
 *  image / PDF / spreadsheet / Word / media viewer. Editing stays on
 *  the editor dialog (reachable via the Edit action). */
export default function SftpPreviewDialog({
  open,
  path,
  name,
  size,
  kind,
  sshArgs,
  onClose,
  onOpenInEditor,
  onDownload,
  onOpenExternal,
}: Props) {
  const { t } = useI18n();
  const { dialogStyle, handleProps } = useDraggableDialog(open);
  const overlayDownRef = useRef(false);
  // The text streamer may discover the file is actually binary and
  // ask us to fall back to the hex viewer.
  const [forceHex, setForceHex] = useState(false);
  const [imageActual, setImageActual] = useState(false);

  useEffect(() => {
    setForceHex(false);
    setImageActual(false);
  }, [path]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const url = cmd.pierfsUrl(sshArgs, path);
  const viewer = (() => {
    const props = { sshArgs, path, name, size };
    if (forceHex) return <HexView {...props} />;
    switch (kind) {
      case "image":
      case "svg":
      case "tiff":
        return (
          <div className="spv-image-scroll">
            <img
              className={"spv-image " + (imageActual ? "is-actual" : "is-fit")}
              src={url}
              alt={name}
            />
          </div>
        );
      case "video":
        return (
          <div className="spv-media">
            <video src={url} controls />
          </div>
        );
      case "audio":
        return (
          <div className="spv-media">
            <audio src={url} controls />
          </div>
        );
      case "pdf":
        return <PdfView {...props} />;
      case "docx":
        return <DocxView {...props} />;
      case "spreadsheet":
        return <TableView {...props} kind="spreadsheet" />;
      case "csv":
        return <TableView {...props} kind="csv" tab={name.toLowerCase().endsWith(".tsv")} />;
      case "text":
      default:
        // Small enough to load fully → read-only CodeMirror (syntax
        // highlighting + selectable text + find). Larger files stream
        // through the virtualized viewer instead.
        return size <= MAX_EDITOR_BYTES ? (
          <TextCodeView {...props} onBinary={() => setForceHex(true)} />
        ) : (
          <TextStreamView {...props} onBinary={() => setForceHex(true)} />
        );
    }
  })();

  const isImage = !forceHex && (kind === "image" || kind === "svg" || kind === "tiff");

  return (
    <div
      className="dlg-overlay"
      onMouseDown={(e) => {
        overlayDownRef.current = e.target === e.currentTarget;
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget && overlayDownRef.current) onClose();
        overlayDownRef.current = false;
      }}
    >
      <div className="dlg dlg--preview" style={dialogStyle} onClick={(e) => e.stopPropagation()}>
        <div className="dlg-head" {...handleProps}>
          <span className="dlg-title">
            <FileText size={13} />
            {name}
          </span>
          <span className="spv-chip">{t(KIND_LABEL[forceHex ? "text" : kind])}</span>
          <span className="spv-chip">{formatBytes(size)}</span>
          <span className="editor-path mono" title={path}>
            {path}
          </span>
          <div className="spv-head-actions">
            {isImage && (
              <IconButton
                variant="mini"
                onClick={() => setImageActual((v) => !v)}
                title={imageActual ? t("Fit to window") : t("Actual size")}
              >
                {imageActual ? <Minimize2 size={12} /> : <Maximize2 size={12} />}
              </IconButton>
            )}
            {onOpenInEditor && (
              <IconButton variant="mini" onClick={onOpenInEditor} title={t("Open in editor")}>
                <Pencil size={12} />
              </IconButton>
            )}
            {onDownload && (
              <IconButton variant="mini" onClick={onDownload} title={t("Download…")}>
                <Download size={12} />
              </IconButton>
            )}
            {onOpenExternal && (
              <IconButton
                variant="mini"
                onClick={onOpenExternal}
                title={t("Open with system editor")}
              >
                <ExternalLink size={12} />
              </IconButton>
            )}
            <IconButton variant="mini" onClick={onClose} title={t("Close")}>
              <X size={12} />
            </IconButton>
          </div>
        </div>
        <div className="spv-body">
          <Suspense fallback={<Fallback />}>{viewer}</Suspense>
        </div>
      </div>
    </div>
  );
}
