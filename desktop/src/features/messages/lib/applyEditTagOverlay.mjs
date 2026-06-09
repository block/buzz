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
 *   - all other tag kinds (`h`, `e`, `p` mentions, etc.) come exclusively
 *     from the original — the edit can't rewrite channel membership,
 *     thread refs, or mention targets.
 *
 * When `editTags` is undefined, returns `originalTags` unchanged.
 */
export function applyEditTagOverlay(originalTags, editTags) {
  if (!editTags) return originalTags;
  const editEmoji = editTags.filter((t) => t[0] === "emoji");
  // imeta is always fully replaced by the edit. emoji is replaced only when
  // the edit actually supplies emoji tags; otherwise the original's are kept.
  const droppedFromOriginal =
    editEmoji.length > 0
      ? (t) => t[0] !== "imeta" && t[0] !== "emoji"
      : (t) => t[0] !== "imeta";
  const baseFromOriginal = originalTags.filter(droppedFromOriginal);
  const overlaidFromEdit = editTags.filter((t) => t[0] === "imeta");
  return [...baseFromOriginal, ...overlaidFromEdit, ...editEmoji];
}
