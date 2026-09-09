import type { Channel } from "@/shared/api/types";
import type { PulseConversation } from "./unifiedFeed";

/** Order joined conversations by the latest relay metadata or loaded message. */
export function sortConversationsByRecency(
  channels: Channel[],
  conversations: PulseConversation[],
): Channel[] {
  const latest = new Map(
    channels.map((channel) => {
      const timestamp = Date.parse(channel.lastMessageAt ?? "");
      return [channel.id, Number.isFinite(timestamp) ? timestamp / 1000 : 0];
    }),
  );
  for (const conversation of conversations) {
    if (conversation.channel && Number.isFinite(conversation.latestAt)) {
      const id = conversation.channel.id;
      latest.set(id, Math.max(latest.get(id) ?? 0, conversation.latestAt));
    }
  }
  return channels
    .filter((channel) => channel.isMember && !channel.archivedAt)
    .sort(
      (a, b) =>
        (latest.get(b.id) ?? 0) - (latest.get(a.id) ?? 0) ||
        a.name.localeCompare(b.name) ||
        a.id.localeCompare(b.id),
    );
}
