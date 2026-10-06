import type {
  ProjectsConversationOpener,
  StoredProjectsAgentConversation,
} from "@/features/projects/lib/projectAgentConversationStorage";
import type { AddChannelMembersInput, Channel } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function projectAgentMembershipInput({
  channelId,
  agentPubkey,
  relayScope,
  signerScope,
}: {
  channelId: string;
  agentPubkey: string;
  relayScope: string | null;
  signerScope: string | null;
}): AddChannelMembersInput {
  return {
    channelId,
    pubkeys: [agentPubkey],
    role: "bot",
    expectedRelayUrl: relayScope ?? undefined,
    expectedSignerPubkey: signerScope ?? undefined,
  };
}

/**
 * True when `event` is the conversation opener or comes after it in the
 * timeline's `(created_at, event_id)` ordering (`compareRelayOrder` in
 * `channelWindowStore.ts`). A bare timestamp cannot make this call — every
 * unrelated event sharing the opener's second would pass — which is why the
 * opener's exact event id participates. Id equality is checked first so the
 * opener itself is admitted even against a legacy persisted `createdAt` that
 * trails the signed event's by a second.
 *
 * Event ids are random within a second, so a fast reply signed in the
 * opener's own second can sort "older" than the opener in relay order. A
 * reply carries an `e` tag naming the opener — causality the id ordering
 * cannot fake — so events that reference the opener are always admitted.
 */
export function isAtOrAfterConversationOpener(
  event: { created_at: number; id: string; tags?: readonly string[][] },
  opener: ProjectsConversationOpener,
): boolean {
  return (
    event.id === opener.eventId ||
    event.created_at > opener.createdAt ||
    (event.created_at === opener.createdAt && event.id <= opener.eventId) ||
    (event.tags?.some((tag) => tag[0] === "e" && tag[1] === opener.eventId) ??
      false)
  );
}

/**
 * Restores an inline Projects conversation strictly from a pointer this
 * feature persisted earlier. DM channels are reused across the app, so
 * inferring a conversation from "the most recent agent DM" would surface
 * unrelated chat history on the Projects page — never infer one here.
 *
 * A stored channel id alone is not proof either: ids can collide across
 * relays, and a stale pointer could name a group channel. The channel must
 * be a DM whose participants are exactly the agent and the current user —
 * anything else renders someone else's conversation and is not restorable.
 */
export function restoreProjectsAgentConversation<
  Agent extends { pubkey: string },
>({
  stored,
  channels,
  candidates,
  currentPubkey,
  homeChannelId,
}: {
  stored: StoredProjectsAgentConversation | null;
  channels: readonly Channel[];
  candidates: readonly Agent[];
  currentPubkey: string | null;
  /** When set, a stored pointer to this project channel (not a DM) can restore. */
  homeChannelId?: string | null;
}): {
  channel: Channel;
  agent: Agent;
  opener: ProjectsConversationOpener;
} | null {
  // Only pointers anchored to a concrete opener event are restorable —
  // anything weaker would render DM history that predates the conversation.
  if (!stored || !currentPubkey) return null;
  const channel = channels.find(
    (candidate) => candidate.id === stored.channelId,
  );
  const agentPubkey = normalizePubkey(stored.agentPubkey);
  const agent = candidates.find(
    (candidate) => candidate.pubkey === agentPubkey,
  );
  if (!channel || !agent) return null;
  const self = normalizePubkey(currentPubkey);
  if (homeChannelId && channel.id === homeChannelId) {
    // Project-home chat lives on the project channel. Membership is the
    // restore proof — the channel is not a 1:1 DM.
    if (!channel.isMember) return null;
    return { agent, channel, opener: stored.opener };
  }
  if (channel.channelType !== "dm") return null;
  const participants = channel.participantPubkeys.map(normalizePubkey);
  const hasAgent = participants.includes(agentPubkey);
  // The contract is participants === {agent, self}: requiring the current
  // user's own membership matters as much as rejecting strangers — a stored
  // pointer must never restore a channel not proven to include the
  // signed-in user (e.g. a stale pointer naming an agent-only channel).
  const hasSelf = participants.includes(self);
  const hasStranger = participants.some(
    (participant) => participant !== agentPubkey && participant !== self,
  );
  if (!hasAgent || !hasSelf || hasStranger) return null;
  return { agent, channel, opener: stored.opener };
}

/**
 * Chat rows for the inline Projects thread: plain messages only, and nothing
 * ordered before the conversation's opener event — the backing DM may hold
 * unrelated history from ordinary DM usage, including history from the
 * opener's own second.
 */
export function visibleConversationMessages<
  Event extends { kind: number; created_at: number; id: string },
