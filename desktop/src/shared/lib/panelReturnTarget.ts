/**
 * One-shot "where did this panel transition come from?" breadcrumb.
 *
 * Panels that replace one another (rather than stacking) can capture an
 * explicit return target when they open, then consume it exactly once when
 * their back affordance fires. This avoids popping the wholesale app/browser
 * history stack, so an in-panel back press never leaves the current screen
 * unexpectedly.
 *
 * Pure store so the semantics are unit-testable; the React binding lives in
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
};

export function createPanelReturnTargetStore<T>(): PanelReturnTargetStore<T> {
  let target: T | null = null;

  return {
    capture(next) {
      target = next;
    },
    clear() {
      target = null;
    },
    consume() {
      const current = target;
      target = null;
      return current;
    },
    peek() {
      return target;
    },
  };
}
