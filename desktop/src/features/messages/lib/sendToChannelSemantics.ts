import type { TimelineMessage } from "@/features/messages/types";
import { orderMentionPubkeysByText } from "@/features/messages/lib/orderMentionPubkeys";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";

const PUBKEY_PATTERN = /^[0-9a-f]{64}$/;
const SHAREABLE_TAG_KINDS = new Set([
  "emoji",
  "imeta",
  "link-preview",
  "mention",
]);

export type SendToChannelSemantics = {
  mentionPubkeys: string[];
  semanticTags: string[][];
};

/**
 * Preserve the source message metadata that gives its body meaning without
 * copying structural channel/thread tags or the source event's self `p` tag.
 */
export function getSendToChannelSemantics(
  message: TimelineMessage,
  profiles?: UserProfileLookup,
): SendToChannelSemantics {
  const sourceAuthors = new Set(
    [message.pubkey, message.signerPubkey]
      .filter((pubkey): pubkey is string => Boolean(pubkey))
      .map(normalizePubkey),
  );
  const seenMentions = new Set<string>();
  const mentionPubkeys: string[] = [];
  const effectiveMentionPubkeys = message.edited
    ? new Set(
        (message.tags ?? []).some((tag) => tag[0] === "buzz:mention-snapshot")
          ? (message.tags ?? [])
              .filter((tag) => tag[0] === "mention")
              .map((tag) => normalizePubkey(tag[1] ?? ""))
              .filter((pubkey) => PUBKEY_PATTERN.test(pubkey))
          : orderMentionPubkeysByText(
              message.body,
              resolveMentionProps(message.tags, profiles).mentionPubkeysByName,
              () => true,
            ),
      )
    : null;
  const semanticTags: string[][] = [];
  const hasPreviewSuppression = message.tags?.some(
    (tag) => tag.length === 2 && tag[0] === "link-preview" && tag[1] === "none",
  );

  for (const tag of message.tags ?? []) {
    if (tag[0] === "p") {
      const pubkey = normalizePubkey(tag[1] ?? "");
      if (
        PUBKEY_PATTERN.test(pubkey) &&
        !sourceAuthors.has(pubkey) &&
        (effectiveMentionPubkeys === null ||
          effectiveMentionPubkeys.has(pubkey)) &&
        !seenMentions.has(pubkey)
      ) {
        seenMentions.add(pubkey);
        mentionPubkeys.push(pubkey);
      }
      continue;
    }

    if (SHAREABLE_TAG_KINDS.has(tag[0] ?? "")) {
      if (
        tag[0] === "link-preview" &&
        hasPreviewSuppression &&
        !(tag.length === 2 && tag[1] === "none")
      ) {
        continue;
      }
      semanticTags.push([...tag]);
    }
  }

  return { mentionPubkeys, semanticTags };
}
