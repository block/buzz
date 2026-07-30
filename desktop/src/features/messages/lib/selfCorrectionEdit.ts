/**
 * Resolves an IRC-style `s/old/new/` self-correction against a channel's
 * timeline into a ready-to-publish edit.
 *
 * This is the bridge between the pure command grammar in `selfCorrection.ts`
 * and the existing kind-40003 edit path: it finds the author's most recent
 * editable message, applies the substitution to its human-visible body, and
 * rebuilds the outgoing edit — preserving imeta attachments and NIP-30 custom
 * emoji exactly like the manual edit-save path does. Kept UI-free so it can be
 * unit-tested; the composing hook only wires state and publishes the result.
 *
 * Mentions: a typo-fix correction intentionally emits no new mention `p` tags
 * (the corrected body keeps the original mention tokens, so nobody is re-woken
 * and existing mentions still render). A substitution that introduces a
 * brand-new `@mention` will therefore not notify that user — an accepted v1
 * limitation, since the shortcut targets quick self-corrections.
 */

import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import type { TimelineMessage } from "@/features/messages/types";
import {
  buildOutgoingMessage,
  findSpoileredImetaMediaUrls,
  imetaMediaFromTags,
  mergeOutgoingTags,
  restoreImetaMediaDisplayLabels,
  stripImetaMediaLines,
} from "@/features/messages/lib/imetaMediaMarkdown";
import {
  applySelfCorrection,
  parseSelfCorrection,
} from "@/features/messages/lib/selfCorrection";

/** A published edit derived from a self-correction command. */
export type SelfCorrectionEdit = {
  eventId: string;
  content: string;
  tags: string[][];
};

/**
 * The author's most recent message that a self-correction may target, mirroring
 * the eligibility rules of ArrowUp-to-edit: own, non-system, not still pending.
 */
export function findLastOwnCorrectable(
  candidates: readonly TimelineMessage[],
  currentPubkey: string,
): TimelineMessage | null {
  let best: TimelineMessage | null = null;
  for (const message of candidates) {
    if (
      message.kind === KIND_SYSTEM_MESSAGE ||
      message.pubkey !== currentPubkey ||
      message.pending
    ) {
      continue;
    }
    if (!best || message.createdAt >= best.createdAt) {
      best = message;
    }
  }
  return best;
}

/**
 * Resolve `text` as a self-correction against `candidates`. Returns the edit to
 * publish, or `null` when `text` is not a command, there is no editable target,
 * or the pattern does not occur in the target (all of which mean "send `text`
 * literally instead").
 */
export function resolveSelfCorrection(
  text: string,
  candidates: readonly TimelineMessage[],
  currentPubkey: string,
  customEmoji: ReadonlyArray<CustomEmoji>,
): SelfCorrectionEdit | null {
  const command = parseSelfCorrection(text.trim());
  if (!command) return null;

  const target = findLastOwnCorrectable(candidates, currentPubkey);
  if (!target) return null;

  // Rebuild the human-editable body (imeta markdown lines stripped) so the
  // substitution runs over what the author actually sees, then reassemble the
  // outgoing edit the same way the manual edit-save path does.
  const editableImeta = restoreImetaMediaDisplayLabels(
    target.body,
    imetaMediaFromTags(target.tags ?? []),
  );
  const editableBody = stripImetaMediaLines(target.body, editableImeta);
  const correctedBody = applySelfCorrection(editableBody, command);
  if (correctedBody === null || correctedBody === editableBody) {
    return null;
  }

  const { content, mediaTags } = buildOutgoingMessage(
    correctedBody,
    editableImeta,
    findSpoileredImetaMediaUrls(target.body, editableImeta),
  );
  const tags =
    mergeOutgoingTags(mediaTags, buildCustomEmojiTags(content, customEmoji)) ??
    [];

  return { eventId: target.id, content, tags };
}
