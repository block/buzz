import { Check } from "lucide-react";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
export function RuntimeReadinessIndicator({
  runtime,
  ready,
}: {
  runtime: AcpRuntimeCatalogEntry;
  ready: boolean;
}) {
  // Checkmark temporarily hidden; flip to true to restore it.
  const showReadinessCheckmark = false;
  if (!ready || !showReadinessCheckmark) return null;

  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute right-8 top-8 flex h-8 w-8 items-center justify-center rounded-full border border-[var(--buzz-welcome-chartreuse)] bg-[var(--buzz-welcome-chartreuse)]"
      data-testid={`onboarding-runtime-check-${runtime.id}`}
    >
      <Check
        className="h-4 w-4 text-foreground"
        data-testid={`onboarding-runtime-checkmark-${runtime.id}`}
        strokeWidth={3}
      />
    </span>
  );
}
