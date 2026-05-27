/**
 * Type declarations for the pure overlay helpers in `applyEditOverlay.mjs`.
 * The runtime lives in `.mjs` so the (TS-loader-less) test runner can import
 * it directly; this file gives TypeScript callers a typed view.
 */

export type Tag = string[];

export type EventLike = {
  content: string;
  tags: Tag[];
};

export type EditOverlayResult = {
  body: string;
  tags: Tag[];
};

/**
 * Merge an event's tags with an edit's tags: imeta from the edit (full new
 * attachment set), all other tag kinds from the original. Pass-through if
 * `editTags` is `undefined`.
 */
export function applyEditTagOverlay(
  originalTags: Tag[],
  editTags: Tag[] | undefined,
): Tag[];

/**
 * Apply both content and imeta-tag overlay. `body` from the edit when
 * present; tags merged via `applyEditTagOverlay`.
 */
export function applyEditOverlay(
  originalEvent: EventLike,
  edit: EventLike | undefined,
): EditOverlayResult;
