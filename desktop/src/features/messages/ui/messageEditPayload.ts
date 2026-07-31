/**
 * Builds the wire payload for a message edit submitted from `MessageComposer`.
 *
 * Split out of `MessageComposer.tsx` to keep that module under the desktop
 * file-size ceiling (see `desktop/scripts/check-file-sizes.mjs`). Pure —
 * everything the edit path derives before it touches composer state lives
 * here, so the ordering rules below are testable without a mounted editor.
 */
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { diffAddedMentionPubkeys } from "@/features/messages/lib/threading";

export type MessageEditPayload = {
  finalContent: string;
  outgoingTags: string[][];
  addedMentionPubkeys: string[];
};

export function buildMessageEditPayload(params: {
  trimmed: string;
  pendingImeta: ImetaMedia[];
  spoileredAttachmentUrls: Set<string>;
  customEmoji: ReadonlyArray<CustomEmoji>;
  previousBody: string;
  extractMentionPubkeys: (body: string) => string[];
  ownerPubkey: string;
}): MessageEditPayload {
  // Coerce `mediaTags ?? []` because edit semantics use `[]` as the explicit
  // "wipe all attachments" signal — the receiver overlay drops imeta when the
  // edit carries an empty (but defined) set.
  const { content: finalContent, mediaTags } = buildOutgoingMessage(
    params.trimmed,
    params.pendingImeta,
    params.spoileredAttachmentUrls,
  );

  // NIP-30: attach `["emoji", shortcode, url]` tags for custom emoji in the
  // edited body, exactly like the send path. Without this an edited message
  // ships with no emoji tags, so the receiver can't resolve a `:shortcode:`
  // and renders the literal text.
  const outgoingTags =
    mergeOutgoingTags(
      mediaTags,
      buildCustomEmojiTags(finalContent, params.customEmoji),
    ) ?? [];

  // Notify only mentions this edit *newly adds*: a typo-fix edit that leaves
  // the mention set unchanged emits no `p` tags and re-wakes nobody.
  const addedMentionPubkeys = diffAddedMentionPubkeys(
    params.extractMentionPubkeys(params.previousBody),
    params.extractMentionPubkeys(finalContent),
    params.ownerPubkey,
  );

  return { finalContent, outgoingTags, addedMentionPubkeys };
}
