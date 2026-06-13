import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { EditorState, type Extension } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers as cmLineNumbers,
  highlightSpecialChars,
  drawSelection,
} from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import { search, searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import * as cmd from "../../lib/commands";
import {
  MAX_EDITOR_BYTES,
  buildEditorTheme,
  languageFromFilename,
} from "../../lib/sftpEditor";
import { useI18n } from "../../i18n/useI18n";
import { useThemeStore } from "../../stores/useThemeStore";
import { useSettingsStore } from "../../stores/useSettingsStore";
import type { ViewerProps } from "./types";

type Props = ViewerProps & {
  /** Called when the backend sniffs the file as binary — the dialog
   *  switches to the hex viewer. */
  onBinary: () => void;
};

/** Read-only CodeMirror viewer for text files small enough to load in
 *  full (≤ MAX_EDITOR_BYTES). Gives syntax highlighting, native text
 *  selection / copy, and find — the same engine as the editor dialog,
 *  minus editing. Larger files stream through TextStreamView instead. */
export default function TextCodeView({ sshArgs, path, name, onBinary }: Props) {
  const { t } = useI18n();
  const resolvedDark = useThemeStore((s) => s.resolvedDark);
  const wrap = useSettingsStore((s) => s.editorWrapDefault);
  const showNums = useSettingsStore((s) => s.editorLineNumbersDefault);

  const hostRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<string>("");
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");
  const [meta, setMeta] = useState({ encoding: "", lines: 0, lossy: false });

  // Fetch the full file once per target. Binary content (mislabeled
  // by extension) hands off to the hex viewer.
  useEffect(() => {
    let alive = true;
    setStatus("loading");
    setError("");
    cmd
      .sftpReadText({ ...sshArgs, path, maxBytes: MAX_EDITOR_BYTES })
      .then((res) => {
        if (!alive) return;
        if (res.encoding === "binary") {
          onBinary();
          return;
        }
        contentRef.current = res.content;
        setMeta({
          encoding: res.encoding,
          lines: res.content.length ? res.content.split("\n").length : 0,
          lossy: res.lossy,
        });
        setStatus("ready");
      })
      .catch((e) => {
        if (!alive) return;
        setError(e instanceof Error ? e.message : String(e));
        setStatus("error");
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  // Mount the read-only editor once content is in hand. Re-mounts on a
  // theme / wrap / gutter change — all rare, so a fresh view is simpler
  // than reconfiguring compartments.
  useEffect(() => {
    if (status !== "ready") return;
    const host = hostRef.current;
    if (!host) return;
    const lang = languageFromFilename(name);
    const extensions: Extension[] = [
      showNums ? cmLineNumbers() : [],
      highlightSpecialChars(),
      drawSelection(),
      EditorState.allowMultipleSelections.of(true),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      highlightSelectionMatches(),
      search({ top: true }),
      wrap ? EditorView.lineWrapping : [],
      EditorState.readOnly.of(true),
      keymap.of([...defaultKeymap, ...searchKeymap]),
      ...buildEditorTheme(resolvedDark),
    ];
    if (lang) extensions.push(lang);
    const view = new EditorView({
      state: EditorState.create({ doc: contentRef.current, extensions }),
      parent: host,
    });
    return () => view.destroy();
  }, [status, name, resolvedDark, wrap, showNums]);

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
      <div className="spv-code ux-selectable" ref={hostRef} />
      <div className="spv-statusbar">
        <span>{meta.encoding || "—"}</span>
        <span>
          {meta.lines.toLocaleString()} {t("lines")}
        </span>
        {meta.lossy && <span className="spv-warn">{t("invalid UTF-8 (replaced)")}</span>}
        <span className="spv-grow" />
        <span>{t("read-only")}</span>
      </div>
    </>
  );
}
