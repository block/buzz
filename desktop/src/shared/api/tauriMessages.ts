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
   * NIP-AD: pubkeys of the agents this message is addressed to. A non-empty
   * list makes it a marked request awaiting a disposition, and each pubkey
   * becomes an `["agent", <pubkey>]` tag that a disposition must be signed
   * by to resolve this request. Deliberately not derived from the `p`
   * mention set — a mentioned human must not be able to close an agent's
   * obligation. See docs/nips/NIP-AD.md.
   */
  requestAgentPubkeys?: string[],
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
      mentionPubkeys: mentionPubkeys ?? null,
      kind: kind ?? null,
      // Tenant scope captured by the caller before its first await; the
      // backend fails closed when the active community no longer matches.
      expectedRelayUrl: expectedRelayUrl ?? null,
      // Signer identity captured with the relay scope; the backend fails
      // closed when the active identity no longer matches, so a community
      // switch cannot re-sign the captured tenant's content as the new one.
      expectedSignerPubkey: expectedSignerPubkey ?? null,
      requestAgentPubkeys: requestAgentPubkeys ?? null,
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
