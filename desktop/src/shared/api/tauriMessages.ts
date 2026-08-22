import { invokeTauri } from "@/shared/api/tauri";
import type { RawSendChannelMessageResult } from "@/shared/api/tauriMessageTypes";
import type { SendChannelMessageResult } from "@/shared/api/types";

export async function sendChannelMessage(
  channelId: string,
  content: string,
  parentEventId?: string | null,
  mediaTags?: string[][],
  mentionPubkeys?: string[],
  kind?: number,
  emojiTags?: string[][],
  mentionTags?: string[][],
  linkPreviewTags?: string[][],
  sentFromThreadTag?: string[],
  expectedRelayUrl?: string,
  expectedSignerPubkey?: string,
  /**
   * `["e", "<id>", "", "supersedes"]` tags — one per attachment the user
   * opted into linking as a new version of an earlier upload. Rides its own
   * Tauri arg (not `mediaTags`) because the Rust `imeta_tags` validator
   * rejects any non-`imeta`-prefixed tag.
   *
   * Positioned after upstream's params, not before: 0.5.18 claimed slots 11
   * and 12 for the tenant-scope guard, and upstream call sites pass those
   * positionally.
   */
  supersedesTags?: string[][],
  /**
   * `"channel"` or `"here"` — the `@channel` / `@here` marker, as its own arg
   * for the same reason `supersedesTags` is: every tag group the Rust side
   * accepts is validated against its own prefix, so there is no general
   * passthrough to ride.
   */
  mentionScope?: string | null,
): Promise<SendChannelMessageResult> {
  const response = await invokeTauri<RawSendChannelMessageResult>(
    "send_channel_message",
    {
      channelId,
      content,
      parentEventId,
      mediaTags: mediaTags ?? null,
      emojiTags: emojiTags ?? null,
      mentionTags: mentionTags ?? null,
      linkPreviewTags,
      sentFromThreadTag: sentFromThreadTag ?? null,
      supersedesTags: supersedesTags ?? null,
      mentionPubkeys: mentionPubkeys ?? null,
      kind: kind ?? null,
      mentionScope: mentionScope ?? null,
      // Tenant scope captured by the caller before its first await; the
      // backend fails closed when the active community no longer matches.
      expectedRelayUrl: expectedRelayUrl ?? null,
      // Signer identity captured with the relay scope; the backend fails
      // closed when the active identity no longer matches, so a community
      // switch cannot re-sign the captured tenant's content as the new one.
      expectedSignerPubkey: expectedSignerPubkey ?? null,
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
