import { getChannelDetail } from "@/features/channels/lib/channelDescription";
import { isEphemeralChannel } from "@/features/channels/lib/ephemeralChannel";
import type { TimelineMessage } from "@/features/messages/types";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import type { Channel } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

export function shouldUseFocusIdleDrawer({
  channelManagementOpen,
  hasAgentSession,
  hasIdleAuxiliaryPanel,
  hasIdlePanelCloseHandler,
  hasProfilePanel,
  hasThreadSurface,
  overrideThread = false,
  useSplitAuxiliaryPane,
}: {
  channelManagementOpen: boolean;
  hasAgentSession: boolean;
  hasIdleAuxiliaryPanel: boolean;
  hasIdlePanelCloseHandler: boolean;
  hasProfilePanel: boolean;
  hasThreadSurface: boolean;
  overrideThread?: boolean;
  useSplitAuxiliaryPane: boolean;
}): boolean {
  return (
    (useSplitAuxiliaryPane || overrideThread) &&
    !channelManagementOpen &&
    !hasAgentSession &&
    !hasProfilePanel &&
    (!hasThreadSurface || overrideThread) &&
    hasIdleAuxiliaryPanel &&
    hasIdlePanelCloseHandler
  );
}

export function getChannelIntroKind(
  channel: Channel,
  projectHome = false,
): string {
  if (projectHome) {
    return "project channel";
  }

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
  return getChannelDetail(channel);
}

/** Whether a caller-owned auxiliary sheet should render ahead of a thread. */
export function shouldPrioritizeIdleAuxiliary(
  overrideThread: boolean,
  hasIdleAuxiliary: boolean,
) {
  return overrideThread && hasIdleAuxiliary;
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

export function mentionsKnownAgent(
  mentionPubkeys: string[],
  knownAgentPubkeys: ReadonlySet<string>,
) {
  return mentionPubkeys.some((pubkey) =>
    knownAgentPubkeys.has(pubkey.toLowerCase()),
  );
}

export function selectThreadComposerBotTypingPubkeys(
  entries: TypingIndicatorEntry[],
  threadHeadId: string | null,
) {
  if (!threadHeadId) return [];
  return entries
    .filter((entry) => entry.threadHeadId === threadHeadId)
    .map((entry) => entry.pubkey)
    .filter(
      (pubkey, index, all) =>
        all.findIndex(
          (candidate) => candidate.toLowerCase() === pubkey.toLowerCase(),
        ) === index,
    );
}
