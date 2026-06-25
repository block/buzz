import type { AgentConversation } from "@/features/agents/agentConversations";
import { collectMessageMentionPubkeys } from "@/features/messages/lib/formatTimelineMessages";
import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import type { ManagedAgent, RelayAgent, RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const AGENT_STATUS_REACTION_EMOJIS = new Set(["👀", "💬"]);
const AGENT_PARTICIPANT_PREVIEW_LIMIT = 3;

export type AgentConversationParticipant = {
  avatarUrl: string | null;
  canMessage: boolean;
  displayName: string;
  pubkey: string;
};

type KnownAgentParticipant = {
  canMessage: boolean;
  displayName: string;
  pubkey: string;
};

export function uniqueMessages(messages: TimelineMessage[]) {
  const byId = new Map<string, TimelineMessage>();
  for (const message of messages) {
    byId.set(message.id, message);
  }
  return [...byId.values()].sort((a, b) => a.createdAt - b.createdAt);
}

export function flattenConversationMessages(messages: TimelineMessage[]) {
  return messages.map((message) => ({
    ...message,
    depth: 0,
    parentId: null,
    rootId: null,
  }));
}

function getAgentParticipantPreview(
  participants: readonly AgentConversationParticipant[],
) {
  const visibleParticipants = participants.slice(
    0,
    AGENT_PARTICIPANT_PREVIEW_LIMIT,
  );

  return {
    hiddenCount: Math.max(
      0,
      participants.length - AGENT_PARTICIPANT_PREVIEW_LIMIT,
    ),
    visibleParticipants,
  };
}

export function formatAgentParticipantNames(
  participants: readonly AgentConversationParticipant[],
) {
  const { hiddenCount, visibleParticipants } =
    getAgentParticipantPreview(participants);
  const names = visibleParticipants.map(
    (participant) => participant.displayName,
  );

  return hiddenCount > 0
    ? [...names, `+${hiddenCount} more`].join(", ")
    : names.join(", ");
}

export function isConversationMessage(
  message: TimelineMessage,
  conversation: AgentConversation,
) {
  return (
    message.id === conversation.threadRootId ||
    message.id === conversation.agentReply.id ||
    message.rootId === conversation.threadRootId ||
    message.parentId === conversation.threadRootId
  );
}

export function formatAgentMentionList(names: readonly string[]) {
  const mentions = names.map((name) => `@${name}`);

  if (mentions.length === 0) {
    return "this agent";
  }

  if (mentions.length === 1) {
    return mentions[0];
  }

  if (mentions.length === 2) {
    return `${mentions[0]} and ${mentions[1]}`;
  }

  return `${mentions.slice(0, -1).join(", ")}, and ${
    mentions[mentions.length - 1]
  }`;
}

export function getLatestRelayMessageEvent(events: RelayEvent[]) {
  return events.reduce<RelayEvent | null>((latest, event) => {
    if (!latest || event.created_at > latest.created_at) {
      return event;
    }

    return latest;
  }, null);
}

function stripAgentStatusReactionUsers(
  reaction: TimelineReaction,
  agentPubkeys: ReadonlySet<string>,
): TimelineReaction | null {
  if (!AGENT_STATUS_REACTION_EMOJIS.has(reaction.emoji)) {
    return reaction;
  }

  const remainingUsers = reaction.users.filter(
    (user) => !agentPubkeys.has(normalizePubkey(user.pubkey)),
  );
  const removedCount = reaction.users.length - remainingUsers.length;
  if (removedCount <= 0) {
    return reaction;
  }

  const nextCount = Math.max(0, reaction.count - removedCount);
  if (nextCount === 0) {
    return null;
  }

  return {
    ...reaction,
    count: nextCount,
    users: remainingUsers,
  };
}

export function stripAgentStatusReactions(
  message: TimelineMessage,
  agentPubkeys: ReadonlySet<string>,
) {
  if (!message.reactions?.length || agentPubkeys.size === 0) {
    return message;
  }

  let didChange = false;
  const reactions = message.reactions
    .map((reaction) => {
      const nextReaction = stripAgentStatusReactionUsers(
        reaction,
        agentPubkeys,
      );
      if (nextReaction !== reaction) {
        didChange = true;
      }
      return nextReaction;
    })
    .filter((reaction): reaction is TimelineReaction => reaction !== null);

  if (!didChange) {
    return message;
  }

  return {
    ...message,
    reactions: reactions.length > 0 ? reactions : undefined,
  };
}

function isRelayAgentMessageable(agent: RelayAgent) {
  return agent.respondTo === "anyone";
}

export function normalizeRecapTextForComparison(
  value: string | null | undefined,
) {
  return (value ?? "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
}

export function buildKnownAgentParticipants({
  conversation,
  managedAgents,
  relayAgents,
}: {
  conversation: AgentConversation;
  managedAgents: ManagedAgent[] | undefined;
  relayAgents: RelayAgent[] | undefined;
}) {
  const participants = new Map<string, KnownAgentParticipant>();
  const add = (participant: KnownAgentParticipant) => {
    const normalized = normalizePubkey(participant.pubkey);
    if (!normalized) {
      return;
    }

    const current = participants.get(normalized);
    participants.set(normalized, {
      canMessage: current?.canMessage || participant.canMessage,
      displayName:
        current?.displayName && current.displayName !== current.pubkey
          ? current.displayName
          : participant.displayName,
      pubkey: current?.pubkey ?? participant.pubkey,
    });
  };

  for (const agent of managedAgents ?? []) {
    add({
      canMessage: true,
      displayName: agent.name,
      pubkey: agent.pubkey,
    });
  }

  for (const agent of relayAgents ?? []) {
    add({
      canMessage: isRelayAgentMessageable(agent),
      displayName: agent.name,
      pubkey: agent.pubkey,
    });
  }

  if (!participants.has(normalizePubkey(conversation.agentPubkey))) {
    add({
      canMessage: true,
      displayName: conversation.agentName,
      pubkey: conversation.agentPubkey,
    });
  }

  return participants;
}

export function getKnownAgentPubkeysInMessages(
  messages: readonly TimelineMessage[],
  knownAgents: ReadonlyMap<string, KnownAgentParticipant>,
) {
  const pubkeys: string[] = [];
  const add = (pubkey: string | null | undefined) => {
    if (!pubkey) {
      return;
    }

    const normalized = normalizePubkey(pubkey);
    if (
      normalized &&
      knownAgents.has(normalized) &&
      !pubkeys.some((current) => normalizePubkey(current) === normalized)
    ) {
      pubkeys.push(knownAgents.get(normalized)?.pubkey ?? pubkey);
    }
  };

  for (const message of messages) {
    add(message.pubkey);
  }
  for (const pubkey of collectMessageMentionPubkeys([...messages])) {
    add(pubkey);
  }

  return pubkeys;
}

export function collectTimelineMessageAuthorPubkeys(
  messages: readonly TimelineMessage[],
) {
  return messages
    .map((message) => message.pubkey)
    .filter((pubkey): pubkey is string => Boolean(pubkey));
}
