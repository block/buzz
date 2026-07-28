import { invoke } from "@tauri-apps/api/core";

export type ProviderUsageId = "codex" | "claude" | "grok";
export type ProviderUsagePreference = "auto" | ProviderUsageId;
export type ProviderUsageAvailability =
  | "available"
  | "not_installed"
  | "not_authenticated"
  | "unsupported"
  | "incompatible_version"
  | "temporarily_unavailable";

export type ProviderUsageCapability = {
  id: ProviderUsageId;
  name: string;
  availability: ProviderUsageAvailability;
  detail: string;
};

export type ProviderUsageWindow = {
  id: string;
  label: string;
  usedPercent: number;
  remainingPercent: number;
  resetsAt: number | null;
  durationMinutes: number | null;
};

export type ProviderUsageSnapshot = {
  provider: ProviderUsageId;
  vendor: "openai" | "anthropic" | "xai";
  product: string;
  source: "personalAllowance";
  planType: string | null;
  windows: ProviderUsageWindow[];
  totals: {
    creditBalance: string | null;
    resetCreditsAvailable: number | null;
    lifetimeTokens: number | null;
    latestDailyTokens: number | null;
    latestDailyDate: string | null;
  };
  fetchedAt: number;
};

export function listProviderUsageCapabilities(): Promise<
  ProviderUsageCapability[]
> {
  return invoke<ProviderUsageCapability[]>("list_provider_usage_capabilities");
}

export function getProviderUsage(
  provider: ProviderUsageId,
): Promise<ProviderUsageSnapshot> {
  return invoke<ProviderUsageSnapshot>("get_provider_usage", { provider });
}
