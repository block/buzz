import { useQuery } from "@tanstack/react-query";

import {
  getProviderUsage,
  listProviderUsageCapabilities,
} from "@/shared/api/tauriProviderUsage";
import { useFeatureEnabled } from "@/shared/features";
import {
  resolveProviderUsagePreference,
  useProviderUsagePreference,
} from "./providerUsagePreference";

export const PROVIDER_USAGE_STALE_MS = 5 * 60 * 1_000;

export function providerUsageProductLabel(provider: string): string {
  if (provider === "codex") return "Codex";
  if (provider === "claude") return "Claude";
  if (provider === "grok") return "Grok";
  return "Provider";
}

/**
 * Shared provider-allowance query. The capability read is cheap and must
 * resolve before Buzz starts a provider process; disabling the preview keeps
 * both IPC calls dormant. Every mounted consumer shares the provider-scoped
 * React Query entries, so the Agents dashboard and chrome indicator never
 * multiply app-server reads.
 */
export function useProviderUsageSnapshot() {
  const featureEnabled = useFeatureEnabled("providerUsage");
  const preference = useProviderUsagePreference();
  const capabilitiesQuery = useQuery({
    queryKey: ["provider-usage-capabilities"],
    queryFn: listProviderUsageCapabilities,
    enabled: featureEnabled,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const provider = resolveProviderUsagePreference(
    preference,
    capabilitiesQuery.data,
  );
  const capability = capabilitiesQuery.data?.find(
    (candidate) => candidate.id === provider,
  );
  const adapterAvailable = capability?.availability === "available";
  const query = useQuery({
    queryKey: ["provider-usage", provider],
    queryFn: () => getProviderUsage(provider),
    enabled: featureEnabled && adapterAvailable,
    staleTime: PROVIDER_USAGE_STALE_MS,
    refetchInterval: PROVIDER_USAGE_STALE_MS,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    retry: 1,
  });

  return {
    adapterAvailable,
    capabilitiesQuery,
    capability,
    featureEnabled,
    preference,
    productLabel: providerUsageProductLabel(provider),
    provider,
    query,
  };
}
