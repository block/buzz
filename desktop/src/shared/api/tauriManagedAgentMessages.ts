import type { SendChannelMessageResult } from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

type RawSendChannelMessageResult = {
  event_id: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  created_at: number;
};

export async function sendManagedAgentChannelMessage(input: {
  agentPubkey: string;
  channelId: string;
  content: string;
  marker?: string;
  markerScope?: "agent" | "channel";
  mentionPubkeys?: string[];
  parentEventId?: string;
  additionalMarkers?: string[];
}): Promise<SendChannelMessageResult> {
  const response = await invokeTauri<RawSendChannelMessageResult>(
    "send_managed_agent_channel_message",
    {
      agentPubkey: input.agentPubkey,
      channelId: input.channelId,
      content: input.content,
      marker: input.marker ?? null,
      markerScope: input.markerScope ?? null,
      mentionPubkeys: input.mentionPubkeys ?? null,
      parentEventId: input.parentEventId ?? null,
      additionalMarkers: input.additionalMarkers ?? null,
    },
  );

  return {
    eventId: response.event_id,
    parentEventId: response.parent_event_id,
    rootEventId: response.root_event_id,
    depth: response.depth,
    createdAt: response.created_at,
  };
}
