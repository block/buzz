import { invokeTauri } from "@/shared/api/tauri";

export async function editMessage(
  channelId: string,
  eventId: string,
  content: string,
  mediaTags?: string[][],
  emojiTags?: string[][],
  mentionPubkeys?: string[],
  suppressLinkPreviews?: boolean,
  mentionTags?: string[][],
): Promise<void> {
  await invokeTauri("edit_message", {
    input: {
      channelId,
      eventId,
      content,
      mediaTags: mediaTags ?? [],
      emojiTags: emojiTags ?? [],
      mentionPubkeys: mentionPubkeys ?? [],
      suppressLinkPreviews: suppressLinkPreviews ?? false,
      mentionTags: mentionTags ?? null,
    },
  });
}
