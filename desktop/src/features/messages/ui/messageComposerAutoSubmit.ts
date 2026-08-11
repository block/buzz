// Auto-submit scheduler for a confirmed draft that arrived via ?autoSend. A
// draft containing a supported link is normally still settling (350 ms
// debounce + metadata/upload) at mount, so a submit fired immediately bails on
// the pending-snapshot guard. A one-shot `setTimeout(0)` would consume the
// trigger and silently drop the draft; instead poll until settling finishes
// (bounded by the preview hook's own anti-trap cap) then submit exactly once.
// The `didSubmit` guard prevents a double fire, and the initial defer lets the
// draft-persist lifecycle effect load the draft into the editor first.
//
// Extracted from MessageComposer as a pure, timer-injectable helper so the
// retry/one-shot contract is unit-testable without mounting the composer.
export function scheduleSettleGatedAutoSubmit({
  isPending,
  submit,
  retryDelayMs = 50,
  timers = {
    set: (fn: () => void, ms: number) => window.setTimeout(fn, ms),
    clear: (id: number) => window.clearTimeout(id),
  },
}: {
  isPending: () => boolean;
  submit: () => void;
  retryDelayMs?: number;
  timers?: {
    set: (fn: () => void, ms: number) => number;
    clear: (id: number) => void;
  };
}): () => void {
  let didSubmit = false;
  let retryTimer = 0;
  const attempt = () => {
    if (didSubmit) return;
    if (isPending()) {
      retryTimer = timers.set(attempt, retryDelayMs);
      return;
    }
    didSubmit = true;
    submit();
  };
  const initialTimer = timers.set(attempt, 0);
  return () => {
    timers.clear(initialTimer);
    timers.clear(retryTimer);
  };
}
