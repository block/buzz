import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { MAX_EXPLICIT_CHANNEL_VALUES } from "@/shared/api/relayClientShared";
import {
  getChannelMembers,
  type OpenDmInput,
} from "@/shared/api/tauriChannels";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { relayEventChannelId } from "./dmResurface";
import { createHiddenDmResurfaceCoordinator } from "./hiddenDmResurfaceCoordinator";
import { resurfaceHiddenDmMessage } from "./hiddenDmResurfaceAction";
import { useHiddenDmIds } from "./useHiddenDmIds";

type UseDmResurfaceFromMessagesOptions = {
  pubkey: string | undefined;
  relayUrl: string | undefined;
  reopen: (input: OpenDmInput) => Promise<{ id: string }>;
};

/**
 * Resurfaces a hidden DM row the moment new activity lands in it.
 *
 * The subscription is `#h`-scoped to the current hidden-DM id set: channel
 * messages carry a `channel_id`, and the relay only fans channel-scoped events
 * to channel-scoped subscriptions (`fan_out_scoped`), so a community-global
 * `#p` filter would never receive them. Scoping to `#h` also means every
 * delivered event is already for a hidden DM the reader belongs to (hiding
 * never drops membership), so no per-event visibility fetch is needed and
 * untagged CLI/agent DMs resurface too.
 *
 * The relay rejects a REQ whose aggregate explicit `#h` values exceed
 * {@link MAX_EXPLICIT_CHANNEL_VALUES}, so the hidden set is split into batches
 * of at most that size, each its own subscription. Every batch is owned by the
 * current effect generation and disposed together on hidden-set/relay/signer
 * change, so an over-limit hidden set no longer silently disables resurfacing.
 */
export function useDmResurfaceFromMessages({
  pubkey,
  relayUrl,
  reopen,
}: UseDmResurfaceFromMessagesOptions) {
  const hiddenDmIds = useHiddenDmIds(pubkey);
  const generationRef = React.useRef(0);
  const reopenLatest = React.useEffectEvent(reopen);

  // Stable dependency for the hidden-set membership, order-independent.
  const hiddenDmKey = React.useMemo(
    () => [...hiddenDmIds].sort().join(","),
    [hiddenDmIds],
  );

  React.useEffect(() => {
    const expectedSignerPubkey = pubkey?.trim().toLowerCase() ?? "";
    const expectedRelayUrl = relayUrl?.trim() ?? "";
    const channelIds = hiddenDmKey.length > 0 ? hiddenDmKey.split(",") : [];
    const generation = ++generationRef.current;
    if (!expectedSignerPubkey || !expectedRelayUrl || channelIds.length === 0) {
      return;
    }

    const hiddenDmIdSet = new Set(channelIds);
    let disposed = false;
    const unsubscribers: Array<() => Promise<void>> = [];
    const isCurrent = () => !disposed && generationRef.current === generation;

    // A coordinator owned by this generation: coalescing and cleanup touch only
    // its private map, so a torn-down generation's in-flight attempt can never
    // drop a follower coalesced onto the replacement subscription. One
    // coordinator spans every batch — the map is keyed by channel id, so which
    // batch delivered an event is irrelevant.
    const coordinator = createHiddenDmResurfaceCoordinator({
      resurface: (event) =>
        resurfaceHiddenDmMessage({
          event,
          expectedRelayUrl,
          expectedSignerPubkey,
          hiddenDmIds: hiddenDmIdSet,
          fetchMembers: getChannelMembers,
          isCurrent,
          reopen: reopenLatest,
        }),
      isCurrent,
      onError: (channelId, error) => {
        if (isCurrent()) {
          console.error("Failed to resurface hidden DM", channelId, error);
        }
      },
    });

    const handleEvent = (event: RelayEvent) => {
      if (!isCurrent()) return;
      const channelId = relayEventChannelId(event);
      if (!channelId || !hiddenDmIdSet.has(channelId)) return;
      coordinator.handle(channelId, event);
    };

    const since = Math.floor(Date.now() / 1_000) - 5;
    for (
      let start = 0;
      start < channelIds.length;
      start += MAX_EXPLICIT_CHANNEL_VALUES
    ) {
      const batch = channelIds.slice(
        start,
        start + MAX_EXPLICIT_CHANNEL_VALUES,
      );
      void relayClient
        .subscribeLive(
          {
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            "#h": batch,
            since,
            limit: 100,
          },
          handleEvent,
        )
        .then((dispose) => {
          if (!isCurrent()) {
            void dispose().catch(() => {});
            return;
          }
          unsubscribers.push(dispose);
        })
        .catch((error) => {
          if (isCurrent()) {
            console.error("Failed to subscribe to hidden DM activity", error);
          }
        });
    }

    return () => {
      disposed = true;
      generationRef.current += 1;
      for (const unsubscribe of unsubscribers) {
        void unsubscribe().catch(() => {});
      }
    };
  }, [pubkey, relayUrl, hiddenDmKey]);
}
