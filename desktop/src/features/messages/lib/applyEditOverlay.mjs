/**
 * Pure helpers for applying an edit event's payload onto an original message
 * event when the renderer asks for the effective state.
 *
 * Lives in `.mjs` (not `.ts`) so the test runner (`node --test`, no TS loader)
 * can import the same source the production code uses. The TypeScript-facing
 * caller (`formatTimelineMessages.ts`) still gets typed access via local
 * type annotations at the callsite — these are pure data projections, no
 * runtime types to lose.
 */

/**
 * Merge the original event's tags with an edit's tags so that:
 *   - `imeta` tags come exclusively from the edit (full new attachment set);
 *   - all other tag kinds (`h`, `e`, `p` mentions, etc.) come exclusively
 *     from the original — the edit can't rewrite channel membership,
 *     thread refs, or mention targets.
 *
 * When `edit` is undefined, returns `originalTags` unchanged.
 */
export function applyEditTagOverlay(originalTags, editTags) {
  if (!editTags) return originalTags;
  const nonImetaOriginal = originalTags.filter((t) => t[0] !== "imeta");
  const imetaFromEdit = editTags.filter((t) => t[0] === "imeta");
  return [...nonImetaOriginal, ...imetaFromEdit];
}

/**
 * Apply both content and imeta-tag overlay. Body is taken from the edit
 * when present; tags are merged via `applyEditTagOverlay`.
 */
export function applyEditOverlay(originalEvent, edit) {
  if (!edit) {
    return { body: originalEvent.content, tags: originalEvent.tags };
  }
  return {
    body: edit.content,
    tags: applyEditTagOverlay(originalEvent.tags, edit.tags),
  };
}
