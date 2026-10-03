import { cn } from "@/shared/lib/cn";

import type { PersonaModelDiscoveryStatus } from "./personaModelDiscoveryStatus";

/**
 * Status line under the Model control: progressive loading, failure copy,
 * and optional Retry. Shared by onboarding (`AgentModelField`), create
 * (`PersonaModelField`), and instance edit so surfaces do not fork markup.
 */
export function ModelDiscoveryStatusLine({
  disabled = false,
  loading,
  loadingMessage,
  onRetry,
  status,
  testId = "model-discovery-status",
}: {
  disabled?: boolean;
  loading: boolean;
  /** Full progressive copy for the status line (may be long). */
  loadingMessage?: string | null;
  onRetry?: () => void;
  status: PersonaModelDiscoveryStatus | null;
  testId?: string;
}) {
  if (loading) {
    // Only render once progressive copy is ready (after MODEL_DISCOVERY_SLOW_MS).
    // Early loading is communicated solely by the control's short label.
    const text = loadingMessage?.trim();
    if (!text) return null;
    return (
      <p
        aria-live="polite"
        className="text-xs text-muted-foreground"
        data-testid={testId}
      >
        {text}
      </p>
    );
  }

  if (status === null) {
    return null;
  }

  return (
    <div
      className="flex flex-wrap items-start gap-x-3 gap-y-1"
      data-testid={testId}
    >
      <p
        aria-live="polite"
        className={cn(
          "min-w-0 flex-1 text-xs",
          status.tone === "warning" ? "text-warning" : "text-muted-foreground",
        )}
      >
        {status.message}
      </p>
      {status.retryable && onRetry ? (
        <button
          className="shrink-0 text-xs font-medium text-foreground underline-offset-2 hover:underline disabled:opacity-50"
          data-testid="model-discovery-retry"
          disabled={disabled}
          onClick={onRetry}
          type="button"
        >
          Retry
        </button>
      ) : null}
    </div>
  );
}
