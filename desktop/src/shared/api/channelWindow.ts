import { invokeTauri } from "@/shared/api/tauri";
import type { ChannelPageCursor, RelayEvent } from "@/shared/api/types";

/** Fetch the flat Nostr event array for one server-assembled channel window. */
export async function getChannelWindowEvents(
  channelId: string,
  cursor: ChannelPageCursor | null = null,
  limitRows = 50,
): Promise<RelayEvent[]> {
  return invokeTauri<RelayEvent[]>("get_channel_window", {
    channelId,
    limitRows,
    cursor: cursor
      ? { created_at: cursor.createdAt, event_id: cursor.eventId }
      : null,
  });
}
