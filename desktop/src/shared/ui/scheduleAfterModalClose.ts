import { MODAL_CLOSE_DURATION_MS } from "./modalMotion";

/**
 * Run `task` after a Radix dialog close animation. Opening a second dialog
 * in the same turn as unmounting the first leaves the new dialog painted
 * but inert (GitHub #6076).
 */
export function scheduleAfterModalClose(task: () => void): () => void {
  const timeoutId = window.setTimeout(task, MODAL_CLOSE_DURATION_MS);
  return () => {
    window.clearTimeout(timeoutId);
  };
}
