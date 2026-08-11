import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Eyes reaction buzz-acp stamps on queued/triggering messages. */
const SEEN_REACTION = "👀";

export type AgentWorkingAnchor = {
  messageId: string;
  /** Working agent pubkeys tied to this message (stable, lowercased). */
  agentPubkeys: string[];
};

/**
 * Resolve which timeline messages should host an in-progress agent indicator.
 *
 * Prefer messages that already carry a 👀 reaction from a working agent (the
 * harness stamps that on the triggering event). If a working agent has no 👀
 * yet (race before the reaction lands), fall back to the newest non-agent
 * message in `messages` so the indicator still sits under the human prompt.
 */
export function resolveAgentWorkingAnchors(
  messages: readonly Pick<
    TimelineMessage,
    "id" | "reactions" | "isAgent" | "createdAt"
  >[],
  workingBotPubkeys: readonly string[],
): AgentWorkingAnchor[] {
  if (workingBotPubkeys.length === 0 || messages.length === 0) {
    return [];
  }

  const working = new Set(
    workingBotPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  const byMessage = new Map<string, Set<string>>();
  const anchoredAgents = new Set<string>();

  for (const message of messages) {
    for (const reaction of message.reactions ?? []) {
      if (reaction.emoji !== SEEN_REACTION) {
        continue;
      }
      for (const user of reaction.users) {
        const key = normalizePubkey(user.pubkey);
        if (!working.has(key)) {
          continue;
        }
        let set = byMessage.get(message.id);
        if (!set) {
          set = new Set();
          byMessage.set(message.id, set);
        }
        set.add(key);
        anchoredAgents.add(key);
      }
    }
  }

  // Fallback: agents still working but 👀 not visible yet → newest human msg.
  const unanchored = [...working].filter((key) => !anchoredAgents.has(key));
  if (unanchored.length > 0) {
    let fallbackId: string | null = null;
    let fallbackCreatedAt = Number.NEGATIVE_INFINITY;
    for (const message of messages) {
      if (message.isAgent) {
        continue;
      }
      if (message.createdAt >= fallbackCreatedAt) {
        fallbackCreatedAt = message.createdAt;
        fallbackId = message.id;
      }
    }
    if (fallbackId) {
      let set = byMessage.get(fallbackId);
      if (!set) {
        set = new Set();
        byMessage.set(fallbackId, set);
      }
      for (const key of unanchored) {
        set.add(key);
      }
    }
  }

  // Preserve message order from the input list.
  const order = new Map(messages.map((message, index) => [message.id, index]));
  return [...byMessage.entries()]
    .map(([messageId, agentPubkeys]) => ({
      messageId,
      agentPubkeys: [...agentPubkeys].sort(),
    }))
    .sort(
      (a, b) => (order.get(a.messageId) ?? 0) - (order.get(b.messageId) ?? 0),
    );
}
