import { useHostStartReceiver } from "./useHostStart";
import { useHostPresence } from "./useHostPresence";
import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { ReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  hostQueryKey,
  type HostBridge,
  type HostSnapshot,
  type HostReport,
  type LocalHost,
} from "./registration";

import { createHostRegistrationLifecycle } from "./registrationLifecycle";
import { createHostPublicationJournal } from "./pendingPublication";
import { hostNativeDrain } from "./hostNativeDrain";

export const HOST_REFRESH = "buzz:refresh-hosts";
const EMPTY: HostSnapshot = { rows: [], checking: true };

export function useHostSnapshot() {
  const { activeCommunity } = useCommunities();
  const { data: identity } = useIdentityQuery();
  return (
    useQuery({
      queryKey: hostQueryKey(activeCommunity?.relayUrl, identity?.pubkey),
      queryFn: async () => EMPTY,
      enabled: false,
    }).data ?? EMPTY
  );
}

/** App lifecycle, not Agents-page lifecycle. No cross-community singleton cache. */
export function useHostRegistration(
  owner: string | undefined,
  relayUrl: string | undefined,
) {
  const queryClient = useQueryClient();
  const snapshot =
    useQuery({
      queryKey: hostQueryKey(relayUrl, owner),
      queryFn: async () => EMPTY,
      enabled: false,
    }).data ?? EMPTY;
  const localRegistration = snapshot.rows.find(
    (row) => row.host === snapshot.local?.host,
  )?.registration;
  useHostPresence(owner, relayUrl, localRegistration);
  useHostStartReceiver(owner, relayUrl, !!localRegistration);
  useEffect(() => {
    if (!owner || !relayUrl) return;
    const key = hostQueryKey(relayUrl, owner);
    const bridge: HostBridge = {
      local: () =>
        invokeTauri<LocalHost>("get_local_host", { expectedOwner: owner }),
      registration: () =>
        invokeTauri<RelayEvent>("create_host_registration", {
          expectedOwner: owner,
        }),
      report: (registration) =>
        invokeTauri<RelayEvent>("create_host_report", {
          expectedOwner: owner,
          registration,
        }),
      inspect: (registration) =>
        invokeTauri<string>("inspect_host_registration", {
          expectedOwner: owner,
          registration,
        }),
      decode: (registration, report) =>
        invokeTauri<HostReport>("decode_host_report", {
          expectedOwner: owner,
          registration,
          report,
        }),
    };
    const lifecycle = createHostRegistrationLifecycle({
      owner,
      bridge,
      journal: createHostPublicationJournal(relayUrl, owner),
      connect: () => {
        const client = new ReadOnlyRelayClient(relayUrl);
        return {
          // Only HTTP preserves the before_id keyset extension. Owner signing
          // stays native; the socket is used solely for acknowledged writes.
          fetchEvents: (filter) =>
            invokeTauri<RelayEvent[]>("get_host_history_page", {
              expectedOwner: owner,
              relayUrl,
              filter,
            }),
          publishEvent: (event) => client.publishEvent(event),
          disconnect: () => client.disconnect(),
        };
      },
      now: () => Math.floor(Date.now() / 1000),
      after: hostNativeDrain.wait(),
      checking: () =>
        queryClient.setQueryData<HostSnapshot>(key, (previous) => ({
          ...(previous ?? EMPTY),
          checking: true,
        })),
      success: (result) => {
        queryClient.setQueryData(key, result);
      },
      failure: () =>
        queryClient.setQueryData<HostSnapshot>(key, (previous) => ({
          ...(previous ?? EMPTY),
          checking: false,
          // Native/transport diagnostics may contain private paths or payloads.
          error:
            "Could not verify host history or relay acceptance; retry shortly",
        })),
    });
    const refresh = () => {
      void lifecycle.refresh();
    };
    refresh();
    const timer = window.setInterval(refresh, 30_000);
    window.addEventListener(HOST_REFRESH, refresh);
    window.addEventListener("online", refresh);
    window.addEventListener("focus", refresh);
    return () => {
      hostNativeDrain.hold(lifecycle.stop());
      window.clearInterval(timer);
      window.removeEventListener(HOST_REFRESH, refresh);
      window.removeEventListener("online", refresh);
      window.removeEventListener("focus", refresh);
      // Decrypted private metadata must not survive an identity/community switch.
      queryClient.removeQueries({ queryKey: key, exact: true });
    };
  }, [owner, relayUrl, queryClient]);
}
