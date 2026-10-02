import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export async function getEventById(eventId: string): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("get_event", { eventId });
  return JSON.parse(eventJson) as RelayEvent;
}

export async function getEventsByIds(
  eventIds: string[],
): Promise<RelayEvent[]> {
  if (eventIds.length === 0) return [];
  return invokeTauri<RelayEvent[]>("get_events", { eventIds });
}
