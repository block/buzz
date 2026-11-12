import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ChannelType } from "@/shared/api/types";

export const DM_THREAD_AGENT_MENTION_ERROR =
  "Agents must already be in a DM to be mentioned in its threads. Start a new conversation that includes the agent.";
export const DM_THREAD_MEMBERS_LOADING_ERROR =
  "Checking conversation members. Try again in a moment.";

/**
 * Why a DM thread reply may not mention an agent, or null when it may.
 *
 * A DM's participant set is fixed at creation, so a thread reply can only
 * mention agents already in it — persona mentions (which would create a new
 * agent) are always refused.
 */
export function dmThreadAgentMentionError({
  trimmed,
  isThreadReply,
  channelType,
  extractMentionPersonas,
  extractMentionPubkeys,
  isAgentPubkey,
  hasResolvedMembers,
  memberPubkeys,
}: {
  trimmed: string;
  isThreadReply: boolean;
  channelType: ChannelType | null;
  extractMentionPersonas: (text: string) => unknown[];
  extractMentionPubkeys: (text: string) => string[];
  isAgentPubkey: (pubkey: string) => boolean;
  hasResolvedMembers: boolean;
  memberPubkeys: ReadonlySet<string>;
}): string | null {
  if (channelType !== "dm" || !isThreadReply) {
    return null;
  }

  if (extractMentionPersonas(trimmed).length > 0) {
    return DM_THREAD_AGENT_MENTION_ERROR;
  }

  const agentPubkeys = extractMentionPubkeys(trimmed).filter(isAgentPubkey);
  if (agentPubkeys.length === 0) {
    return null;
  }

  if (!hasResolvedMembers) {
    return DM_THREAD_MEMBERS_LOADING_ERROR;
  }

  return agentPubkeys.some(
    (pubkey) => !memberPubkeys.has(normalizePubkey(pubkey)),
  )
    ? DM_THREAD_AGENT_MENTION_ERROR
    : null;
}
