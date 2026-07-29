/**
 * Type declarations for the pure overlay helper in `applyEditTagOverlay.mjs`.
 * Runtime lives in `.mjs` so the (TS-loader-less) `node:test` runner can
 * import it directly; this file gives TypeScript callers a typed view.
 */

export type Tag = string[];

/**
 * Merge an event's tags with an edit's tags: imeta from the edit (full new
 * attachment set), NIP-30 emoji and NIP-CM notify tags from the edit when it
 * supplies any (otherwise the original's are preserved), all other tag kinds
 * from the original. Pass-through when `editTags` is `undefined`.
 */
export function applyEditTagOverlay(
  originalTags: Tag[],
  editTags: Tag[] | undefined,
): Tag[];
