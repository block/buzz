import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  clearProviderSecret,
  getProviderSecretStatus,
  setProviderSecret,
} from "@/shared/api/tauri";
import type { AcpProviderProfile } from "@/shared/api/types";

const providerSecretQueryKey = (providerId: string) => [
  "provider-secret-status",
  providerId,
];

export function useProviderSecret(
  providerId: string,
  profiles: readonly AcpProviderProfile[] | undefined,
  enabled = true,
) {
  const normalized = providerId.trim().toLowerCase();
  const profile = profiles?.find(
    (candidate) =>
      candidate.id === normalized || candidate.aliases.includes(normalized),
  );
  const credential = profile?.credential?.deviceKeyring
    ? profile.credential
    : null;
  const canonicalProviderId = profile?.id ?? normalized;
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: providerSecretQueryKey(canonicalProviderId),
    queryFn: () => getProviderSecretStatus(canonicalProviderId),
    enabled: enabled && credential !== null,
    staleTime: 5_000,
  });
  const setMutation = useMutation({
    mutationFn: (value: string) =>
      setProviderSecret(canonicalProviderId, value),
    onSuccess: (status) => {
      queryClient.setQueryData(
        providerSecretQueryKey(canonicalProviderId),
        status,
      );
    },
  });
  const clearMutation = useMutation({
    mutationFn: () => clearProviderSecret(canonicalProviderId),
    onSuccess: (status) => {
      queryClient.setQueryData(
        providerSecretQueryKey(canonicalProviderId),
        status,
      );
    },
  });

  return {
    credential,
    configured: statusQuery.data?.configured ?? false,
    source: statusQuery.data?.source ?? null,
    restartedCount: statusQuery.data?.restartedCount ?? 0,
    failedRestartCount: statusQuery.data?.failedRestartCount ?? 0,
    isPending:
      statusQuery.isPending || setMutation.isPending || clearMutation.isPending,
    error:
      statusQuery.error ?? setMutation.error ?? clearMutation.error ?? null,
    set: setMutation.mutateAsync,
    clear: clearMutation.mutateAsync,
  };
}
