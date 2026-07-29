import { useQueries, useQuery } from "@tanstack/react-query";

import {
  discoverBackendProviders,
  probeBackendProvider,
} from "@/shared/api/tauri";
import type { BackendProviderCandidate } from "@/shared/api/types";

/**
 * Queries about backend providers — the binaries that let an agent run
 * somewhere other than this computer.
 *
 * Their own module rather than another block in `hooks.ts`: providers are the
 * one axis of the agent surface that is about WHERE an agent lives, and the
 * three surfaces that ask (onboarding, Settings → Remote servers, the create
 * dialog) share nothing else in that file.
 */

export const backendProvidersQueryKey = ["backend-providers"] as const;
/** Keyed by binary path, not id: the path is what is actually spawned. */
export const backendProviderProbeQueryKey = ["backend-provider-probe"] as const;

/**
 * Which providers are installed.
 *
 * A `PATH` walk, not a spawn: cheap enough for any surface to ask on mount.
 */
export function useBackendProvidersQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: backendProvidersQueryKey,
    queryFn: discoverBackendProviders,
    staleTime: 30_000,
  });
}

/**
 * `info` for each discovered provider, so a surface can name and version them.
 *
 * Deliberately NOT what `runTargetOptions` does: the create dialog renders its
 * provider list before the user has asked anything, so probing there would
 * spawn every discovered binary to decorate a dropdown. This hook exists for
 * the one surface whose entire subject IS the providers (Settings → Agents →
 * Remote servers), where the round-trip is the thing the user asked for.
 *
 * `info` is the only op that opens no connection (see docs/remote-agents.md):
 * it is a local spawn under a 10s desktop budget, not an SSH handshake, so N
 * of them cost N short-lived child processes and never block on a host.
 *
 * `retry: false` per entry — one broken provider must not blank the gallery;
 * its own row reports the failure.
 */
export function useBackendProviderProbesQuery(
  providers: readonly BackendProviderCandidate[],
) {
  return useQueries({
    queries: providers.map((provider) => ({
      queryKey: [...backendProviderProbeQueryKey, provider.binaryPath],
      queryFn: () => probeBackendProvider(provider.binaryPath),
      staleTime: 60_000,
      retry: false,
    })),
  });
}
