import { t } from "../i18n";

type StatusBadgeVariant = "warning" | "ready" | "error";

export function StatusBadge({
  variant,
  message,
}: {
  variant: StatusBadgeVariant;
  message: string;
}) {
  return (
    <div className={`status-badge status-badge-${variant}`} role="status" aria-live="polite">
      <span
        className={
          variant === "ready"
            ? "kind-chip"
            : variant === "warning"
              ? "missing-chip"
              : "status-badge-error-chip"
        }
      >
        {variant === "ready"
          ? t("status.ready", "Ready")
          : variant === "warning"
            ? t("status.warning", "Warning")
            : t("status.error", "Error")}
      </span>
      <span>{message}</span>
    </div>
  );
}
