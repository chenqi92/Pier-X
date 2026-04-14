import { useI18n } from "../i18n/useI18n";

type Props = {
  version?: string;
  coreInfo?: string;
};

export default function StatusBar({ version, coreInfo }: Props) {
  const { t } = useI18n();

  return (
    <footer className="statusbar">
      <span className="statusbar__text">{t("Ready")}</span>
      <span className="statusbar__spacer" />
      {version ? (
        <span className="statusbar__meta">v{version}{coreInfo ? ` · ${coreInfo}` : ""}</span>
      ) : null}
    </footer>
  );
}
