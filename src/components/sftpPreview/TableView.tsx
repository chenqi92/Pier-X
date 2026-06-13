import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import * as cmd from "../../lib/commands";
import type { SftpTablePreview } from "../../lib/commands";
import PreviewTable from "../PreviewTable";
import Select from "../Select";
import { useI18n } from "../../i18n/useI18n";
import type { DataPreview } from "../../lib/types";
import type { ViewerProps } from "./types";

type Props = ViewerProps & {
  /** `spreadsheet` (xlsx/xls/ods) or `csv`/`tsv`. */
  kind: "spreadsheet" | "csv";
  /** True for tab-delimited files (`.tsv`). */
  tab?: boolean;
};

/** Spreadsheet / CSV viewer. Parsing happens in pier-core (calamine /
 *  csv) and the rows render in the shared PreviewTable. Spreadsheets
 *  get a sheet picker. */
export default function TableView({ sshArgs, path, kind, tab }: Props) {
  const { t } = useI18n();
  const [data, setData] = useState<SftpTablePreview | null>(null);
  const [sheet, setSheet] = useState(0);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setError("");
    const call =
      kind === "csv"
        ? cmd.sftpPreviewCsv({ ...sshArgs, path, tab: tab ?? false })
        : cmd.sftpPreviewSpreadsheet({ ...sshArgs, path, sheetIndex: sheet });
    call
      .then((res) => {
        if (cancelled) return;
        setData(res);
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
  }, [path, kind, tab, sheet, sshArgs.host, sshArgs.port, sshArgs.user, sshArgs.authMode]);

  if (status === "loading") {
    return (
      <div className="spv-center">
        <Loader2 size={20} className="spv-spin" />
        {t("Parsing…")}
      </div>
    );
  }
  if (status === "error") {
    return <div className="spv-center is-error">{error}</div>;
  }

  const preview: DataPreview | null = data
    ? { columns: data.columns, rows: data.rows, truncated: data.truncated }
    : null;
  const showPicker = kind === "spreadsheet" && data && data.sheetNames.length > 1;

  return (
    <>
      {showPicker && (
        <div className="spv-table-bar">
          <span>{t("Sheet")}</span>
          <Select
            compact
            value={String(sheet)}
            onChange={(v) => setSheet(Number(v))}
            items={data!.sheetNames.map((nm, i) => ({ value: String(i), label: nm }))}
          />
        </div>
      )}
      <div className="spv-table-scroll">
        <PreviewTable preview={preview} emptyLabel={t("No rows.")} />
      </div>
    </>
  );
}
