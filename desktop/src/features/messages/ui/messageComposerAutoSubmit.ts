// Auto-submit a confirmed draft after the draft-persist lifecycle has restored
// it into the editor. Link-preview preparation is promoted by submit itself, so
// it must never hold this trigger in a polling loop.
export function scheduleSettleGatedAutoSubmit({
  submit,
  timers = {
    set: (fn: () => void, ms: number) => window.setTimeout(fn, ms),
    clear: (id: number) => window.clearTimeout(id),
  },
}: {
  submit: () => void;
  timers?: {
    set: (fn: () => void, ms: number) => number;
    clear: (id: number) => void;
  };
}): () => void {
  const timer = timers.set(submit, 0);
  return () => timers.clear(timer);
}
