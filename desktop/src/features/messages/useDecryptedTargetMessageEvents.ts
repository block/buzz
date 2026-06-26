import * as React from "react";

import {
  dmPeerPubkey,
  makeDmIngestDecryptor,
} from "@/features/messages/lib/dmCrypto";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Decrypt deep-link / search-hit target events before they reach the rendered
 * timeline.
 *
 * The route layer fetches deep-link targets, thread ancestors, and search hits
 * as RAW RelayEvents (no decrypt) and threads them down as `targetMessageEvents`.
 * `ChannelScreen` merges those into the rendered list — so for a DM, where the
 * body is NIP-44 v2 ciphertext, the raw target would render garbled and, on an
 * id collision, CLOBBER the decrypted cache copy (the merge keeps the last
 * writer).
 *
 * This is the single choke point for that whole class: every contributor flows
 * through the one `targetMessageEvents` array, so decrypting it here once covers
 * the synchronous mount-seed, the cached-search-hit path, and the async fetch
 * path uniformly — a decrypt split across the individual setters would miss the
 * synchronous mount-seed and leak on a search-jump first paint.
 *
 * Outside a DM there is nothing to decrypt, so the events pass through
 * synchronously with no held-back frame. Inside a DM the events are held back
 * (empty) until the async decrypt resolves — INCLUDING the cold-start window
 * before identity resolves (`selfPubkey === undefined`), where decrypt is not
 * yet possible. Unlike the cache path, these targets are component state merged
 * directly onto the rendered timeline with no `[...,null]` vs `[...,pubkey]`
 * bucket-orphaning to discard a raw write, so a DM must never pass a target
 * through un-decrypted — it holds back until identity lands and decrypt runs.
 */

/**
 * What the rendered timeline should see for the fetched route/search targets,
 * given the decrypted set the effect has produced so far.
 *
 * A 2-party DM is the peer-encrypted case: its targets are NIP-44 ciphertext
 * and must be held back (the decrypted set is empty until decrypt resolves, and
 * stays empty during the pre-identity cold-start window where decrypt is not
 * yet possible) so raw ciphertext never reaches the render merge. Everything
 * else — non-DM channels and group DMs (>2 participants), which are not peer-
 * encrypted — passes through synchronously so a deep-link splices its target on
 * first paint with no held-back frame.
 *
 * Keyed on channel shape (`channelType` + participant count), NOT on whether a
 * peer pubkey currently resolves: the peer resolves only once `selfPubkey` is
 * known, so keying on it would pass raw ciphertext through during cold start —
 * the leak this hook exists to close.
 */
export function resolveTargetRenderEvents(
  activeChannel: Channel | null,
  targetMessageEvents: RelayEvent[],
  decryptedEvents: RelayEvent[],
): RelayEvent[] {
  const isTwoPartyDm =
    activeChannel?.channelType === "dm" &&
    activeChannel.participantPubkeys.length === 2;
  return isTwoPartyDm ? decryptedEvents : targetMessageEvents;
}

export function useDecryptedTargetMessageEvents(
  activeChannel: Channel | null,
  targetMessageEvents: RelayEvent[],
  selfPubkey: string | undefined,
): RelayEvent[] {
  const canDecrypt =
    activeChannel !== null && dmPeerPubkey(activeChannel, selfPubkey) !== null;

  const [decryptedEvents, setDecryptedEvents] = React.useState<RelayEvent[]>(
    [],
  );

  React.useEffect(() => {
    setDecryptedEvents([]);
  }, [activeChannel, selfPubkey]);

  React.useEffect(() => {
    if (!canDecrypt || targetMessageEvents.length === 0) {
      return;
    }

    let isCancelled = false;
    const decryptIngested = makeDmIngestDecryptor(activeChannel, selfPubkey);
    void decryptIngested(targetMessageEvents).then((decrypted) => {
      if (!isCancelled) {
        setDecryptedEvents(decrypted);
      }
    });

    return () => {
      isCancelled = true;
    };
  }, [activeChannel, canDecrypt, selfPubkey, targetMessageEvents]);

  return resolveTargetRenderEvents(
    activeChannel,
    targetMessageEvents,
    decryptedEvents,
  );
}
