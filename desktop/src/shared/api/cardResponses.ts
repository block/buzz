/**
 * Publish + subscribe helpers for kind:40009 interactive-card responses.
 *
 * Lives outside `relayClientSession.ts` (size-ratcheted) and composes its
 * public surface: `publishEvent` for the signed check-off and `subscribeLive`
 * for the per-card response stream.
 */

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CARD_RESPONSE } from "@/shared/constants/kinds";

/**
 * Publish a kind:40009 check-off response for an interactive card item,
 * signed with the current user's key so the click is attributable.
 * Resolves with the signed event once the relay acknowledges it.
 */
export async function sendCardResponse(
  channelId: string,
  cardEventId: string,
  itemId: string,
  done: boolean,
): Promise<RelayEvent> {
  const event = await signRelayEvent({
    kind: KIND_CARD_RESPONSE,
    content: JSON.stringify({ done }),
    tags: [
      ["h", channelId],
      ["e", cardEventId],
      ["item", itemId],
    ],
  });

  return relayClient.publishEvent(
    event,
    "Timed out while updating the to-do item.",
    "Failed to update the to-do item.",
  );
}

/**
 * Subscribe to kind:40009 responses for one card, with history replay so a
 * reload reconstructs card state. Scoped by `#e` (the card's event id) so
 * regular channel traffic never reaches the card's fold.
 */
export async function subscribeToCardResponses(
  channelId: string,
  cardEventId: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_CARD_RESPONSE],
      "#h": [channelId],
      "#e": [cardEventId],
      limit: 500,
    },
    onEvent,
  );
}
