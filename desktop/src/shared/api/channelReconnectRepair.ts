import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export type ChannelReconnectRepairRequest = {
  channelId: string;
  since: number;
  limit: number;
  until?: number;
  beforeId?: string;
};

/** Fetch one fixed-kind, channel-scoped reconnect repair page. */
export async function getChannelReconnectRepairEvents({
  channelId,
  since,
  limit,
  until,
  beforeId,
}: ChannelReconnectRepairRequest): Promise<RelayEvent[]> {
  return invokeTauri<RelayEvent[]>("get_channel_reconnect_repair", {
    channelId,
    since,
    limit,
    until: until ?? null,
    beforeId: beforeId ?? null,
  });
}
