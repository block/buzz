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
 *   - all other tag kinds (`h`, `e`, `p` mentions, etc.) come exclusively
 *     from the original — the edit can't rewrite channel membership,
 *     thread refs, or mention targets.
 *
 * When `editTags` is undefined, returns `originalTags` unchanged.
 */
export function applyEditTagOverlay(originalTags, editTags) {
  if (!editTags) return originalTags;
  const nonImetaOriginal = originalTags.filter((t) => t[0] !== "imeta");
  const imetaFromEdit = editTags.filter((t) => t[0] === "imeta");
  return [...nonImetaOriginal, ...imetaFromEdit];
}
