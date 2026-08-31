/**
 * Data-free native-work barrier shared across keyed React remounts. Only a
 * Promise<void> survives: no owner, relay, event, report, or query cache. This
 * deliberately must NOT reset on community switching (that would lose the
 * outstanding native work); decrypted state remains in the scoped component.
 */
export const hostNativeDrain = {
  pending: Promise.resolve() as Promise<void>,
  wait(): Promise<void> {
    return this.pending;
  },
  hold(drain: Promise<void>): void {
    this.pending = Promise.allSettled([this.pending, drain]).then(() => {});
  },
};
