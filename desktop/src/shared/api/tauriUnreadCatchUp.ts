import { invokeTauri } from "@/shared/api/tauri";
import type { ObservedUnreadEvent } from "@/features/channels/unreadChannelCounts";
import type { ThreadActivityItem } from "@/features/channels/threadActivityStorage";

export type UnreadCatchUpChannel = {
  id: string;
  type: string;
  name: string;
  readAt: number | null;
};

export type UnreadCatchUpRequest = {
  channels: UnreadCatchUpChannel[];
  selfPubkey: string;
  mutedChannelIds: string[];
};

export type UnreadCatchUpChannelResult =
  | {
      status: "success";
      channelId: string;
      observedEvents: ObservedUnreadEvent[];
      maxTrigger: number;
      activityRows: ThreadActivityItem[];
      discovered: {
        participated: string[];
        authored: string[];
        mentioned: string[];
      };
    }
  | { status: "error"; channelId: string; error: string };

export function unreadCatchUp(
  request: UnreadCatchUpRequest,
): Promise<{ channels: UnreadCatchUpChannelResult[] }> {
  return invokeTauri("unread_catch_up", { request });
}
