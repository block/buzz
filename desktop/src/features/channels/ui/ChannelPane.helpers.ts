import { isEphemeralChannel } from "@/features/channels/lib/ephemeralChannel";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function getChannelIntroKind(channel: Channel): string {
  const isPrivate = channel.visibility === "private";
  const isEphemeral = isEphemeralChannel(channel);

  if (isPrivate && isEphemeral) {
    return "private ephemeral channel";
  }
  if (isPrivate) {
    return "private channel";
  }
  if (isEphemeral) {
    return "ephemeral channel";
  }
  return "regular channel";
}

export function getChannelIntroDescription(channel: Channel): string | null {
  return (
    channel.topic?.trim() ||
    channel.purpose?.trim() ||
    channel.description.trim() ||
    null
  );
}

export function isWelcomeSetupSystemMessage(message: TimelineMessage) {
  if (message.kind !== KIND_SYSTEM_MESSAGE) {
    return false;
  }

  try {
    const payload = JSON.parse(message.body) as { type?: string };
    return (
      payload.type === "channel_created" || payload.type === "member_joined"
    );
  } catch {
    return false;
  }
}

export function canOpenAgentConversationInChannel({
  channel,
  publishMarker,
}: {
  channel: Channel | null;
  publishMarker?: boolean;
}) {
  if (!channel) {
    return false;
  }

  if (publishMarker === false) {
    return true;
  }

  return channel.archivedAt === null && channel.isMember;
}

export function mentionsKnownAgent(
  mentionPubkeys: string[],
  knownAgentPubkeys: ReadonlySet<string>,
) {
  return mentionPubkeys.some((pubkey) =>
    knownAgentPubkeys.has(pubkey.toLowerCase()),
  );
}

function singleKnownAgentPubkey(
  pubkeys: Iterable<string | null | undefined>,
  knownAgentPubkeys: ReadonlySet<string>,
) {
  const agentPubkeys = new Map<string, string>();

  for (const pubkey of pubkeys) {
    if (!pubkey) {
      continue;
    }

    const normalized = normalizePubkey(pubkey);
    if (!knownAgentPubkeys.has(normalized)) {
      continue;
    }

    agentPubkeys.set(normalized, pubkey);
  }

  return agentPubkeys.size === 1 ? [...agentPubkeys.values()] : [];
}

export function getDmAutoRouteAgentPubkeys({
  channel,
  currentPubkey,
  knownAgentPubkeys,
}: {
  channel: Channel | null;
  currentPubkey?: string;
  knownAgentPubkeys: ReadonlySet<string>;
}) {
  if (channel?.channelType !== "dm") {
    return [];
  }

  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return singleKnownAgentPubkey(
    channel.participantPubkeys.filter(
      (pubkey) =>
        !normalizedCurrentPubkey ||
        normalizePubkey(pubkey) !== normalizedCurrentPubkey,
    ),
    knownAgentPubkeys,
  );
}

export function mergeAutoRouteMentionPubkeys({
  autoRouteAgentPubkeys,
  mentionPubkeys,
}: {
  autoRouteAgentPubkeys: readonly string[];
  mentionPubkeys: readonly string[];
}) {
  const seenPubkeys = new Set<string>();
  const merged: string[] = [];
  const add = (pubkey: string) => {
    const normalized = normalizePubkey(pubkey);
    if (!normalized || seenPubkeys.has(normalized)) {
      return;
    }

    seenPubkeys.add(normalized);
    merged.push(pubkey);
  };

  for (const pubkey of autoRouteAgentPubkeys) {
    add(pubkey);
  }
  for (const pubkey of mentionPubkeys) {
    add(pubkey);
  }

  return merged;
}
