import { getChatMetadata } from "@/shared/api/tauriChats";
import type { ChatMetadata } from "@/shared/api/types";

export type ChatOpenDestination =
  | { kind: "chat"; chatId: string }
  | { kind: "channel"; channelId: string };

type MetadataLookup = (channelId: string) => Promise<ChatMetadata | null>;

/**
 * Chat links carry the underlying private channel id. Builds that understand
 * chat metadata should open the Chats surface; otherwise the same id still
 * works as a private channel route.
 */
export async function resolveChatOpenDestination(
  channelId: string,
  lookup: MetadataLookup = getChatMetadata,
): Promise<ChatOpenDestination> {
  try {
    const metadata = await lookup(channelId);
    if (metadata) {
      return { kind: "chat", chatId: channelId };
    }
  } catch {
    // Missing access/metadata should degrade to the ordinary channel route,
    // which can show the existing private-channel access state.
  }
  return { kind: "channel", channelId };
}
