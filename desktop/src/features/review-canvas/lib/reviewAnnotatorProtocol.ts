import type { ReviewBlock } from "./reviewContract";

/** Static first-party asset served from the app's `public/` directory. */
export const ANNOTATOR_ASSET_PATH = "synaxis-review-annotator.js";

/** First message handed to the frame together with the capability port. */
export const ANNOTATOR_INIT_TYPE = "synaxis-review:init";

/** Messages the frame's annotator may send over the capability port. */
export type AnnotatorMessage =
  | { type: "ready"; blockIds: string[] }
  | { type: "select"; id: string };

/** Messages trusted chrome sends the annotator over the same port. */
export type ChromeMessage =
  | { type: "selection"; id: string | null }
  | { type: "focus"; id: string };

export function buildInitMessage(blocks: ReviewBlock[]) {
  return {
    type: ANNOTATOR_INIT_TYPE,
    blocks: blocks.map(({ id, title }) => ({ id, title })),
  };
}

/**
 * Validate one message from the frame. Everything the frame says is untrusted
 * input: only IDs the parent itself extracted and validated are honoured, and
 * the title/source reference always come from the parent's own block table.
 */
export function parseAnnotatorMessage(
  data: unknown,
  knownIds: ReadonlySet<string>,
): AnnotatorMessage | null {
  if (typeof data !== "object" || data === null) return null;
  const message = data as Record<string, unknown>;
  if (message.type === "select") {
    return typeof message.id === "string" && knownIds.has(message.id)
      ? { type: "select", id: message.id }
      : null;
  }
  if (message.type === "ready") {
    const ids = message.blockIds;
    return Array.isArray(ids) &&
      ids.length <= knownIds.size &&
      ids.every((id) => typeof id === "string" && knownIds.has(id))
      ? { type: "ready", blockIds: ids }
      : null;
  }
  return null;
}
