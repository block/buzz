import * as React from "react";
import { isEphemeralChannel } from "@/features/channels/lib/ephemeralChannel";
import { useShowJoinLeaveMessages } from "@/features/messages/lib/showJoinLeaveMessages";
import type { TimelineMessage } from "@/features/messages/types";
import { isWelcomeExperienceChannel } from "@/features/onboarding/welcome";
import type { Channel } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

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

const JOIN_LEAVE_SYSTEM_TYPES = new Set([
  "member_joined",
  "member_left",
  "member_removed",
]);

/**
 * Membership-change system rows: "X joined" / "Y added X" (member_joined),
 * "X left" (member_left), and "Y removed X" (member_removed). Hidden from
 * the timeline unless the device-local "Show join and leave messages"
 * setting enables them; the events still flow so member lists stay live.
 */
export function isJoinLeaveSystemMessage(message: TimelineMessage) {
  if (message.kind !== KIND_SYSTEM_MESSAGE) {
    return false;
  }

  try {
    const payload = JSON.parse(message.body) as { type?: string };
    return (
      typeof payload.type === "string" &&
      JOIN_LEAVE_SYSTEM_TYPES.has(payload.type)
    );
  } catch {
    return false;
  }
}

/**
 * Timeline visibility filter: join/leave rows are hidden unless the
 * device-local "Show join and leave messages" setting enables them, and
 * welcome channels additionally hide their setup system rows.
 */
export function filterVisibleTimelineMessages(
  messages: TimelineMessage[],
  activeChannel: Channel | null,
  showJoinLeave: boolean,
): TimelineMessage[] {
  const hideWelcomeSetup =
    activeChannel !== null && isWelcomeExperienceChannel(activeChannel);
  if (showJoinLeave && !hideWelcomeSetup) {
    return messages;
  }
  return messages.filter(
    (message) =>
      (showJoinLeave || !isJoinLeaveSystemMessage(message)) &&
      (!hideWelcomeSetup || !isWelcomeSetupSystemMessage(message)),
  );
}

/**
 * Memoized {@link filterVisibleTimelineMessages} bound to the device-local
 * "Show join and leave messages" setting.
 */
export function useVisibleTimelineMessages(
  messages: TimelineMessage[],
  activeChannel: Channel | null,
): TimelineMessage[] {
  const showJoinLeave = useShowJoinLeaveMessages();
  return React.useMemo(
    () => filterVisibleTimelineMessages(messages, activeChannel, showJoinLeave),
    [activeChannel, messages, showJoinLeave],
  );
}

export function mentionsKnownAgent(
  mentionPubkeys: string[],
  knownAgentPubkeys: ReadonlySet<string>,
) {
  return mentionPubkeys.some((pubkey) =>
    knownAgentPubkeys.has(pubkey.toLowerCase()),
  );
}
