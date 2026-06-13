import { useEffect, useRef, useState } from "react";
import { Virtuoso } from "react-virtuoso";
import * as cmd from "../../lib/commands";
import { useI18n } from "../../i18n/useI18n";
import { formatBytes } from "../../lib/dbConnCache";
import type { ViewerProps } from "./types";

const BYTES_PER_ROW = 16;
const WINDOW_BYTES = 64 * 1024;

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function hexByte(b: number): string {
  return b.toString(16).padStart(2, "0");
}

/** Windowed hex dump for binary files. Rows are virtualized and the
 *  backing 64 KiB windows are fetched on demand as the user scrolls —
 *  any size opens instantly. */
export default function HexView({ sshArgs, path, size }: ViewerProps) {
  const { t } = useI18n();
  const totalRows = Math.max(1, Math.ceil(size / BYTES_PER_ROW));
  const windowsRef = useRef<Map<number, Uint8Array>>(new Map());
  const pendingRef = useRef<Set<number>>(new Set());
  const [, setVersion] = useState(0);
  const [error, setError] = useState("");

  useEffect(() => {
    windowsRef.current = new Map();
    pendingRef.current = new Set();
    setVersion((v) => v + 1);
  }, [path, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  function ensureWindow(win: number) {
    if (windowsRef.current.has(win) || pendingRef.current.has(win)) return;
    pendingRef.current.add(win);
    cmd
      .sftpReadRange({ ...sshArgs, path, offset: win * WINDOW_BYTES, length: WINDOW_BYTES })
      .then((chunk) => {
        windowsRef.current.set(win, base64ToBytes(chunk.base64));
        pendingRef.current.delete(win);
        setVersion((v) => v + 1);
      })
      .catch((e) => {
        pendingRef.current.delete(win);
        setError(e instanceof Error ? e.message : String(e));
      });
  }

  function renderRow(rowIndex: number) {
    const rowOffset = rowIndex * BYTES_PER_ROW;
    const win = Math.floor(rowOffset / WINDOW_BYTES);
    const bytes = windowsRef.current.get(win);
    const offsetLabel = rowOffset.toString(16).padStart(8, "0");
    if (!bytes) {
      ensureWindow(win);
      return (
        <div className="spv-hex-row">
          <span className="spv-hex-offset">{offsetLabel}</span>
          <span className="spv-hex-bytes">{t("…")}</span>
        </div>
      );
    }
    const startInWindow = rowOffset - win * WINDOW_BYTES;
    const slice = bytes.subarray(startInWindow, startInWindow + BYTES_PER_ROW);
    let hex = "";
    let ascii = "";
    for (let i = 0; i < BYTES_PER_ROW; i++) {
      if (i < slice.length) {
        const b = slice[i];
        hex += hexByte(b) + (i === 7 ? "  " : " ");
        ascii += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
      } else {
        hex += i === 7 ? "    " : "   ";
        ascii += " ";
      }
    }
    return (
      <div className="spv-hex-row">
        <span className="spv-hex-offset">{offsetLabel}</span>
        <span className="spv-hex-bytes">{hex.trimEnd()}</span>
        <span className="spv-hex-ascii">{ascii}</span>
      </div>
    );
  }

  if (error) {
    return <div className="spv-center is-error">{error}</div>;
  }

  return (
    <>
      <Virtuoso
        className="spv-scroll"
        style={{ height: "100%" }}
        totalCount={totalRows}
        overscan={800}
        rangeChanged={({ startIndex, endIndex }) => {
          const first = Math.floor((startIndex * BYTES_PER_ROW) / WINDOW_BYTES);
          const last = Math.floor((endIndex * BYTES_PER_ROW) / WINDOW_BYTES);
          for (let w = first; w <= last; w++) ensureWindow(w);
        }}
        itemContent={(index) => renderRow(index)}
      />
      <div className="spv-statusbar">
        <span>{t("Binary")}</span>
        <span className="spv-grow">{formatBytes(size)}</span>
        <span>
          {totalRows.toLocaleString()} {t("rows")} · {BYTES_PER_ROW} {t("bytes/row")}
        </span>
      </div>
    </>
  );
}
