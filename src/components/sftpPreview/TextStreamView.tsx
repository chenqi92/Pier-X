import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { Virtuoso } from "react-virtuoso";
import * as cmd from "../../lib/commands";
import type { SftpTextChunk } from "../../lib/commands";
import { useI18n } from "../../i18n/useI18n";
import { formatBytes } from "../../lib/dbConnCache";
import type { ViewerProps } from "./types";

type Props = ViewerProps & {
  /** Called when the backend sniffs the file as binary — the dialog
   *  switches to the hex viewer. */
  onBinary: () => void;
};

let streamSeq = 0;

/** Streaming, virtualized text/log viewer. The first window paints
 *  after one round trip regardless of file size; more windows stream
 *  in until EOF or the cap, then a "load more" continues. Read-only —
 *  editing is the editor dialog's job. */
export default function TextStreamView({ sshArgs, path, size, onBinary }: Props) {
  const { t } = useI18n();
  const linesRef = useRef<string[]>([]);
  const partialRef = useRef<string>("");
  const streamIdRef = useRef<string>("");
  const [count, setCount] = useState(0);
  const [encoding, setEncoding] = useState("");
  const [loadedBytes, setLoadedBytes] = useState(0);
  const [status, setStatus] = useState<"loading" | "streaming" | "done" | "truncated" | "error">(
    "loading",
  );
  const [error, setError] = useState("");
  const nextOffsetRef = useRef(0);

  function reset() {
    linesRef.current = [];
    partialRef.current = "";
    setCount(0);
  }

  function pushChunk(chunk: SftpTextChunk) {
    if (chunk.kind === "binary") {
      onBinary();
      return;
    }
    if (!encoding && chunk.encoding) setEncoding(chunk.encoding);
    if (chunk.text) {
      const combined = partialRef.current + chunk.text;
      const parts = combined.split("\n");
      partialRef.current = parts.pop() ?? "";
      if (parts.length > 0) {
        for (const line of parts) linesRef.current.push(line);
      }
    }
    nextOffsetRef.current = chunk.nextOffset;
    setLoadedBytes(chunk.nextOffset);
    if (chunk.done) {
      // Flush the trailing partial line at EOF.
      if (!chunk.truncated && partialRef.current.length > 0) {
        linesRef.current.push(partialRef.current);
        partialRef.current = "";
      }
      setCount(linesRef.current.length);
      setStatus(chunk.truncated ? "truncated" : "done");
    } else {
      setCount(linesRef.current.length);
      setStatus("streaming");
    }
  }

  function startStream(startOffset: number) {
    const id = `sftp-text-${++streamSeq}`;
    streamIdRef.current = id;
    cmd
      .sftpStreamText({ ...sshArgs, path, streamId: id, startOffset }, pushChunk)
      .catch((e) => {
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
  }

  useEffect(() => {
    reset();
    setStatus("loading");
    setError("");
    setEncoding("");
    setLoadedBytes(0);
    nextOffsetRef.current = 0;
    startStream(0);
    return () => {
      if (streamIdRef.current) void cmd.sftpCancelTransfer(streamIdRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  if (status === "loading") {
    return (
      <div className="spv-center">
        <Loader2 size={20} className="spv-spin" />
        {t("Loading…")}
      </div>
    );
  }
  if (status === "error") {
    return <div className="spv-center is-error">{error}</div>;
  }

  return (
    <>
      <Virtuoso
        className="spv-scroll"
        style={{ height: "100%" }}
        totalCount={count}
        itemContent={(index) => (
          <div className="spv-text-row">
            <span className="spv-ln">{index + 1}</span>
            <span className="spv-text">{linesRef.current[index] || " "}</span>
          </div>
        )}
      />
      <div className="spv-statusbar">
        <span>{encoding || "—"}</span>
        <span>
          {count.toLocaleString()} {t("lines")}
        </span>
        <span className="spv-grow">
          {formatBytes(loadedBytes)} / {formatBytes(size)}
        </span>
        {status === "streaming" && (
          <span>
            <Loader2 size={11} className="spv-spin" /> {t("streaming…")}
          </span>
        )}
        {status === "truncated" && (
          <button
            type="button"
            className="btn is-ghost"
            onClick={() => {
              setStatus("streaming");
              startStream(nextOffsetRef.current);
            }}
          >
            {t("Load more")}
          </button>
        )}
      </div>
    </>
  );
}
