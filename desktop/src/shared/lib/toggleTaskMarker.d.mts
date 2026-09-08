/**
 * Type declarations for the pure task-marker helpers in
 * `toggleTaskMarker.mjs`. Runtime lives in `.mjs` so the `node:test` runner
 * can import it directly; this file gives TypeScript callers a typed view.
 */

/**
 * Flip the Nth GFM task-list marker in `source`. Returns `null` when the
 * ordinal resolves to nothing, meaning the source drifted from what was
 * rendered — callers must not write a guess in that case.
 */
export function toggleTaskMarker(
  source: string,
  taskIndex: number,
  checked: boolean,
): string | null;

/** Number of task markers the renderer will turn into checkboxes. */
export function countTaskMarkers(source: string): number;
