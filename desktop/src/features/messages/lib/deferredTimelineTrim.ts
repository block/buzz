type TimerHost = {
  setTimeout: (
    handler: () => void,
    delay: number,
  ) => ReturnType<typeof setTimeout>;
  clearTimeout: (timer: ReturnType<typeof setTimeout>) => void;
};

/**
 * Defers leave-only cache trims by one task. React StrictMode immediately
 * re-runs an effect after its synthetic cleanup, so the matching setup can
 * cancel that trim while a real channel departure lets it run.
 */
export function createDeferredTimelineTrim(host: TimerHost = globalThis) {
  const pending = new Map<string, ReturnType<typeof setTimeout>>();

  return {
    cancel(channelId: string) {
      const timer = pending.get(channelId);
      if (timer !== undefined) {
        host.clearTimeout(timer);
        pending.delete(channelId);
      }
    },
    schedule(channelId: string, trim: () => void) {
      this.cancel(channelId);
      const timer = host.setTimeout(() => {
        pending.delete(channelId);
        trim();
      }, 0);
      pending.set(channelId, timer);
    },
  };
}

export const deferredTimelineTrim = createDeferredTimelineTrim();
