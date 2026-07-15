import type { StoredProjectsAgentConversation } from "@/features/projects/lib/projectAgentConversationStorage";
import type { Channel } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Restores an inline Projects conversation strictly from a pointer this
 * feature persisted earlier. DM channels are reused across the app, so
 * inferring a conversation from "the most recent agent DM" would surface
 * unrelated chat history on the Projects page — never infer one here.
 */
export function restoreProjectsAgentConversation<
  Agent extends { pubkey: string },
>({
  stored,
  channels,
  candidates,
}: {
  stored: StoredProjectsAgentConversation | null;
  channels: readonly Channel[];
  candidates: readonly Agent[];
}): { channel: Channel; agent: Agent; visibleAfter: number } | null {
  // A zero cutoff would render the DM's full history; only pointers
  // anchored to a concrete Projects prompt are restorable.
  if (!stored || stored.visibleAfter <= 0) return null;
  const channel = channels.find(
    (candidate) => candidate.id === stored.channelId,
  );
  const agentPubkey = normalizePubkey(stored.agentPubkey);
  const agent = candidates.find(
    (candidate) => candidate.pubkey === agentPubkey,
  );
  if (!channel || !agent) return null;
  return { agent, channel, visibleAfter: stored.visibleAfter };
}

/**
 * Chat rows for the inline Projects thread: plain messages only, and nothing
 * sent before the conversation cutoff — the backing DM may hold unrelated
 * history from ordinary DM usage.
 */
export function visibleConversationMessages<
  Event extends { kind: number; created_at: number },
>(events: readonly Event[], visibleAfter: number): Event[] {
  return events
    .filter(
      (event) =>
        (event.kind === KIND_STREAM_MESSAGE ||
          event.kind === KIND_STREAM_MESSAGE_V2) &&
        event.created_at >= visibleAfter,
    )
    .sort((left, right) => left.created_at - right.created_at);
}
