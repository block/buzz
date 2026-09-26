import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";

import { configureSiteRunbooksScope } from "@/features/site-runbook/lib/store";

import {
  configurePlaygroundScope,
  getPlaygroundStore,
  subscribePlayground,
} from "./lib/sessions";

export function usePlaygroundSessions() {
  const identityQuery = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const pubkey = identityQuery.data?.pubkey ?? "";
  const relayUrl = activeCommunity?.relayUrl ?? "";

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    configurePlaygroundScope(pubkey, relayUrl);
    configureSiteRunbooksScope(pubkey, relayUrl);
  }, [pubkey, relayUrl]);

  return React.useSyncExternalStore(
    subscribePlayground,
    getPlaygroundStore,
    getPlaygroundStore,
  );
}
