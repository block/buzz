/**
 * Pure helper for applying an edit event's imeta tags onto an original
 * message event. Used by both the renderer (formatTimelineMessages.ts)
 * and the post-edit cache update (useEditMessageMutation in hooks.ts) so
 * they stay in sync.
 *
 * Lives in `.mjs` (not `.ts`) so the test runner (`node --test`, no TS
 * loader) can import the same source the production code uses. The
 * TypeScript-facing callers get typed access via the sibling `.d.mts`.
 */

/**
 * Merge the original event's tags with an edit's tags so that:
 *   - `imeta` tags come exclusively from the edit (full new attachment set);
 *   - `p` tags from the edit join the original set because only newly added
 *     mentions notify. Reference-only `mention` tags, by contrast, are a full
 *     snapshot from the edited composer (marked by `buzz:mention-snapshot`)
 *     and therefore replace the original set; this preserves the edited body's
 *     stable recipient identities even before profiles load or after an alias
 *     changes;
 *   - `emoji` (NIP-30 custom-emoji) tags come from the edit *when the edit
 *     supplies any* — the edited body may add or remove custom emoji, so a
 *     supplied set rebuilds the shortcode→url map. But when the edit supplies
 *     NO emoji tags, the original's emoji tags are PRESERVED. A tag-less edit
 *     can come from an older build (before edits carried emoji tags) or another
 *     client that doesn't know this path; dropping the original's emoji tags
 *     there would strip the only shortcode→url mapping and re-break a
 *     `:shortcode:` that the original rendered fine. Preserving on empty is
 *     strictly safe: an orphaned emoji tag whose shortcode is no longer in the
 *     body resolves nothing, so it can't cause a stale render.
 *   - all other tag kinds (`h`, `e`, etc.) come exclusively from the original
 *     so the edit can't rewrite channel membership or thread references.
 *
 * When `editTags` is undefined, returns `originalTags` unchanged.
 */
export function applyEditTagOverlay(originalTags, editTags) {
  if (!editTags) return originalTags;
  const editEmoji = editTags.filter((t) => t[0] === "emoji");
  const hasMentionSnapshot = editTags.some(
    (t) => t[0] === "buzz:mention-snapshot",
  );
  const editMentions = editTags.filter((t) => t[0] === "mention");
  // imeta is always fully replaced by the edit. emoji is replaced only when
  // the edit actually supplies emoji tags; otherwise the original's are kept.
  // An edit carrying the private snapshot marker is authoritative, including
  // an empty mention set. Legacy edits without the marker preserve original
  // references so older clients remain compatible.
  const droppedFromOriginal = (tag) => {
    if (tag[0] === "imeta") return false;
    if (editEmoji.length > 0 && tag[0] === "emoji") return false;
    if (hasMentionSnapshot && tag[0] === "mention") return false;
    return true;
  };
  const baseFromOriginal = originalTags.filter(droppedFromOriginal);
  const overlaidFromEdit = editTags.filter(
    (t) => t[0] === "imeta" || t[0] === "p" || t[0] === "buzz:mention-snapshot",
  );
  return [
    ...baseFromOriginal,
    ...overlaidFromEdit,
    ...editEmoji,
    ...editMentions,
  ];
}
