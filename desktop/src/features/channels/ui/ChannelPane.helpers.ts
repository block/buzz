import { isEphemeralChannel } from "@/features/channels/lib/ephemeralChannel";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel } from "@/shared/api/types";
import { KIND_SURFACE, KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

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
    channel.description?.trim() ||
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

export function isChannelCreatedSystemMessage(message: TimelineMessage) {
  if (message.kind !== KIND_SYSTEM_MESSAGE) {
    return false;
  }

  try {
    return (
      (JSON.parse(message.body) as { type?: string }).type === "channel_created"
    );
  } catch {
    return false;
  }
}

/**
 * Latest own, non-pending message eligible for ArrowUp inline editing.
 * Surfaces are excluded: they are edited via full-spec replacement (CLI/SDK),
 * so ArrowUp must not open a textarea full of JSON.
 */
export function findLastOwnEditableMessage(
  candidates: TimelineMessage[],
  currentPubkey: string,
): TimelineMessage | null {
  let best: TimelineMessage | null = null;
  for (const message of candidates) {
    if (
      message.kind === KIND_SYSTEM_MESSAGE ||
      message.kind === KIND_SURFACE ||
      message.pubkey !== currentPubkey ||
      message.pending
    ) {
      continue;
    }
    if (!best || message.createdAt >= best.createdAt) {
      best = message;
    }
  }
  return best;
}

export function mentionsKnownAgent(
  mentionPubkeys: string[],
  knownAgentPubkeys: ReadonlySet<string>,
) {
  return mentionPubkeys.some((pubkey) =>
    knownAgentPubkeys.has(pubkey.toLowerCase()),
  );
}
