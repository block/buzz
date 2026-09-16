import { useEffect, useRef } from "react";
import type { Community } from "./types";
import {
  fetchCommunityProfile,
  type CommunityProfile,
} from "@/shared/api/communityProfile";
import { relayClient } from "@/shared/api/relayClient";

export const COMMUNITY_NAME_REFRESH_EVENT = "buzz-community-name-refresh";

/** Refresh on launch/add/switch, window focus, network recovery, and reconnect. */
export function useCommunityNames(
  communities: Community[],
  activeId: string | null,
  reinitKey: number,
  apply: (id: string, relayUrl: string, profile: CommunityProfile) => void,
) {
  const scope = JSON.stringify({
    targets: communities.map(({ id, relayUrl }) => ({ id, relayUrl })),
    activeId,
    reinitKey,
  });
  const applyRef = useRef(apply);
  applyRef.current = apply;
  useEffect(() => {
    let disposed = false;
    const generations = new Map<string, number>();
    const { targets } = JSON.parse(scope) as {
      targets: Pick<Community, "id" | "relayUrl">[];
    };
    const refresh = () => {
      for (const target of targets) {
        const generation = (generations.get(target.id) ?? 0) + 1;
        generations.set(target.id, generation);
        void fetchCommunityProfile(target.relayUrl)
          .then((profile) => {
            if (
              !disposed &&
              generations.get(target.id) === generation &&
              profile
            ) {
              applyRef.current(target.id, target.relayUrl, profile);
            }
          })
          .catch(() => {
            /* Keep cached names; retry at the next refresh boundary. */
          });
      }
    };
    refresh();
    const unsubscribe = relayClient.subscribeToConnectionState((state) => {
      if (state === "connected") refresh();
    });
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    window.addEventListener(COMMUNITY_NAME_REFRESH_EVENT, refresh);
    return () => {
      disposed = true;
      unsubscribe();
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
      window.removeEventListener(COMMUNITY_NAME_REFRESH_EVENT, refresh);
    };
  }, [scope]);
}
