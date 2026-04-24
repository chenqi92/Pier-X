import { useI18n } from "../../i18n/useI18n";
import RedisTypeBadge from "./RedisTypeBadge";

type Props = {
  keys: string[];
  /** Detailed info for the currently selected key — used to colour the matching row. */
  selected: string | null;
  /** Type of the selected key (we only know the type of the active selection). */
  selectedKind?: string | null;
  onSelect: (key: string) => void;
  truncated?: boolean;
};

/**
 * Flat key list. A hierarchical (colon-separated) tree would need
 * the backend to expose per-key types in the scan response — see
 * docs/BACKEND-GAPS.md. For the pilot we stick to the list view.
 */
export default function RedisKeyList({
  keys,
  selected,
  selectedKind,
  onSelect,
  truncated,
}: Props) {
  const { t } = useI18n();

  if (keys.length === 0) {
    return (
      <div className="rds-detail-empty" style={{ padding: "var(--sp-6) var(--sp-4)" }}>
        <div>{t("No keys match this pattern.")}</div>
      </div>
    );
  }

  return (
    <div className="rds-keys">
      {keys.map((key) => {
        const isSelected = key === selected;
        return (
          <button
            key={key}
            type="button"
            className={"rds-row" + (isSelected ? " selected" : "")}
            onClick={() => onSelect(key)}
          >
            <RedisTypeBadge kind={isSelected ? selectedKind : null} />
            <span className="rds-key">{key}</span>
            <span className="rds-meta" />
            <span className="rds-meta" />
          </button>
        );
      })}
      {truncated && (
        <div className="rds-truncated-note">
          {t("Results truncated — refine the pattern or bump the scan limit.")}
        </div>
      )}
    </div>
  );
}
