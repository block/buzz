export type ProviderUsageTone = "healthy" | "warning" | "critical";

export function providerUsageTone(remainingPercent: number): ProviderUsageTone;

export function formatTokenCount(value: number | null | undefined): string;

export function formatUsageReset(
  epochSeconds: number | null | undefined,
): string;

export function providerUsageErrorMessage(error: unknown): string;
