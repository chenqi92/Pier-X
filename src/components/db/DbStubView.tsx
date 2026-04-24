import { useI18n } from "../../i18n/useI18n";

type Props = {
  title: string;
  /** Optional subtitle — usually references docs/BACKEND-GAPS.md. */
  subtitle?: string;
};

/**
 * Placeholder for the Structure and Schema tabs. The visual structure
 * lives in the design (panel-db.jsx) but requires introspection
 * commands the backend doesn't expose yet — see docs/BACKEND-GAPS.md.
 */
export default function DbStubView({ title, subtitle }: Props) {
  const { t } = useI18n();
  return (
    <div className="db2-stub">
      <div className="db2-stub-inner">
        <div className="db2-stub-title">{title}</div>
        <div className="db2-stub-sub">
          {subtitle ?? t("This tab will surface more introspection once the backend exposes it.")}
        </div>
      </div>
    </div>
  );
}
