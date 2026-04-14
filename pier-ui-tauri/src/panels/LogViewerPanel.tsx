import { useI18n } from "../i18n/useI18n";
import type { TabState } from "../lib/types";

type Props = { tab: TabState };

export default function LogViewerPanel({ tab }: Props) {
  const { t } = useI18n();

  return (
    <div className="panel-scroll">
      <section className="panel-section">
        <div className="panel-section__title"><span>{t("Logs")}</span></div>
        <div className="empty-note">
          Log viewer will tail remote log files via SSH.
          {tab.logCommand ? <div className="inline-note">Command: {tab.logCommand}</div> : null}
        </div>
      </section>
    </div>
  );
}
