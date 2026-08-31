import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import type { MoveProgress } from "./moveSelection";
import { hostNativeDrain } from "./hostNativeDrain";

export type StartProgress = {
  operation: string;
  action?: "start" | "stop";
  created_at: number;
  current: boolean;
  agent: string;
  host: string;
  run: string;
  status: string;
  error?: string;
};
type Snapshot = {
  operations: StartProgress[];
  moves: MoveProgress[];
  error?: string;
};
const empty: Snapshot = { operations: [], moves: [] };
const key = (owner?: string, relay?: string) => ["host-start", relay, owner];
export const START_REFRESH = "buzz:refresh-host-start";

/** Receiver is scoped to the app, not the Hosts page. Native owns fsynced
 * encrypted outbox and execution ledger; no browser cache is launch authority. */
export function useHostStartReceiver(
  owner?: string,
  relay?: string,
  enabled = false,
) {
  const client = useQueryClient();
  useEffect(() => {
    if (!owner || !relay || !enabled) return;
    let active = true;
    let pending: Promise<void> | undefined;
    const refresh = () => {
      if (!active || pending) return;
      pending = (async () => {
        try {
          const snapshot = await invokeTauri<{
            operations: StartProgress[];
            moves: MoveProgress[];
            errors: string[];
          }>("pump_host_start", {
            expectedOwner: owner,
            expectedRelay: relay,
          });
          if (active)
            client.setQueryData(key(owner, relay), {
              operations: snapshot.operations,
              moves: snapshot.moves ?? [],
              error: snapshot.errors.length
                ? "Destination transport has unconfirmed operations; retries continue independently."
                : undefined,
            });
        } catch {
          if (active)
            client.setQueryData<Snapshot>(key(owner, relay), (old) => ({
              operations: old?.operations ?? [],
              moves: old?.moves ?? [],
              error:
                "Start transport unconfirmed. Retrying the same saved operation; no replacement will be launched.",
            }));
        }
      })().finally(() => {
        pending = undefined;
      });
    };
    void hostNativeDrain.wait().then(refresh);
    const timer = window.setInterval(refresh, 5_000);
    window.addEventListener(START_REFRESH, refresh);
    window.addEventListener("online", refresh);
    window.addEventListener("focus", refresh);
    return () => {
      active = false;
      if (pending) hostNativeDrain.hold(pending);
      window.clearInterval(timer);
      window.removeEventListener(START_REFRESH, refresh);
      window.removeEventListener("online", refresh);
      window.removeEventListener("focus", refresh);
      client.removeQueries({ queryKey: key(owner, relay), exact: true });
    };
  }, [owner, relay, enabled, client]);
}

export function useHostStartProgress() {
  const { activeCommunity } = useCommunities();
  const { data: identity } = useIdentityQuery();
  return (
    useQuery({
      queryKey: key(identity?.pubkey, activeCommunity?.relayUrl),
      queryFn: async () => empty,
      enabled: false,
    }).data ?? empty
  );
}
