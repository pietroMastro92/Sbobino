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
          ? "Ready"
          : variant === "warning"
            ? "Warning"
            : "Error"}
      </span>
      <span>{message}</span>
    </div>
  );
}
