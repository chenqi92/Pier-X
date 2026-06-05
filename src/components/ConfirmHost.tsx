import ConfirmDialog from "./ConfirmDialog";
import { useConfirmStore } from "../stores/useConfirmStore";
import { useI18n } from "../i18n/useI18n";

/**
 * Single app-root host for the imperative `confirm()` API
 * (`useConfirmStore`). Renders one themed `ConfirmDialog` driven by
 * the current request and resolves the awaiting promise on
 * confirm/cancel. Mounted once in `App`.
 */
export default function ConfirmHost() {
  const { t } = useI18n();
  const request = useConfirmStore((s) => s.request);
  const settle = useConfirmStore((s) => s.settle);

  return (
    <ConfirmDialog
      open={request !== null}
      title={request?.title ?? t("Confirm")}
      message={request?.message ?? ""}
      confirmLabel={request?.confirmLabel}
      cancelLabel={request?.cancelLabel}
      tone={request?.tone ?? "neutral"}
      onConfirm={() => settle(true)}
      onCancel={() => settle(false)}
    />
  );
}
