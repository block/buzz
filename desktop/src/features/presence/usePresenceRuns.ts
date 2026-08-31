import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { nextRunExpiry, type PresenceRuns } from "./runPresence";

/** Scope snapshots to the captured reader and community; errors are not offline. */
export function usePresenceRuns(pubkeys: string[]) {
  const { activeCommunity } = useCommunities();
  const { data: identity } = useIdentityQuery();
  const connected = useRelayConnection() === "connected";
  const owner = identity?.pubkey;
  const relay = activeCommunity?.relayUrl;
  const authors = [...new Set(pubkeys.map((key) => key.toLowerCase()))].sort();
  const query = useQuery({
    queryKey: ["presence-runs", relay, owner, ...authors],
    enabled: !!owner && !!relay && authors.length > 0,
    queryFn: () =>
      invokeTauri<PresenceRuns>("get_presence_runs", {
        expectedOwner: owner,
        relayUrl: relay,
        pubkeys: authors,
      }),
    refetchInterval: connected ? 60_000 : false,
    staleTime: 30_000,
    retry: false,
  });
  const [now, setNow] = useState(() => Date.now() / 1000);
  const expiry = nextRunExpiry(query.data, now);
  useEffect(() => {
    // Also reevaluate after a background/sleep interval; do not wait for a poll.
    const update = () => setNow(Date.now() / 1000);
    update();
    const timer =
      expiry === undefined
        ? undefined
        : window.setTimeout(
            update,
            Math.max(0, expiry * 1000 - Date.now()) + 1,
          );
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
    };
  }, [expiry]);
  return { ...query, now: Math.max(now, Date.now() / 1000) };
}
