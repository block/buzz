/**
 * One-shot "where did this panel transition come from?" breadcrumb.
 *
 * Panels that replace one another (rather than stacking) can capture an
 * explicit return target when they open, then consume it exactly once when
 * their back affordance fires. This avoids popping the wholesale app/browser
 * history stack, so an in-panel back press never leaves the current screen
 * unexpectedly.
 *
 * The store is subscribable so UI can react to target presence (e.g. hide
 * the back arrow when there is nowhere to return to). Pure store so the
 * semantics are unit-testable; the React binding lives in
 * `@/shared/hooks/usePanelReturnTarget`.
 */
export type PanelReturnTargetStore<T> = {
  /** Record where the panel is coming from. `null` means "nowhere useful". */
  capture: (target: T | null) => void;
  /** Drop any recorded target without consuming it (e.g. on plain close). */
  clear: () => void;
  /** Take the recorded target, resetting the store — one back per capture. */
  consume: () => T | null;
  /** Read without consuming (for tests and conditional affordances). */
  peek: () => T | null;
  /**
   * Drop the target without notifying subscribers. Render-safe: the React
   * binding calls this while rendering on reset-key changes, where notifying
   * would schedule updates mid-render.
   */
  reset: () => void;
  /** Subscribe to target changes. Returns an unsubscribe function. */
  subscribe: (listener: () => void) => () => void;
};

export function createPanelReturnTargetStore<T>(): PanelReturnTargetStore<T> {
  let target: T | null = null;
  const listeners = new Set<() => void>();

  const notify = () => {
    for (const listener of [...listeners]) {
      listener();
    }
  };

  return {
    capture(next) {
      target = next;
      notify();
    },
    clear() {
      target = null;
      notify();
    },
    consume() {
      const current = target;
      target = null;
      notify();
      return current;
    },
    peek() {
      return target;
    },
    reset() {
      target = null;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}
