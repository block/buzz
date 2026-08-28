import type { Channel } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { DM_NOTIFIABLE_EVENT_KINDS } from "./isDmNotifiableKind";

export function channelCatchUpEventKinds(
  channelType: Channel["channelType"] | undefined,
) {
  return channelType === "dm"
    ? DM_NOTIFIABLE_EVENT_KINDS
    : CHANNEL_MESSAGE_EVENT_KINDS;
}

export function shouldRecordLiveUnread(
  channelId: string,
  activeChannelId: string | null,
  isThreadedReply: boolean,
): boolean {
  return !isThreadedReply || channelId !== activeChannelId;
}
