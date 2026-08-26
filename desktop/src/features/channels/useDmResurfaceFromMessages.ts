import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  getChannelMembers,
  type OpenDmInput,
} from "@/shared/api/tauriChannels";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { relayEventChannelId } from "./dmResurface";
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
 * untagged CLI/agent DMs resurface too. The subscription re-registers whenever
 * the hidden set changes.
 */
export function useDmResurfaceFromMessages({
  pubkey,
  relayUrl,
  reopen,
}: UseDmResurfaceFromMessagesOptions) {
  const hiddenDmIds = useHiddenDmIds(pubkey);
  // Coalesce per channel: the reopen action is idempotent, so concurrent
  // messages for the same DM share one in-flight attempt. `retry` records that
  // a follower event arrived while the attempt was in flight, so a failed
  // reopen re-runs instead of silently dropping that follower.
  const pendingChannelsRef = React.useRef(
    new Map<string, { retry: boolean }>(),
  );
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
    pendingChannelsRef.current.clear();
    if (!expectedSignerPubkey || !expectedRelayUrl || channelIds.length === 0) {
      return;
    }

    const hiddenDmIdSet = new Set(channelIds);
    let disposed = false;
    let unsubscribe: (() => Promise<void>) | undefined;
    const isCurrent = () => !disposed && generationRef.current === generation;

    // Latest event seen per channel drives the in-flight/retry attempt so a
    // coalesced follower reopens from a real event, not a captured stale one.
    const latestEventByChannel = new Map<string, RelayEvent>();

    const attempt = async (channelId: string) => {
      const state = { retry: false };
      pendingChannelsRef.current.set(channelId, state);
      try {
        do {
          state.retry = false;
          const event = latestEventByChannel.get(channelId);
          if (!event) return;
          try {
            await resurfaceHiddenDmMessage({
              event,
              expectedRelayUrl,
              expectedSignerPubkey,
              hiddenDmIds: hiddenDmIdSet,
              fetchMembers: getChannelMembers,
              isCurrent,
              reopen: reopenLatest,
            });
            return;
          } catch (error) {
            if (isCurrent()) {
              console.error("Failed to resurface hidden DM", channelId, error);
            }
          }
        } while (state.retry && isCurrent());
      } finally {
        pendingChannelsRef.current.delete(channelId);
      }
    };

    const handleEvent = (event: RelayEvent) => {
      if (!isCurrent()) return;
      const channelId = relayEventChannelId(event);
      if (!channelId || !hiddenDmIdSet.has(channelId)) return;
      latestEventByChannel.set(channelId, event);
      const pending = pendingChannelsRef.current.get(channelId);
      if (pending) {
        pending.retry = true;
        return;
      }
      void attempt(channelId);
    };

    void relayClient
      .subscribeLive(
        {
          kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
          "#h": channelIds,
          since: Math.floor(Date.now() / 1_000) - 5,
          limit: 100,
        },
        handleEvent,
      )
      .then((dispose) => {
        if (!isCurrent()) {
          void dispose().catch(() => {});
          return;
        }
        unsubscribe = dispose;
      })
      .catch((error) => {
        if (isCurrent()) {
          console.error("Failed to subscribe to hidden DM activity", error);
        }
      });

    return () => {
      disposed = true;
      generationRef.current += 1;
      void unsubscribe?.().catch(() => {});
    };
  }, [pubkey, relayUrl, hiddenDmKey]);
}