>(events: readonly Event[], opener: ProjectsConversationOpener): Event[] {
  return events
    .filter(
      (event) =>
        (event.kind === KIND_STREAM_MESSAGE ||
          event.kind === KIND_STREAM_MESSAGE_V2) &&
        isAtOrAfterConversationOpener(event, opener),
    )
    .sort((left, right) => left.created_at - right.created_at);
}

/**
 * Combines the backing DM's root events with separately queried thread
 * replies. Keep this chronological: appending the reply query after the root
 * query would otherwise render every question before every agent answer.
 */
export function mergeProjectAgentConversationEvents<
  Event extends { id: string; created_at: number },
>(rootEvents: readonly Event[], replyEvents: readonly Event[]): Event[] {
  return [
    ...new Map(
      [...rootEvents, ...replyEvents].map((event) => [event.id, event]),
    ).values(),
  ].sort((left, right) => left.created_at - right.created_at);
}

/**
 * Runs the Projects agent submit sequence with every relay side effect bound
 * to the tenant scope the caller captured before the first await.
 *
 * The sequence suspends twice (managed-agent startup, DM open) and a
 * community switch during either suspension does not cancel this callback —
 * remounting only removes the UI. Binding is therefore delegated to the
 * scoped APIs themselves: `startAgent`, `openDm`, and `send` all receive
 * the captured `expectedRelayUrl`/`expectedSignerPubkey`, and the backing
 * commands fail closed when the active community or identity no longer
 * matches — a stale callback can neither activate the agent pair in the new
 * tenant, nor open a DM there, nor publish (or re-sign) the captured
 * tenant's context. Startup is scoped too because starting a managed agent
 * activates the (agent, relay) pair with channel/tool access — a side effect
 * in its own right, not mere preflight. The signer scope exists because the
 * relay URL and the signing keys change under separate locks during a
 * switch: pinning only the relay would still let the new identity sign the
 * old tenant's content. This function never re-reads the active scope after
 * capture — doing so would race the very switch it guards against.
 *
 * Follow-up sends carry `parentEventId = opener.eventId`: a follow-up signed
 * in the opener's own second gets a random event id, and roughly half of
 * those sort on the rejected side of the same-second id tiebreak in
 * `isAtOrAfterConversationOpener`. The causal reply reference — which the
 * comparator always admits — is what keeps an immediate follow-up visible;
 * id luck cannot.
 */
export async function submitProjectAgentMessage<Ch extends { id: string }>({
  agent,
  conversation,
  content,
  mentionPubkeys,
  mediaTags,
  relayScope,
  signerScope,
  homeChannel,
  startAgent,
  openDm,
  send,
}: {
  agent: { pubkey: string; isManaged: boolean; isActive: boolean | null };
  conversation: { channel: Ch; opener: ProjectsConversationOpener } | null;
  content: string;
  mentionPubkeys: string[];
  mediaTags?: string[][];
  /** Community relay captured before the first await; null when the
   * community has no relay identity (no tenant boundary to protect). */
  relayScope: string | null;
  /** Signing identity (owner pubkey, hex) captured together with
   * `relayScope`; null when unknown. */
  signerScope: string | null;
  /** When set, the first message lands here instead of opening a 1:1 DM. */
  homeChannel?: Ch | null;
  startAgent: (input: {
    pubkey: string;
    expectedRelayUrl?: string;
    expectedSignerPubkey?: string;
  }) => Promise<unknown>;
  openDm: (input: {
    pubkeys: string[];
    expectedRelayUrl?: string;
    expectedSignerPubkey?: string;
  }) => Promise<Ch>;
  send: (request: {
    channelId: string;
    content: string;
    mentionPubkeys: string[];
    mediaTags?: string[][];
    parentEventId?: string;
    expectedRelayUrl?: string;
    expectedSignerPubkey?: string;
  }) => Promise<{ eventId: string; createdAt: number }>;
}): Promise<{ channel: Ch; sent: { eventId: string; createdAt: number } }> {
  const expectedRelayUrl = relayScope ?? undefined;
  const expectedSignerPubkey = signerScope ?? undefined;
  if (agent.isManaged && !agent.isActive) {
    await startAgent({
      pubkey: agent.pubkey,
      expectedRelayUrl,
      expectedSignerPubkey,
    });
  }
  const channel =
    conversation?.channel ??
    homeChannel ??
    (await openDm({
      pubkeys: [agent.pubkey],
      expectedRelayUrl,
      expectedSignerPubkey,
    }));
  const sent = await send({
    channelId: channel.id,
    content,
    mentionPubkeys,
    mediaTags,
    parentEventId: conversation?.opener.eventId,
    expectedRelayUrl,
    expectedSignerPubkey,
  });
  return { channel, sent };
}
