import { useQuery } from "@tanstack/react-query";

import { getIdentity } from "@/shared/api/tauriIdentity";

export function useIdentityQuery() {
  return useQuery({
    queryKey: ["identity"],
    queryFn: getIdentity,
    staleTime: Number.POSITIVE_INFINITY,
    // A failure here means "no native identity backend" (e.g. web deployment
    // outside Tauri) — retrying can't change that. This also sidesteps a
    // query-core gotcha: the default queryClient's networkMode: "always" only
    // bypasses the *online* check in the retryer, not focusManager.isFocused().
    // If the window isn't OS-focused at the moment the single default retry
    // fires, canContinue() returns false and the retryer calls pause() —
    // which then waits indefinitely for a focus/visibility event, leaving
    // this query stuck on fetchStatus "paused" forever instead of "error".
    retry: false,
  });
}
