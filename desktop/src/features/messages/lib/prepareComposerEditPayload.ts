import { buildNip27WireBody } from "@/features/messages/lib/collectMentionPubkeys";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { diffAddedMentionPubkeys } from "@/features/messages/lib/threading";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/**
 * Build the wire body, imeta/emoji tags, and newly-added mention pubkeys for
 * a composer edit save. Composer text stays as `@name`; the wire body is
 * rewritten to NIP-27 `nostr:npub1…` URIs.
 */
export function prepareComposerEditPayload(input: {
  trimmed: string;
  previousBody: string;
  pendingImeta: ImetaMedia[];
  spoileredAttachmentUrls: ReadonlySet<string>;
  customEmoji: CustomEmoji[];
  ownerPubkey: string;
  extractMentionPubkeys: (text: string) => string[];
  getMentionDisplayName: (pubkey: string) => string | null;
}): {
  finalContent: string;
  outgoingTags: string[][];
  addedMentionPubkeys: string[];
} {
  const wireBody = buildNip27WireBody(
    input.trimmed,
    input.extractMentionPubkeys(input.trimmed),
    input.getMentionDisplayName,
  );
  const { content: finalContent, mediaTags } = buildOutgoingMessage(
    wireBody,
    input.pendingImeta,
    input.spoileredAttachmentUrls,
  );
  // `?? []` preserves edit semantics: a defined-but-empty media set means
  // "wipe attachments".
  const outgoingTags =
    mergeOutgoingTags(
      mediaTags,
      buildCustomEmojiTags(finalContent, input.customEmoji),
    ) ?? [];
  const addedMentionPubkeys = diffAddedMentionPubkeys(
    input.extractMentionPubkeys(input.previousBody),
    input.extractMentionPubkeys(input.trimmed),
    input.ownerPubkey,
  );
  return { finalContent, outgoingTags, addedMentionPubkeys };
}
