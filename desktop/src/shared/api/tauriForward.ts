import { invokeTauri, type RawSendChannelMessageResult } from "./tauri";

/**
 * Publish a kind-40009 message forward into `channelId`.
 *
 * `note` is the forwarder's optional note ("" when none). `forwardTags` is
 * the pre-composed `fwd`/`k`/`fwd-src`/`q`/`imeta` tag set from
 * `buildForwardTags` — the Rust side rejects any other tag family.
 * `mentionPubkeys` are the note's recipients; the Rust side turns them into
 * the `p` tags that drive notifications.
 */
export async function forwardMessage(
  channelId: string,
  note: string,
  forwardTags: string[][],
  mentionPubkeys?: string[],
): Promise<{ eventId: string; createdAt: number }> {
  const response = await invokeTauri<RawSendChannelMessageResult>(
    "forward_message",
    {
      channelId,
      note,
      forwardTags,
      mentionPubkeys: mentionPubkeys ?? null,
    },
  );

  return {
    eventId: response.event_id,
    createdAt: response.created_at,
  };
}
